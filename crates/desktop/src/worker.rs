use anyhow::Context;
use labelize_printer_core::{ipc, platform, AppPaths, Request, Settings, Snapshot};
use std::{sync::mpsc, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Refresh,
    Save,
    Install,
    Test,
    Retry,
    Remove,
}

pub enum Work {
    Refresh,
    Save(Settings),
    Install(Settings),
    Test(Option<Settings>),
    Retry(String),
    Remove,
}

impl Work {
    pub fn activity(&self) -> Activity {
        match self {
            Self::Refresh => Activity::Refresh,
            Self::Save(_) => Activity::Save,
            Self::Install(_) => Activity::Install,
            Self::Test(_) => Activity::Test,
            Self::Retry(_) => Activity::Retry,
            Self::Remove => Activity::Remove,
        }
    }

    fn updates_settings(&self) -> bool {
        matches!(
            self,
            Self::Save(_) | Self::Install(_) | Self::Test(Some(_)) | Self::Remove
        )
    }
}

type Status = Result<Snapshot, String>;

pub enum Event {
    Status(Status),
    Completed {
        activity: Activity,
        updates_settings: bool,
        result: Status,
    },
}

pub fn spawn(paths: AppPaths) -> (mpsc::Sender<Work>, async_channel::Receiver<Event>) {
    let (work, commands) = mpsc::channel::<Work>();
    let (events, updates) = async_channel::unbounded();
    std::thread::spawn(move || {
        let initial = platform::ensure_daemon(&paths)
            .and_then(|()| status(&paths))
            .map_err(|error| format!("{error:#}"));
        if events.send_blocking(Event::Status(initial)).is_err() {
            return;
        }
        // Poll and perform actions on the same thread so a pre-action snapshot
        // cannot arrive after a completed save or release its busy state.
        while let Some(event) = match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(work) => Some(Event::Completed {
                activity: work.activity(),
                updates_settings: work.updates_settings(),
                result: execute(&paths, work).map_err(|error| format!("{error:#}")),
            }),
            Err(mpsc::RecvTimeoutError::Timeout) => Some(Event::Status(
                status(&paths).map_err(|error| format!("{error:#}")),
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
        } {
            if events.send_blocking(event).is_err() {
                break;
            }
        }
    });
    (work, updates)
}

fn status(paths: &AppPaths) -> anyhow::Result<Snapshot> {
    ipc::request(paths, Request::Status)?.decode()
}

fn save(paths: &AppPaths, settings: Settings) -> anyhow::Result<()> {
    ipc::request(paths, Request::SaveSettings { settings })?.decode::<Settings>()?;
    Ok(())
}

fn execute(paths: &AppPaths, work: Work) -> anyhow::Result<Snapshot> {
    match work {
        Work::Refresh => {
            platform::ensure_daemon(paths)?;
            return ipc::request(paths, Request::RefreshPlatform)?.decode();
        }
        Work::Save(settings) => save(paths, settings)?,
        Work::Install(settings) => {
            save(paths, settings)?;
            return ipc::request(paths, Request::Install)
                .and_then(|response| response.decode())
                .context("输出设置已保存，打印机配置失败");
        }
        Work::Test(settings) => {
            if let Some(settings) = settings {
                save(paths, settings)?;
            }
            ipc::request(paths, Request::TestPrint)?.decode::<serde_json::Value>()?;
        }
        Work::Retry(id) => {
            ipc::request(paths, Request::Retry { id })?.decode::<serde_json::Value>()?;
        }
        Work::Remove => return ipc::request(paths, Request::Uninstall)?.decode(),
    }
    status(paths)
}
