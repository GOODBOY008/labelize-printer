use super::{asset, checked, sibling_binary};
use crate::{paths::atomic_write, AppPaths};
use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn autostart_installed() -> bool {
    launch_agent_path().is_ok_and(|path| path.exists())
}

fn launch_agent_path() -> Result<PathBuf> {
    Ok(directories::UserDirs::new()
        .context("No home directory")?
        .home_dir()
        .join("Library/LaunchAgents/com.labelize.printer.plist"))
}

pub(super) fn set_autostart(paths: &AppPaths, daemon: &Path, enabled: bool) -> Result<()> {
    let file = launch_agent_path()?;
    if enabled {
        fs::create_dir_all(file.parent().unwrap())?;
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>Label</key><string>com.labelize.printer</string>
<key>ProgramArguments</key><array><string>{}</string><string>serve</string></array>
<key>EnvironmentVariables</key><dict><key>LABELIZE_PRINTER_HOME</key><string>{}</string></dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string>
</dict></plist>"#,
            xml(daemon),
            xml(&paths.root),
            xml(&paths.root.join("daemon.log")),
            xml(&paths.root.join("daemon.log"))
        );
        atomic_write(&file, plist.as_bytes())?;
    } else if file.exists() {
        fs::remove_file(file)?;
    }
    // Registration takes effect next login; the already-running instance is retained.
    Ok(())
}

pub(super) fn run_lpadmin_as_admin(program: &str, args: &[&str]) -> Result<()> {
    let command = std::iter::once(program)
        .chain(args.iter().copied())
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");
    checked(
        Command::new("osascript")
            .arg(asset("macos/admin.applescript")?)
            .arg(command),
    )?;
    Ok(())
}

fn xml(path: &Path) -> String {
    path.to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(super) fn refresh_studio_launch_agent(paths: &AppPaths) -> Result<()> {
    let file = launch_agent_path()?;
    let daemon = sibling_binary("labelize-printer-daemon")?;
    if !file.is_file()
        || daemon
            != Path::new("/Applications/Labelize Studio.app/Contents/MacOS/labelize-printer-daemon")
    {
        return Ok(());
    }
    let json = checked(
        Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&file),
    )?;
    let record: serde_json::Value = serde_json::from_str(&json)?;
    if studio_launch_agent_needs_refresh(&record, &paths.root) {
        set_autostart(paths, &daemon, true)?;
    }
    Ok(())
}

fn studio_launch_agent_needs_refresh(record: &serde_json::Value, root: &Path) -> bool {
    record["Label"] == "com.labelize.printer"
        && record["ProgramArguments"][0]
            == "/Applications/Labelize Printer.app/Contents/MacOS/labelize-printer-daemon"
        && record["EnvironmentVariables"]["LABELIZE_PRINTER_HOME"].as_str() == root.to_str()
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn only_refreshes_the_existing_users_legacy_launch_agent() {
        let root = Path::new("/users/test/LabelizePrinter");
        let mut record = serde_json::json!({
            "Label": "com.labelize.printer",
            "ProgramArguments": ["/Applications/Labelize Printer.app/Contents/MacOS/labelize-printer-daemon", "serve"],
            "EnvironmentVariables": {"LABELIZE_PRINTER_HOME": root}
        });
        assert!(studio_launch_agent_needs_refresh(&record, root));
        assert!(!studio_launch_agent_needs_refresh(
            &record,
            Path::new("/private/qa")
        ));
        record["ProgramArguments"][0] =
            "/Applications/Labelize Studio.app/Contents/MacOS/labelize-printer-daemon".into();
        assert!(!studio_launch_agent_needs_refresh(&record, root));
        record["ProgramArguments"][0] = "/private/qa/labelize-printer-daemon".into();
        assert!(!studio_launch_agent_needs_refresh(&record, root));
        assert!(!studio_launch_agent_needs_refresh(
            &serde_json::json!({}),
            root
        ));
    }
}
