use crate::{paths::atomic_write, *};
use anyhow::{bail, Context, Result};
#[cfg(unix)]
use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

pub const QUEUE_NAME: &str = "LabelizePrinter";
pub const DRIVER_URL: &str =
    "https://www.zebra.com/us/en/support-downloads/printer-software/printer-drivers.html";

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
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().context("Cannot start platform utility")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

pub fn diagnostics(paths: &AppPaths) -> PlatformStatus {
    let mut status = PlatformStatus::default();
    #[cfg(unix)]
    {
        status.queue_installed = Command::new("lpstat")
            .args(["-p", QUEUE_NAME])
            .output()
            .is_ok_and(|o| o.status.success());
        if !sibling_binary("labelize-printer-ipp").is_ok_and(|p| p.is_file()) {
            status.notes.push("IPP adapter is missing; run scripts/build-native.sh or install the complete package".into());
        }
    }
    #[cfg(target_os = "macos")]
    {
        status.autostart_installed = launch_agent_path().is_ok_and(|p| p.exists());
    }
    #[cfg(target_os = "linux")]
    {
        status.autostart_installed = user_unit_path().is_ok_and(|p| p.exists());
    }
    #[cfg(windows)]
    {
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
            status.notes.push(format!(
                "Install the official ZDesigner ZPL driver: {DRIVER_URL}"
            ));
        }
    }
    let _ = paths;
    status
}

pub fn install_queue(paths: &AppPaths, settings: &Settings) -> Result<()> {
    settings.validate()?;
    #[cfg(unix)]
    {
        ensure_own_queue()?;
        let url = format!("ipp://127.0.0.1:{}/ipp/print", settings.ipp_port);
        let args = [
            "-p",
            QUEUE_NAME,
            "-E",
            "-v",
            &url,
            "-m",
            "everywhere",
            "-D",
            "Labelize Printer",
            "-o",
            "printer-is-shared=false",
            "-o",
            "printer-error-policy=retry-job",
        ];
        if !Command::new("/usr/sbin/lpadmin")
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
        {
            #[cfg(target_os = "macos")]
            {
                let command = std::iter::once("/usr/sbin/lpadmin")
                    .chain(args)
                    .map(shell_quote)
                    .collect::<Vec<_>>()
                    .join(" ");
                checked(
                    Command::new("osascript")
                        .arg(asset("macos/admin.applescript")?)
                        .arg(command),
                )?;
            }
            #[cfg(target_os = "linux")]
            {
                checked(Command::new("pkexec").arg("/usr/sbin/lpadmin").args(args))?;
            }
        }
    }
    #[cfg(windows)]
    {
        anyhow::ensure!(
            settings
                .windows_driver
                .as_ref()
                .is_some_and(|d| d.contains("ZDesigner") && d.contains("ZPL")),
            "Choose an installed ZDesigner ZPL driver first. Download: {DRIVER_URL}"
        );
        let config = paths.root.join("install.json");
        atomic_write(&config, &serde_json::to_vec(settings)?)?;
        checked(
            Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(asset("windows/install-queue.ps1")?)
                .arg("-ConfigPath")
                .arg(config),
        )?;
    }
    set_autostart(paths, settings.autostart)?;
    Ok(())
}

pub fn uninstall_queue(paths: &AppPaths) -> Result<()> {
    #[cfg(unix)]
    {
        ensure_own_queue()?;
        if !Command::new("lpstat")
            .args(["-p", QUEUE_NAME])
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return set_autostart(paths, false);
        }
        if !Command::new("/usr/sbin/lpadmin")
            .args(["-x", QUEUE_NAME])
            .output()
            .is_ok_and(|o| o.status.success())
        {
            #[cfg(target_os = "macos")]
            {
                checked(
                    Command::new("osascript")
                        .arg(asset("macos/admin.applescript")?)
                        .arg(format!("/usr/sbin/lpadmin -x {QUEUE_NAME}")),
                )?;
            }
            #[cfg(target_os = "linux")]
            {
                checked(Command::new("pkexec").args(["/usr/sbin/lpadmin", "-x", QUEUE_NAME]))?;
            }
        }
    }
    #[cfg(windows)]
    {
        checked(
            Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(asset("windows/install-queue.ps1")?)
                .arg("-Remove"),
        )?;
    }
    set_autostart(paths, false)
}

#[cfg(unix)]
fn ensure_own_queue() -> Result<()> {
    let output = Command::new("lpstat").args(["-v", QUEUE_NAME]).output()?;
    if output.status.success() {
        let uri = String::from_utf8_lossy(&output.stdout);
        anyhow::ensure!(
            uri.contains("ipp://127.0.0.1:") && uri.trim_end().ends_with("/ipp/print"),
            "A different printer already uses the LabelizePrinter queue name"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_agent_path() -> Result<PathBuf> {
    Ok(directories::UserDirs::new()
        .context("No home directory")?
        .home_dir()
        .join("Library/LaunchAgents/com.labelize.printer.plist"))
}
#[cfg(target_os = "linux")]
fn user_unit_path() -> Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("No config directory")?
        .config_dir()
        .join("systemd/user/labelize-printer.service"))
}

pub fn set_autostart(paths: &AppPaths, enabled: bool) -> Result<()> {
    let daemon = sibling_binary("labelize-printer-daemon")?;
    #[cfg(target_os = "macos")]
    {
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
                xml(&daemon),
                xml(&paths.root),
                xml(&paths.root.join("daemon.log")),
                xml(&paths.root.join("daemon.log"))
            );
            atomic_write(&file, plist.as_bytes())?;
        } else if file.exists() {
            fs::remove_file(file)?;
        }
        // Registration takes effect next login; the already-running instance is retained.
    }
    #[cfg(target_os = "linux")]
    {
        let file = user_unit_path()?;
        if enabled {
            fs::create_dir_all(file.parent().unwrap())?;
            let unit = format!("[Unit]\nDescription=Labelize Printer\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n[Service]\nType=simple\nExecStart={} serve\nEnvironment=LABELIZE_PRINTER_HOME={}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=graphical-session.target\n",
                systemd_quote(&daemon), systemd_quote(&paths.root));
            atomic_write(&file, unit.as_bytes())?;
            checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
            checked(Command::new("systemctl").args([
                "--user",
                "enable",
                "labelize-printer.service",
            ]))?;
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
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(asset("windows/autostart.ps1")?)
            .arg("-Daemon")
            .arg(&daemon)
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
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn xml(path: &Path) -> String {
    path.to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn ensure_daemon(paths: &AppPaths) -> Result<()> {
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
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
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
