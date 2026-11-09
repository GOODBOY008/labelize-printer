//! The helper is copied outside the installation tree and uses no GPUI code.
use crate::{platform, validate_verified, InstallSource, Verified};
use anyhow::{bail, ensure, Context, Result};
use fs2::FileExt;
use labelize_printer_core::{
    ipc, maintenance,
    paths::{atomic_write, private_dir},
    AppPaths, Request,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub source: InstallSource,
    pub version: String,
    pub installer: Option<Verified>,
    pub manual: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub data_dir: PathBuf,
    pub parent: u32,
    pub app: PathBuf,
    pub plan: Plan,
    pub phase: String,
    pub error: Option<String>,
}
impl Ticket {
    pub fn dir(&self) -> PathBuf {
        self.data_dir.join("updates").join(&self.id)
    }
    fn save(&self, paths: &AppPaths) -> Result<()> {
        atomic_write(
            &maintenance::pending(paths),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}
fn lock(paths: &AppPaths) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.root.join("update.lock"))?;
    file.try_lock_exclusive()
        .context("An update installer is still open; finish or close it before continuing")?;
    Ok(file)
}
pub fn pending(paths: &AppPaths) -> Result<Option<Ticket>> {
    match fs::read(maintenance::pending(paths)) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).context(
            "The update handoff record is damaged; keep it for diagnosis",
        )?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn prepare(paths: &AppPaths, plan: Plan) -> Result<Ticket> {
    maintenance::require_ready(paths)?;
    ensure!(
        plan.source != InstallSource::Unknown,
        "Choose the installation source first"
    );
    ensure!(
        semver::Version::parse(&plan.version)? > semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Update must be newer than this app"
    );
    if plan.source == InstallSource::Package {
        validate_verified(plan.installer.as_ref().context("No verified installer")?)?;
    }
    let app = std::env::current_exe()?;
    let ticket = Ticket {
        id: uuid::Uuid::new_v4().to_string(),
        data_dir: paths.root.clone(),
        parent: std::process::id(),
        app,
        plan,
        phase: "preparing".into(),
        error: None,
    };
    let directory = ticket.dir();
    private_dir(&directory)?;
    let name = if cfg!(windows) {
        "labelize-printer-updater.exe"
    } else {
        "labelize-printer-updater"
    };
    let helper = directory.join(name);
    fs::copy(
        labelize_printer_core::platform::sibling_binary("labelize-printer-updater")?,
        &helper,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
    }
    let request = directory.join("request.json");
    atomic_write(&request, &serde_json::to_vec(&ticket)?)?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("handoff.log"))?;
    let mut command = Command::new(helper);
    platform::hide(&mut command);
    let mut child = command
        .arg("--handoff")
        .arg(request)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if directory.join("ready").is_file() {
            return Ok(ticket);
        }
        if let Some(status) = child.try_wait()? {
            let message = fs::read_to_string(directory.join("error"))
                .unwrap_or_else(|_| format!("Update helper exited: {status}"));
            bail!("{message}");
        }
        if Instant::now() > until {
            atomic_write(&directory.join("cancel"), b"cancel")?;
            bail!("Update preparation timed out; the helper will restore printing");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn authorize(ticket: &Ticket) -> Result<()> {
    atomic_write(&ticket.dir().join("go"), b"install")
}
pub fn cancel(ticket: &Ticket) {
    let _ = atomic_write(&ticket.dir().join("cancel"), b"cancel");
}
fn wait_until(mut predicate: impl FnMut() -> bool, timeout: Duration, message: &str) -> Result<()> {
    let until = Instant::now() + timeout;
    while !predicate() {
        ensure!(Instant::now() < until, "{message}");
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
pub fn run(mut ticket: Ticket) -> Result<()> {
    uuid::Uuid::parse_str(&ticket.id)?;
    ensure!(ticket.app.is_absolute(), "Invalid app path");
    let paths = AppPaths::new(ticket.data_dir.clone())?;
    let _lock = lock(&paths)?;
    maintenance::require_ready(&paths)?;
    let mut commit_requested = false;
    let prepared = (|| -> Result<()> {
        if let Some(installer) = &ticket.plan.installer {
            validate_verified(installer)?;
        }
        ipc::request(
            &paths,
            Request::PrepareUpdate {
                token: ticket.id.clone(),
            },
        )?
        .decode::<serde_json::Value>()?;
        ticket.save(&paths)?;
        atomic_write(&ticket.dir().join("ready"), b"ready")?;
        wait_until(
            || {
                ticket.dir().join("go").exists()
                    || ticket.dir().join("cancel").exists()
                    || !platform::alive(ticket.parent)
            },
            Duration::from_secs(60),
            "Desktop did not authorize installation",
        )?;
        ensure!(
            ticket.dir().join("go").exists() && !ticket.dir().join("cancel").exists(),
            "Update cancelled before installation"
        );
        wait_until(
            || !platform::alive(ticket.parent),
            Duration::from_secs(30),
            "The desktop is still running; installation was not started",
        )?;
        commit_requested = true;
        ipc::request(
            &paths,
            Request::CommitUpdate {
                token: ticket.id.clone(),
            },
        )?
        .decode::<serde_json::Value>()?;
        Ok(())
    })();
    if let Err(error) = prepared {
        if commit_requested {
            // The reply may have been lost after the daemon committed. Keep restart protection.
            ticket.phase = "verify".into();
            ticket.error = Some(format!("{error:#}"));
            ticket.save(&paths)?;
        } else {
            let _ = ipc::request(
                &paths,
                Request::CancelUpdate {
                    token: ticket.id.clone(),
                },
            );
            let _ = fs::remove_file(maintenance::pending(&paths));
        }
        let _ = atomic_write(&ticket.dir().join("error"), format!("{error:#}").as_bytes());
        return Err(error);
    }
    // All queued/active jobs were excluded and the adapter was reaped before CommitUpdate replied.
    let result = (|| -> Result<()> {
        wait_until(
            || {
                fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(paths.root.join("daemon.lock"))
                    .is_ok_and(|file| file.try_lock_exclusive().is_ok())
            },
            Duration::from_secs(15),
            "Background service has not stopped",
        )?;
        finish_installation(&mut ticket, &paths, launch)
    })();
    if let Err(error) = &result {
        ticket.phase = "verify".into();
        ticket.error = Some(format!("{error:#}"));
        ticket.save(&paths)?;
    }
    result
}
fn finish_installation(
    ticket: &mut Ticket,
    paths: &AppPaths,
    launch: impl FnOnce(&Ticket) -> Result<()>,
) -> Result<()> {
    ticket.phase = "installer".into();
    ticket.save(paths)?;
    let result = launch(ticket);
    ticket.phase = "verify".into();
    ticket.error = result.as_ref().err().map(|e| format!("{e:#}"));
    ticket.save(paths)?;
    result
}
fn launch(ticket: &Ticket) -> Result<()> {
    if ticket.plan.source == InstallSource::Homebrew {
        return Ok(());
    }
    let installer = ticket.plan.installer.as_ref().context("No installer")?;
    validate_verified(installer)?;
    if ticket.plan.manual {
        open::that(installer.path.parent().context("Invalid installer path")?)?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let directory = ticket
            .app
            .parent()
            .context("No installed application directory")?;
        // NSIS requires /D to be the final, unquoted command-line tail (including spaces).
        ensure!(
            Command::new(&installer.path)
                .arg("/LABELIZEUPDATE")
                .raw_arg(format!("/D={}", directory.display()))
                .status()?
                .success(),
            "Installer exited without success; reopen the app to verify"
        );
    }
    #[cfg(target_os = "macos")]
    {
        ensure!(
            Command::new("/usr/bin/open")
                .args(["-W", "-n", "-a", "Installer"])
                .arg(&installer.path)
                .status()?
                .success(),
            "Could not open macOS Installer"
        );
    }
    #[cfg(target_os = "linux")]
    {
        open::that(&installer.path)?;
    }
    Ok(())
}
/// Only the user acknowledges cancellation; success is derived from installed versions.
pub fn recover(paths: &AppPaths, acknowledge_old: bool) -> Result<String> {
    let _lock = lock(paths)?;
    let Some(ticket) = pending(paths)? else {
        return Ok(String::new());
    };
    platform::package_manager_idle()?;
    let current = env!("CARGO_PKG_VERSION");
    ensure!(current == ticket.plan.version || acknowledge_old, "Installation has not been confirmed. Finish the installer, or choose Resume current version after cancelling it.");
    for name in ["labelize-printer-daemon", "labelize-printer-updater"] {
        platform::component_version(
            &labelize_printer_core::platform::sibling_binary(name)?,
            current,
        )?;
    }
    #[cfg(unix)]
    platform::component_version(
        &labelize_printer_core::platform::sibling_binary("labelize-printer-ipp")?,
        current,
    )?;
    platform::receipt_version(current)?;
    if let Ok(response) = ipc::request(paths, Request::Status) {
        let running: labelize_printer_core::Snapshot = response.decode()?;
        ensure!(
            running.version == current,
            "Running background service has a different version; restart it before resuming"
        );
        let _ = ipc::request(paths, Request::CancelUpdate { token: ticket.id });
    }
    fs::remove_file(maintenance::pending(paths))?;
    Ok(if current == ticket.plan.version {
        format!("Updated to {current}")
    } else {
        format!("Resumed {current}; update was not installed")
    })
}

/// Use the cached helper for the notice so no installed executable stays mapped.
pub fn redirect_if_installing(paths: &AppPaths) -> Result<bool> {
    if !maintenance::pending(paths).exists() {
        return Ok(false);
    }
    if lock(paths).is_ok() {
        return Ok(false);
    }
    if let Some(ticket) = pending(paths)? {
        let binary = ticket.dir().join(if cfg!(windows) {
            "labelize-printer-updater.exe"
        } else {
            "labelize-printer-updater"
        });
        let mut command = Command::new(binary);
        platform::hide(&mut command);
        let _ = command
            .arg("--installation-notice")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_launch_failure_retains_recovery_record_and_lock_blocks_second_instance() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().to_path_buf()).unwrap();
        let mut ticket = Ticket {
            id: uuid::Uuid::new_v4().to_string(),
            data_dir: paths.root.clone(),
            parent: std::process::id(),
            app: paths.root.join("app"),
            plan: Plan {
                source: InstallSource::Homebrew,
                version: "99.0.0".into(),
                installer: None,
                manual: true,
            },
            phase: "preparing".into(),
            error: None,
        };
        let held = lock(&paths).unwrap();
        assert!(lock(&paths).is_err());
        assert!(finish_installation(&mut ticket, &paths, |_| bail!(
            "Installer executable missing"
        ))
        .is_err());
        let saved = pending(&paths).unwrap().unwrap();
        assert_eq!(saved.phase, "verify");
        assert!(saved.error.unwrap().contains("missing"));
        drop(held);
        assert!(lock(&paths).is_ok());
        assert!(recover(&paths, false).is_err()); // Old version cannot claim a completed upgrade.
        assert!(maintenance::pending(&paths).exists());
    }
    #[test]
    fn corrupted_handoff_does_not_silently_allow_service_restart() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().to_path_buf()).unwrap();
        fs::write(maintenance::pending(&paths), b"broken").unwrap();
        assert!(pending(&paths).is_err());
        assert!(maintenance::require_ready(&paths).is_err());
        assert!(recover(&paths, true).is_err());
        assert!(maintenance::pending(&paths).exists());
    }
}
