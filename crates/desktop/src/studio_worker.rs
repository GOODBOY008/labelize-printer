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
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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
    cancelled: Arc<AtomicBool>,
) -> Result<Preview> {
    anyhow::ensure!(!cancelled.load(Ordering::Acquire), "Preview cancelled");
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
    let child = command.spawn().context(
        "Cannot start the preview renderer. Install matching desktop and daemon versions.",
    )?;
    let mut guard = ChildGuard(child);
    let child = &mut guard.0;
    let start = Instant::now();
    loop {
        anyhow::ensure!(!cancelled.load(Ordering::Acquire), "Preview cancelled");
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

pub fn submit(
    paths: &AppPaths,
    bytes: Vec<u8>,
    kind: DocumentKind,
    settings: Settings,
    title: String,
) -> Result<JobRecord> {
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
    Ok(job)
}

pub fn wait_for_output(paths: &AppPaths, id: String) -> Result<Vec<PathBuf>> {
    let start = Instant::now();
    loop {
        let job: JobRecord = ipc::request(paths, Request::Job { id: id.clone() })?.decode()?;
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

// Always reap a cancelled or failed renderer before the queue starts another.
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Decode off the UI thread and keep display textures bounded. Full-resolution
/// PNGs remain on disk for pagination; exports use the original document.
type PreviewImages = (Arc<gpui::RenderImage>, Vec<(usize, Arc<gpui::RenderImage>)>);
pub fn load_preview_images(
    directory: PathBuf,
    pages: Vec<PathBuf>,
    page: usize,
    cancel: Arc<AtomicBool>,
) -> Result<PreviewImages> {
    static DECODE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _decode = DECODE.lock().unwrap_or_else(|e| e.into_inner());
    anyhow::ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
    fn load(path: PathBuf, width: u32, height: u32) -> Result<Arc<gpui::RenderImage>> {
        let image = image::open(path)?.thumbnail(width, height);
        let mut pixels = image.into_rgba8();
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Ok(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            pixels,
        )])))
    }
    let active = pages.get(page).context("Preview page is unavailable")?;
    let image = load(directory.join(active), 2048, 2048)?;
    let start = page.saturating_sub(2).min(pages.len().saturating_sub(5));
    let mut thumbnails = Vec::new();
    for (i, page) in pages.iter().enumerate().skip(start).take(5) {
        anyhow::ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
        thumbnails.push((i, load(directory.join(page), 120, 176)?));
    }
    Ok((image, thumbnails))
}
