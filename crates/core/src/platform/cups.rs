#[cfg(target_os = "linux")]
use super::linux::run_lpadmin_as_admin;
#[cfg(target_os = "macos")]
use super::macos::run_lpadmin_as_admin;
use super::{sibling_binary, QUEUE_NAME};
use crate::{PlatformStatus, Settings};
use anyhow::{ensure, Result};
use std::process::Command;

const LPADMIN: &str = "/usr/sbin/lpadmin";

pub(super) fn diagnostics(status: &mut PlatformStatus) {
    status.queue_installed = queue_installed();
    if !sibling_binary("labelize-printer-ipp").is_ok_and(|path| path.is_file()) {
        status.notes.push(
            "IPP adapter is missing. Run bash scripts/build/native.sh or install the complete package."
                .into(),
        );
    }
}

pub(super) fn install_queue(settings: &Settings) -> Result<()> {
    ensure_own_queue()?;
    let url = format!("ipp://127.0.0.1:{}/ipp/print", settings.ipp_port);
    lpadmin(&[
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
    ])
}

pub(super) fn uninstall_queue() -> Result<()> {
    ensure_own_queue()?;
    if queue_installed() {
        lpadmin(&["-x", QUEUE_NAME])?;
    }
    Ok(())
}

fn queue_installed() -> bool {
    Command::new("lpstat")
        .args(["-p", QUEUE_NAME])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn ensure_own_queue() -> Result<()> {
    let output = Command::new("lpstat").args(["-v", QUEUE_NAME]).output()?;
    if output.status.success() {
        let uri = String::from_utf8_lossy(&output.stdout);
        ensure!(
            uri.contains("ipp://127.0.0.1:") && uri.trim_end().ends_with("/ipp/print"),
            "A different printer already uses the LabelizePrinter queue name"
        );
    }
    Ok(())
}

fn lpadmin(args: &[&str]) -> Result<()> {
    if !Command::new(LPADMIN)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
    {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        run_lpadmin_as_admin(LPADMIN, args)?;
    }
    Ok(())
}
