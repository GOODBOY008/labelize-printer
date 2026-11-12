use super::checked;
use crate::{paths::atomic_write, AppPaths};
use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn autostart_installed() -> bool {
    user_unit_path().is_ok_and(|path| path.exists())
}

fn user_unit_path() -> Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("No config directory")?
        .config_dir()
        .join("systemd/user/labelize-printer.service"))
}

pub(super) fn set_autostart(paths: &AppPaths, daemon: &Path, enabled: bool) -> Result<()> {
    let file = user_unit_path()?;
    if enabled {
        fs::create_dir_all(file.parent().unwrap())?;
        let unit = format!("[Unit]\nDescription=Labelize Studio\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n[Service]\nType=simple\nExecStart={} serve\nEnvironment=LABELIZE_PRINTER_HOME={}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=graphical-session.target\n",
            systemd_quote(daemon), systemd_quote(&paths.root));
        atomic_write(&file, unit.as_bytes())?;
        checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        checked(Command::new("systemctl").args(["--user", "enable", "labelize-printer.service"]))?;
    } else {
        let _ = checked(Command::new("systemctl").args([
            "--user",
            "disable",
            "labelize-printer.service",
        ]));
        if file.exists() {
            fs::remove_file(file)?;
        }
        let _ = checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    }
    Ok(())
}

pub(super) fn run_lpadmin_as_admin(program: &str, args: &[&str]) -> Result<()> {
    checked(Command::new("pkexec").arg(program).args(args))?;
    Ok(())
}

fn systemd_quote(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('\n', "\\n")
    )
}
