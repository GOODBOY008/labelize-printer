#[cfg(target_os = "macos")]
use crate::brew_installed;
use crate::{InstallSource, Target};
use anyhow::{bail, ensure, Context, Result};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Bound local discovery as well as network I/O; a hung package manager must not stall startup.
pub fn output(command: &mut Command, timeout: Duration) -> Result<String> {
    hide(command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(4 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let until = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "System command failed: {status}");
            return Ok(String::from_utf8(
                rx.recv_timeout(Duration::from_secs(1))??,
            )?);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            bail!("System command timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn hide(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}
pub fn target() -> Result<Target> {
    #[cfg(target_os = "macos")]
    {
        // The hardware capability remains true even in an Intel process under Rosetta.
        let arm = output(
            Command::new("/usr/sbin/sysctl").args(["-n", "hw.optional.arm64"]),
            Duration::from_secs(5),
        )?;
        Ok(if arm.trim() == "1" {
            Target::MacArm
        } else {
            Target::MacIntel
        })
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok(Target::WindowsX64)
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok(Target::LinuxX64)
    }
    #[cfg(not(any(
        target_os = "macos",
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        bail!("No installer is available for this platform")
    }
}
pub fn source() -> InstallSource {
    #[cfg(target_os = "macos")]
    {
        let candidates = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"];
        let mut unknown = false;
        for brew in candidates.into_iter().filter(|p| Path::new(p).is_file()) {
            match output(
                Command::new(brew)
                    .args(["info", "--json=v2", "--installed"])
                    .env("HOMEBREW_NO_AUTO_UPDATE", "1")
                    .env("HOMEBREW_NO_ANALYTICS", "1"),
                Duration::from_secs(10),
            )
            .and_then(|s| brew_installed(s.as_bytes()))
            {
                Ok(true) => return InstallSource::Homebrew,
                Ok(false) => (),
                Err(_) => unknown = true,
            }
        }
        if unknown {
            InstallSource::Unknown
        } else {
            InstallSource::Package
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        InstallSource::Package
    }
}
pub fn deb_handler() -> bool {
    #[cfg(target_os = "linux")]
    {
        output(
            Command::new("xdg-mime").args([
                "query",
                "default",
                "application/vnd.debian.binary-package",
            ]),
            Duration::from_secs(5),
        )
        .is_ok_and(|s| !s.trim().is_empty())
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn apt_command(path: &Path) -> String {
    format!("sudo apt install {}", quote(&path.to_string_lossy()))
}

pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(windows)]
    {
        unsafe {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, WAIT_TIMEOUT},
                System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
            };
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }
}
/// Dpkg uses POSIX record locks, not flock. Query without acquiring or removing them.
pub fn package_manager_idle() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        for path in ["/var/lib/dpkg/lock-frontend", "/var/lib/dpkg/lock"] {
            if !Path::new(path).exists() {
                continue;
            }
            let file = std::fs::File::open(path)?;
            let mut lock: libc::flock = unsafe { std::mem::zeroed() };
            lock.l_type = libc::F_WRLCK as _;
            lock.l_whence = libc::SEEK_SET as _;
            ensure!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) } != -1,
                "Cannot inspect package manager lock"
            );
            ensure!(
                lock.l_type == libc::F_UNLCK as libc::c_short,
                "The system package manager is still installing; wait and retry"
            );
        }
    }
    Ok(())
}
pub fn component_version(binary: &Path, expected: &str) -> Result<()> {
    let result = output(
        Command::new(binary).arg("--version"),
        Duration::from_secs(10),
    )
    .with_context(|| format!("Cannot verify component {}", binary.display()))?;
    ensure!(
        result.split_whitespace().last() == Some(expected),
        "Component version mismatch: {} ({})",
        binary.display(),
        result.trim()
    );
    Ok(())
}
pub fn receipt_version(expected: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let text = output(
            Command::new("/usr/sbin/pkgutil").args(["--pkg-info", "com.labelize.printer"]),
            Duration::from_secs(10),
        )?;
        ensure!(
            text.lines()
                .any(|l| l.trim() == format!("version: {expected}")),
            "macOS package receipt does not match the app; finish or repeat installation"
        );
    }
    #[cfg(target_os = "linux")]
    {
        let text = output(
            Command::new("dpkg-query").args(["-W", "-f=${Status} ${Version}", "labelize-printer"]),
            Duration::from_secs(10),
        )?;
        ensure!(
            text.trim() == format!("install ok installed {expected}"),
            "DEB is not fully configured at the app version; finish installation"
        );
    }
    #[cfg(windows)]
    {
        let text = output(
            Command::new("reg.exe").args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LabelizePrinter",
                "/v",
                "DisplayVersion",
            ]),
            Duration::from_secs(10),
        )?;
        ensure!(
            text.split_whitespace().last() == Some(expected),
            "Installer registration does not match the application version"
        );
    }
    Ok(())
}
/// A second desktop must not keep Windows executables open during replacement.
pub fn installation_notice() {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("/usr/bin/osascript").args(["-e", "display dialog \"安装器仍在运行，请完成或关闭它后再打开应用。\\nAn installer is still running. Finish or close it before reopening Labelize Studio.\" with title \"Labelize Studio\" buttons {\"OK\"} default button 1"]).status();
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let message: Vec<u16> =
            "An installer is still running. Finish or close it before reopening Labelize Studio.\0"
                .encode_utf16()
                .collect();
        let title: Vec<u16> = "Labelize Studio\0".encode_utf16().collect();
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("zenity").args(["--info", "--title=Labelize Studio", "--text=An installer is still running. Finish or close it before reopening Labelize Studio."]).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_paths_are_literal() {
        assert_eq!(
            apt_command(Path::new("/tmp/a'$(touch bad) 文件.deb")),
            "sudo apt install '/tmp/a'\\''$(touch bad) 文件.deb'"
        );
    }
    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::process::id()));
    }
}
