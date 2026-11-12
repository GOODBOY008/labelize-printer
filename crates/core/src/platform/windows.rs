use super::{asset, checked, DRIVER_URL};
use crate::{paths::atomic_write, AppPaths, PlatformStatus, Settings};
use anyhow::{ensure, Result};
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};

pub(super) fn hide_console(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

pub(super) fn diagnostics(paths: &AppPaths, status: &mut PlatformStatus) {
    let result = checked(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
        "@(Get-PrinterDriver | Where-Object { $_.Name -match 'ZDesigner' -and $_.Name -match 'ZPL' } | Select-Object -ExpandProperty Name) | ConvertTo-Json -Compress"]));
    if let Ok(s) = result {
        status.windows_drivers = serde_json::from_str::<Vec<String>>(&s).unwrap_or_else(|_| {
            serde_json::from_str::<String>(&s)
                .map(|v| vec![v])
                .unwrap_or_default()
        });
    }
    status.queue_installed = checked(Command::new("powershell.exe").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "if (!(Get-Printer -Name LabelizePrinter -ErrorAction SilentlyContinue)) { exit 1 }",
    ]))
    .is_ok();
    status.autostart_installed = paths.root.join("autostart.enabled").exists();
    if status.windows_drivers.is_empty() {
        status.notes.push(
            "Install the official ZDesigner ZPL driver, then refresh printer detection.".into(),
        );
    }
}

pub(super) fn install_queue(paths: &AppPaths, settings: &Settings) -> Result<()> {
    ensure!(
        settings
            .windows_driver
            .as_ref()
            .is_some_and(|d| d.contains("ZDesigner") && d.contains("ZPL")),
        "Choose an installed ZDesigner ZPL driver first. Download: {DRIVER_URL}"
    );
    let config = paths.root.join("install.json");
    atomic_write(&config, &serde_json::to_vec(settings)?)?;
    checked(
        powershell_script("windows/install-queue.ps1")?
            .arg("-ConfigPath")
            .arg(config),
    )?;
    Ok(())
}

pub(super) fn uninstall_queue() -> Result<()> {
    checked(powershell_script("windows/install-queue.ps1")?.arg("-Remove"))?;
    Ok(())
}

pub(super) fn set_autostart(paths: &AppPaths, daemon: &Path, enabled: bool) -> Result<()> {
    let mut cmd = powershell_script("windows/autostart.ps1")?;
    cmd.arg("-Daemon")
        .arg(daemon)
        .arg("-DataDir")
        .arg(&paths.root);
    if !enabled {
        cmd.arg("-Remove");
    }
    checked(&mut cmd)?;
    let marker = paths.root.join("autostart.enabled");
    if enabled {
        atomic_write(&marker, b"enabled")?;
    } else {
        let _ = fs::remove_file(marker);
    }
    Ok(())
}

fn powershell_script(script: &str) -> Result<Command> {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(asset(script)?);
    Ok(command)
}
