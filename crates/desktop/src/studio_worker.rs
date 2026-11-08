use anyhow::{bail, Context, Result};
use labelize_printer_core::{
    document::{DocumentKind, PreviewPage, PreviewRequest},
    ipc,
    paths::{atomic_write, private_dir},
    platform, AppPaths, JobRecord, JobState, Request, Settings, WORKER_TIMEOUT_SECONDS,
};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Preview {
    pub directory: tempfile::TempDir,
    pub pages: Vec<PreviewPage>,
}

pub fn preview(
    paths: &AppPaths,
    bytes: Vec<u8>,
    kind: DocumentKind,
    settings: Settings,
) -> Result<Preview> {
    let cache = paths.root.join("previews");
    private_dir(&cache)?;
    let directory = tempfile::Builder::new()
        .prefix("document-")
        .tempdir_in(cache)?;
    let input = directory.path().join("input");
    atomic_write(&input, &bytes)?;
    let request_path = directory.path().join("request.json");
    atomic_write(
        &request_path,
        &serde_json::to_vec(&PreviewRequest {
            input,
            kind,
            settings,
        })?,
    )?;
    let log = directory.path().join("renderer.log");
    let mut command = Command::new(platform::sibling_binary("labelize-printer-daemon")?);
    command
        .arg("--data-dir")
        .arg(&paths.root)
        .arg("preview")
        .arg("--request")
        .arg(&request_path)
        .arg("--output")
        .arg(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().context(
        "Cannot start the preview renderer. Install matching desktop and daemon versions.",
    )?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!(
                    "{}",
                    fs::read_to_string(&log)
                        .unwrap_or_else(|_| format!("Preview renderer exited: {status}"))
                );
            }
            break;
        }
        if start.elapsed() > Duration::from_secs(WORKER_TIMEOUT_SECONDS) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Preview timed out");
        }
        thread::sleep(Duration::from_millis(40));
    }
    let pages = serde_json::from_slice(&fs::read(directory.path().join("pages.json"))?)?;
    Ok(Preview { directory, pages })
}

pub fn export(
    paths: &AppPaths,
    bytes: Vec<u8>,
    kind: DocumentKind,
    settings: Settings,
    title: String,
) -> Result<Vec<PathBuf>> {
    platform::ensure_daemon(paths)?;
    let directory = tempfile::Builder::new()
        .prefix("document-")
        .tempdir_in(paths.root.join("incoming"))?;
    let input = directory.path().join("input");
    atomic_write(&input, &bytes)?;
    let job: JobRecord = ipc::request(
        paths,
        Request::SubmitDocument {
            path: input,
            title,
            language: kind,
            settings,
        },
    )?
    .decode()?;
    // The daemon has durably copied the input before responding. Closing the
    // desktop after submission cannot interrupt this output job.
    drop(directory);
    let start = Instant::now();
    loop {
        let job: JobRecord = ipc::request(paths, Request::Job { id: job.id.clone() })?.decode()?;
        match job.state {
            JobState::Saved => return Ok(job.output_files),
            JobState::Failed => bail!("{}", job.error.unwrap_or_else(|| "Output failed".into())),
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(WORKER_TIMEOUT_SECONDS + 30) {
            bail!("The job is still in the background queue. Check Recent jobs before submitting again.");
        }
        thread::sleep(Duration::from_millis(200));
    }
}
