#[cfg(unix)]
mod cups;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use crate::{ipc, AppPaths, PlatformStatus, Request, Settings};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

pub const QUEUE_NAME: &str = "LabelizePrinter";
// The queue, IPC, data and service identifiers also identify older installations.
pub const DRIVER_URL: &str =
    "https://www.zebra.com/us/en/support-downloads/printers/printer-drivers.html";

pub fn sibling_binary(name: &str) -> Result<PathBuf> {
    let parent = std::env::current_exe()?
        .parent()
        .context("Executable has no directory")?
        .to_path_buf();
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    Ok(parent.join(filename))
}

pub fn asset(name: &str) -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let parent = exe.parent().context("Executable has no directory")?;
    let candidates = [
        parent.join("platform").join(name),
        parent.join("../Resources/platform").join(name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../platform")
            .join(name),
    ];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .context(format!("Missing platform resource: {name}"))
}

fn checked(command: &mut Command) -> Result<String> {
    #[cfg(windows)]
    windows::hide_console(command);
    let output = command.output().context("Cannot start platform utility")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

pub fn diagnostics(paths: &AppPaths) -> PlatformStatus {
    let mut status = PlatformStatus::default();
    #[cfg(unix)]
    cups::diagnostics(&mut status);
    #[cfg(target_os = "macos")]
    {
        status.autostart_installed = macos::autostart_installed();
    }
    #[cfg(target_os = "linux")]
    {
        status.autostart_installed = linux::autostart_installed();
    }
    #[cfg(windows)]
    windows::diagnostics(paths, &mut status);
    let _ = paths;
    status
}

pub fn install_queue(paths: &AppPaths, settings: &Settings) -> Result<()> {
    settings.validate()?;
    #[cfg(unix)]
    cups::install_queue(settings)?;
    #[cfg(windows)]
    windows::install_queue(paths, settings)?;
    set_autostart(paths, settings.autostart)
}

pub fn uninstall_queue(paths: &AppPaths) -> Result<()> {
    #[cfg(unix)]
    cups::uninstall_queue()?;
    #[cfg(windows)]
    windows::uninstall_queue()?;
    set_autostart(paths, false)
}

pub fn set_autostart(paths: &AppPaths, enabled: bool) -> Result<()> {
    let daemon = sibling_binary("labelize-printer-daemon")?;
    #[cfg(target_os = "macos")]
    macos::set_autostart(paths, &daemon, enabled)?;
    #[cfg(target_os = "linux")]
    linux::set_autostart(paths, &daemon, enabled)?;
    #[cfg(windows)]
    windows::set_autostart(paths, &daemon, enabled)?;
    Ok(())
}

pub fn ensure_daemon(paths: &AppPaths) -> Result<()> {
    crate::maintenance::require_ready(paths)?;
    #[cfg(target_os = "macos")]
    macos::refresh_studio_launch_agent(paths)?;
    if ipc::request(paths, Request::Status).is_ok() {
        return Ok(());
    }
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.root.join("daemon.log"))?;
    let mut command = Command::new(sibling_binary("labelize-printer-daemon")?);
    command
        .arg("serve")
        .env("LABELIZE_PRINTER_HOME", &paths.root)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(windows)]
    windows::hide_console(&mut command);
    command
        .spawn()
        .context("Cannot start the background service")?;
    for _ in 0..50 {
        if ipc::request(paths, Request::Status).is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!(
        "Background service did not start; see {}",
        paths.root.join("daemon.log").display()
    )
}
