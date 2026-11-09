use labelize_printer_core::AppPaths;
use labelize_printer_updater::{
    self as updater,
    handoff::{self, Ticket},
    InstallSource, Offer, Target, Verified,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub enum UpdateEvent {
    Detected(InstallSource, Result<Target, String>, bool),
    Checked(Result<Option<Offer>, String>, bool),
    Progress(u64, u64),
    Downloaded(Result<Verified, String>),
    Prepared(Result<Ticket, String>),
    Recovery(Result<String, String>),
    Pending(String),
}
#[derive(PartialEq)]
pub enum Phase {
    Idle,
    Checking,
    Downloading,
    Preparing,
}
pub struct Updates {
    pub paths: AppPaths,
    pub events: async_channel::Sender<UpdateEvent>,
    pub source: InstallSource,
    pub target: Option<Target>,
    pub manual_install: bool,
    pub phase: Phase,
    pub offer: Option<Offer>,
    pub verified: Option<Verified>,
    pub progress: (u64, u64),
    pub error: Option<String>,
    pub recovery: Option<String>,
    pub message: Option<String>,
    pub checked: bool,
    pub cancel: Arc<AtomicBool>,
    pub ticket: Option<Ticket>,
}
impl Updates {
    pub fn new(paths: AppPaths) -> (Self, async_channel::Receiver<UpdateEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let send = tx.clone();
        let root = paths.clone();
        std::thread::spawn(move || {
            if let Err(error) = updater::cleanup_cache(&root) {
                log::warn!("Update cache: {error:#}");
            }
            if labelize_printer_core::maintenance::pending(&root).exists() {
                match handoff::recover(&root, false) {
                    Ok(message) => {
                        let _ = send.send_blocking(UpdateEvent::Recovery(Ok(message)));
                    }
                    Err(error) => {
                        let _ = send.send_blocking(UpdateEvent::Pending(format!("{error:#}")));
                    }
                }
            }
            let _ = send.send_blocking(UpdateEvent::Detected(
                updater::platform::source(),
                updater::platform::target().map_err(|e| format!("{e:#}")),
                !updater::platform::deb_handler(),
            ));
        });
        (
            Self {
                paths,
                events: tx,
                source: InstallSource::Unknown,
                target: None,
                manual_install: false,
                phase: Phase::Idle,
                offer: None,
                verified: None,
                progress: (0, 0),
                error: None,
                recovery: None,
                message: None,
                checked: false,
                cancel: Arc::new(AtomicBool::new(false)),
                ticket: None,
            },
            rx,
        )
    }
    pub fn check(&mut self, manual: bool) {
        if self.phase != Phase::Idle {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        self.phase = Phase::Checking;
        self.error = None;
        let tx = self.events.clone();
        std::thread::spawn(move || {
            let result =
                updater::check(env!("CARGO_PKG_VERSION"), target).map_err(|e| format!("{e:#}"));
            let _ = tx.send_blocking(UpdateEvent::Checked(result, manual));
        });
    }
    pub fn download(&mut self) {
        if self.phase != Phase::Idle || self.source != InstallSource::Package {
            return;
        }
        let Some(offer) = self.offer.clone() else {
            return;
        };
        self.phase = Phase::Downloading;
        self.error = None;
        self.verified = None;
        self.progress = (0, offer.installer.size);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let tx = self.events.clone();
        let paths = self.paths.clone();
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now();
            let result = updater::download(&paths, offer, &cancel, |n, total| {
                if last.elapsed().as_millis() >= 100 || n == total {
                    let _ = tx.send_blocking(UpdateEvent::Progress(n, total));
                    last = std::time::Instant::now();
                }
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send_blocking(UpdateEvent::Downloaded(result));
        });
    }
    pub fn recover(&mut self, acknowledge: bool) {
        if self.phase != Phase::Idle {
            return;
        }
        self.phase = Phase::Preparing;
        let paths = self.paths.clone();
        let tx = self.events.clone();
        std::thread::spawn(move || {
            let result = handoff::recover(&paths, acknowledge).map_err(|e| format!("{e:#}"));
            let _ = tx.send_blocking(UpdateEvent::Recovery(result));
        });
    }
}
impl Drop for Updates {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
