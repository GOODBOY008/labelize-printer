//! One cancellable renderer at a time, newest revision per document.
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
};

/// A callback is valid only for the current request, even if a cancelled preview
/// is restarted at the same document revision after switching tabs.
#[derive(Default)]
pub struct PreviewState {
    serial: u64,
    current: Option<(u64, u64)>,
}
impl PreviewState {
    pub fn begin(&mut self, revision: u64) -> u64 {
        self.serial += 1;
        self.current = Some((self.serial, revision));
        self.serial
    }
    pub fn cancel(&mut self) {
        self.current = None;
    }
    pub fn is_running(&self) -> bool {
        self.current.is_some()
    }
    pub fn finish(&mut self, request: u64, revision: u64) -> bool {
        if self.current == Some((request, revision)) {
            self.current = None;
            true
        } else {
            false
        }
    }
}

type Render<T> = Box<dyn FnOnce(Arc<AtomicBool>) -> Result<T, String> + Send>;
struct Request<T> {
    id: String,
    run: Render<T>,
    result: async_channel::Sender<Result<T, String>>,
}
struct State<T> {
    pending: VecDeque<Request<T>>,
    active: Option<String>,
    running: Option<(String, Arc<AtomicBool>)>,
    stopped: bool,
}
struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
}
pub struct PreviewQueue<T: Send + 'static> {
    shared: Arc<Shared<T>>,
}
impl<T: Send + 'static> Default for PreviewQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Send + 'static> PreviewQueue<T> {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: VecDeque::<Request<T>>::new(),
                active: None,
                running: None,
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        thread::spawn(move || loop {
            let (request, cancelled) = {
                let mut state = worker.state.lock().unwrap();
                while !state.stopped && state.pending.is_empty() {
                    state = worker.wake.wait(state).unwrap();
                }
                if state.stopped {
                    break;
                }
                let index = state
                    .pending
                    .iter()
                    .position(|r| Some(&r.id) == state.active.as_ref())
                    .unwrap_or(0);
                let request = state.pending.remove(index).unwrap();
                let cancelled = Arc::new(AtomicBool::new(false));
                state.running = Some((request.id.clone(), cancelled.clone()));
                (request, cancelled)
            };
            let result = (request.run)(cancelled.clone());
            let result = if cancelled.load(Ordering::Acquire) {
                Err("Preview cancelled".into())
            } else {
                result
            };
            let _ = request.result.send_blocking(result);
            worker.state.lock().unwrap().running = None;
        });
        Self { shared }
    }
    pub fn activate(&self, id: Option<String>) {
        let mut state = self.shared.state.lock().unwrap();
        state.active = id;
    }
    pub fn cancel(&self, id: &str) {
        let mut state = self.shared.state.lock().unwrap();
        state.pending.retain(|r| r.id != id);
        if let Some((running, cancel)) = &state.running {
            if running == id {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    pub fn submit(
        &self,
        id: String,
        run: impl FnOnce(Arc<AtomicBool>) -> Result<T, String> + Send + 'static,
    ) -> async_channel::Receiver<Result<T, String>> {
        let (tx, rx) = async_channel::bounded(1);
        let mut state = self.shared.state.lock().unwrap();
        state.pending.retain(|r| r.id != id);
        if let Some((running, cancel)) = &state.running {
            if running == &id || state.active.as_ref() == Some(&id) {
                cancel.store(true, Ordering::Release);
            }
        }
        state.pending.push_back(Request {
            id,
            run: Box::new(run),
            result: tx,
        });
        self.shared.wake.notify_one();
        rx
    }
}
impl<T: Send + 'static> Drop for PreviewQueue<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.stopped = true;
        state.pending.clear();
        if let Some((_, cancel)) = &state.running {
            cancel.store(true, Ordering::Release);
        }
        self.shared.wake.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn stale_callbacks_cannot_clear_a_restarted_preview_at_the_same_revision() {
        let mut state = PreviewState::default();
        let old = state.begin(7);
        state.cancel();
        let current = state.begin(7);
        assert!(!state.finish(old, 7));
        assert!(state.is_running());
        assert!(!state.finish(current, 8));
        assert!(state.finish(current, 7));
        assert!(!state.is_running());
    }
    #[test]
    fn supersedes_running_revision_and_serializes_children() {
        let queue = PreviewQueue::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let old = queue.submit("a".into(), move |cancel| {
            ready_tx.send(()).unwrap();
            while !cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(1)
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let new = queue.submit("a".into(), |_| Ok(2));
        assert!(old.recv_blocking().unwrap().is_err());
        assert_eq!(new.recv_blocking().unwrap().unwrap(), 2);
    }
    #[test]
    fn active_document_precedes_queued_background_and_closed_is_cancelled() {
        let queue = PreviewQueue::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let first = queue.submit("first".into(), move |_| {
            tx.send(()).unwrap();
            wait.recv().unwrap();
            Ok(0)
        });
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (order_tx, order_rx) = std::sync::mpsc::channel();
        let tx = order_tx.clone();
        let background = queue.submit("background".into(), move |_| {
            tx.send(2).unwrap();
            Ok(2)
        });
        let closed = queue.submit("closed".into(), |_| panic!("Closed preview must not run"));
        queue.cancel("closed");
        queue.activate(Some("active".into()));
        let active = queue.submit("active".into(), move |_| {
            order_tx.send(1).unwrap();
            Ok(1)
        });
        release.send(()).unwrap();
        let _ = first.recv_blocking();
        assert!(closed.recv_blocking().is_err());
        assert_eq!(active.recv_blocking().unwrap().unwrap(), 1);
        assert_eq!(background.recv_blocking().unwrap().unwrap(), 2);
        assert_eq!(order_rx.recv().unwrap(), 1);
        assert_eq!(order_rx.recv().unwrap(), 2);
    }
}
