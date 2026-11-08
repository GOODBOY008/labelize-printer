use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use fs2::FileExt;
use interprocess::local_socket::prelude::*;
use labelize_printer_core::{
    ipc,
    paths::{atomic_write, private_file},
    platform, *,
};
use std::{
    fs,
    io::{BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "Labelize Printer background service and local control client"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Serve,
    #[command(hide = true)]
    Preview {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Status,
    Refresh,
    Submit {
        path: PathBuf,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value = "Label print job")]
        title: String,
    },
    IngestRaster {
        manifest: PathBuf,
        #[arg(long)]
        wait: bool,
    },
    #[command(hide = true)]
    BeginRaster {
        title: String,
        source: String,
    },
    #[command(hide = true)]
    AbortRaster {
        id: String,
        error: String,
    },
    Job {
        id: String,
    },
    Retry {
        id: String,
        #[arg(long)]
        wait: bool,
    },
    TestPrint {
        #[arg(long)]
        wait: bool,
    },
    Settings {
        #[arg(long)]
        file: Option<PathBuf>,
    },
    Install,
    Uninstall,
    Stop,
    Watch,
    #[command(hide = true)]
    Worker {
        id: String,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = match cli.data_dir {
        Some(p) => AppPaths::new(p)?,
        None => AppPaths::discover()?,
    };
    let (request, wait) = match cli.command {
        Action::Serve => return serve(paths),
        Action::Preview { request, output } => {
            let request: document::PreviewRequest =
                serde_json::from_slice(&document::read_input(&request)?)?;
            document::preview(&request, &output)?;
            return Ok(());
        }
        Action::Worker { id } => return worker(&paths, &id),
        Action::BeginRaster { title, source } => {
            let job: JobRecord =
                ipc::request(&paths, Request::BeginRaster { title, source })?.decode()?;
            println!("{}", job.id);
            return Ok(());
        }
        Action::Status => (Request::Status, false),
        Action::Refresh => (Request::RefreshPlatform, false),
        Action::Submit { path, wait, title } => {
            let path = fs::canonicalize(path)?;
            let kind = document::DocumentKind::detect(&path, &document::read_input(&path)?);
            if kind == document::DocumentKind::Zpl {
                (Request::SubmitZpl { path, title }, wait)
            } else {
                let snapshot: Snapshot = ipc::request(&paths, Request::Status)?.decode()?;
                (
                    Request::SubmitDocument {
                        path,
                        title,
                        language: kind,
                        settings: snapshot.settings,
                    },
                    wait,
                )
            }
        }
        Action::IngestRaster { manifest, wait } => (
            Request::SubmitRaster {
                manifest: fs::canonicalize(manifest)?,
            },
            wait,
        ),
        Action::AbortRaster { id, error } => (Request::AbortRaster { id, error }, false),
        Action::Job { id } => (Request::Job { id }, false),
        Action::Retry { id, wait } => (Request::Retry { id }, wait),
        Action::TestPrint { wait } => (Request::TestPrint, wait),
        Action::Install => (Request::Install, false),
        Action::Uninstall => (Request::Uninstall, false),
        Action::Stop => (Request::Shutdown, false),
        Action::Watch => (Request::Watch, false),
        Action::Settings { file } => {
            if let Some(file) = file {
                (
                    Request::SaveSettings {
                        settings: serde_json::from_slice(&fs::read(file)?)?,
                    },
                    false,
                )
            } else {
                let snapshot: Snapshot = ipc::request(&paths, Request::Status)?.decode()?;
                println!("{}", serde_json::to_string_pretty(&snapshot.settings)?);
                return Ok(());
            }
        }
    };
    if matches!(request, Request::Install | Request::Uninstall) {
        platform::ensure_daemon(&paths)?;
    }
    if matches!(request, Request::Shutdown) && ipc::request(&paths, Request::Status).is_err() {
        return Ok(());
    }
    if matches!(request, Request::Watch) {
        let mut stream = interprocess::local_socket::Stream::connect(ipc::endpoint(&paths)?)?;
        ipc::write_json(
            &mut stream,
            &AuthenticatedRequest {
                token: paths.token()?,
                request,
            },
        )?;
        let mut reader = BufReader::new(stream);
        loop {
            let response: Response = ipc::read_json(&mut reader)?;
            println!("{}", serde_json::to_string(&response)?);
        }
    }
    let response = ipc::request(&paths, request)?;
    if wait {
        let job: JobRecord = response.decode()?;
        let result = wait_for_job(&paths, &job.id)?;
        println!("{}", serde_json::to_string(&result)?);
    } else {
        let data: serde_json::Value = response.decode()?;
        println!("{}", serde_json::to_string_pretty(&data)?);
    }
    Ok(())
}

fn wait_for_job(paths: &AppPaths, id: &str) -> Result<JobRecord> {
    loop {
        let job: JobRecord = ipc::request(paths, Request::Job { id: id.into() })?.decode()?;
        match job.state {
            JobState::Saved => return Ok(job),
            JobState::Failed => bail!("{}", job.error.unwrap_or_else(|| "Print failed".into())),
            _ => (),
        }
        thread::sleep(Duration::from_millis(200));
    }
}

struct Service {
    paths: AppPaths,
    raw_listening: AtomicBool,
    platform: Mutex<PlatformStatus>,
    adapter: Mutex<Option<Child>>,
    configuration: Mutex<()>,
}

fn serve(paths: AppPaths) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.root.join("daemon.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        // Successful exit prevents the login supervisor repeatedly starting a second daemon.
        if ipc::request(&paths, Request::Status).is_ok() {
            return Ok(());
        }
        bail!("Another background instance owns the data directory");
    }
    paths.ensure_token()?;
    let store = Store::open(paths.clone())?;
    store.recover()?;
    let mut settings = store.settings()?;
    let raw_listener = bind_raw(&mut settings);
    if !settings.setup_complete {
        settings.ipp_port = available_port(settings.ipp_port, settings.raw_port)?;
    }
    store.save_settings(&settings)?;
    let listener = ipc::listener(&paths)?;
    let service = Arc::new(Service {
        platform: Mutex::new(platform::diagnostics(&paths)),
        paths: paths.clone(),
        raw_listening: AtomicBool::new(raw_listener.is_ok()),
        adapter: Mutex::new(None),
        configuration: Mutex::new(()),
    });
    if let Err(error) = start_adapter(&service, &settings) {
        service
            .platform
            .lock()
            .unwrap()
            .notes
            .push(format!("IPP: {error:#}"));
    }
    match raw_listener {
        Ok(listener) => {
            let service = service.clone();
            thread::spawn(move || receive_raw(listener, service));
        }
        Err(error) => service
            .platform
            .lock()
            .unwrap()
            .notes
            .push(format!("RAW listener: {error:#}")),
    }
    let scheduler_paths = paths.clone();
    thread::spawn(move || schedule(scheduler_paths));
    #[cfg(unix)]
    {
        let service = service.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            let stopped = service
                .adapter
                .lock()
                .unwrap()
                .as_mut()
                .is_some_and(|child| child.try_wait().is_ok_and(|s| s.is_some()));
            if stopped {
                let _configuration = service.configuration.lock().unwrap();
                if let Ok(store) = Store::open(service.paths.clone()) {
                    if let Ok(jobs) = store.jobs(1000) {
                        for job in jobs.into_iter().filter(|j| {
                            j.state == JobState::Receiving && j.source.starts_with("ipp:")
                        }) {
                            let _ = store.transition(
                                &job.id,
                                JobState::Failed,
                                Some(
                                    "IPP adapter stopped while receiving; resubmit the task".into(),
                                ),
                            );
                        }
                    }
                    if let Ok(settings) = store.settings() {
                        if let Err(error) = start_adapter(&service, &settings) {
                            eprintln!("IPP restart: {error:#}");
                        }
                    }
                }
            }
        });
    }
    let connections = Arc::new(AtomicUsize::new(0));
    eprintln!("Labelize Printer ready; data={}", paths.root.display());
    for connection in listener.incoming() {
        let connection = match connection {
            Ok(c) => c,
            Err(e) => {
                eprintln!("IPC accept: {e}");
                continue;
            }
        };
        if connections.fetch_add(1, Ordering::SeqCst) >= 16 {
            connections.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let service = service.clone();
        let connections = connections.clone();
        thread::spawn(move || {
            let result = handle_connection(connection, &service);
            if let Err(error) = result {
                eprintln!("IPC: {error:#}");
            }
            connections.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

fn snapshot(service: &Service) -> Result<Snapshot> {
    let store = Store::open(service.paths.clone())?;
    let mut platform = service.platform.lock().unwrap().clone();
    platform.ipp_running = service
        .adapter
        .lock()
        .unwrap()
        .as_mut()
        .is_some_and(|c| c.try_wait().is_ok_and(|s| s.is_none()));
    Ok(Snapshot {
        version: env!("CARGO_PKG_VERSION").into(),
        settings: store.settings()?,
        jobs: store.jobs(100)?,
        raw_listening: service.raw_listening.load(Ordering::SeqCst),
        platform,
    })
}

fn handle_connection(stream: interprocess::local_socket::Stream, service: &Service) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let envelope: AuthenticatedRequest = ipc::read_json(&mut reader)?;
    if envelope.token != service.paths.token()? {
        ipc::write_json(reader.get_mut(), &Response::error("Access denied"))?;
        return Ok(());
    }
    if matches!(envelope.request, Request::Watch) {
        loop {
            let response = Response::success(snapshot(service)?);
            // Heartbeats also detect disconnected subscribers with no job changes.
            ipc::write_json(reader.get_mut(), &response)?;
            thread::sleep(Duration::from_secs(1));
        }
    }
    let shutdown = matches!(envelope.request, Request::Shutdown);
    let response = match dispatch(envelope.request, service) {
        Ok(data) => Response::success(data),
        Err(error) => Response::error(format!("{error:#}")),
    };
    ipc::write_json(reader.get_mut(), &response)?;
    if shutdown && response.ok {
        thread::spawn(|| {
            thread::sleep(Duration::from_millis(200));
            std::process::exit(0);
        });
    }
    Ok(())
}

fn dispatch(request: Request, service: &Service) -> Result<serde_json::Value> {
    let store = Store::open(service.paths.clone())?;
    Ok(match request {
        Request::Status => serde_json::to_value(snapshot(service)?)?,
        Request::RefreshPlatform => {
            *service.platform.lock().unwrap() = platform::diagnostics(&service.paths);
            serde_json::to_value(snapshot(service)?)?
        }
        Request::SaveSettings { settings } => {
            let _configuration = service.configuration.lock().unwrap();
            settings.validate()?;
            let old = store.settings()?;
            anyhow::ensure!(settings.raw_port == old.raw_port && settings.ipp_port == old.ipp_port,
                "Listener ports are selected at startup; use queue repair after restarting with new ports");
            fs::create_dir_all(&settings.output_dir).context("Cannot create output directory")?;
            let probe = settings
                .output_dir
                .join(format!(".labelize-write-check-{}", std::process::id()));
            atomic_write(&probe, b"write check")?;
            fs::remove_file(probe)?;
            if settings.autostart != old.autostart || settings.setup_complete {
                platform::set_autostart(&service.paths, settings.autostart)?;
            }
            store.save_settings(&settings)?;
            serde_json::to_value(settings)?
        }
        Request::SubmitZpl { path, title } => {
            serde_json::to_value(store.import_zpl(&path, &title, "local-zpl")?)?
        }
        Request::SubmitDocument {
            path,
            title,
            language,
            settings,
        } => serde_json::to_value(store.import_document(&path, &title, language, settings)?)?,
        Request::SubmitRaster { manifest } => {
            serde_json::to_value(store.import_raster(&manifest)?)?
        }
        Request::BeginRaster { title, source } => {
            let _configuration = service.configuration.lock().unwrap();
            anyhow::ensure!(
                source.starts_with("ipp-") && source.len() <= 100,
                "Invalid IPP source ID"
            );
            serde_json::to_value(store.create(
                &title,
                &format!("ipp:{source}"),
                PrintJobInput::RasterPages { pages: Vec::new() },
                JobState::Receiving,
            )?)?
        }
        Request::AbortRaster { id, error } => {
            let job = store.job(&id)?;
            anyhow::ensure!(job.source.starts_with("ipp:"), "Not an IPP task");
            if job.state == JobState::Receiving {
                serde_json::to_value(store.transition(
                    &id,
                    JobState::Failed,
                    Some(error.chars().take(4000).collect()),
                )?)?
            } else {
                serde_json::to_value(job)?
            }
        }
        Request::Job { id } => serde_json::to_value(store.job(&id)?)?,
        Request::Retry { id } => serde_json::to_value(store.retry(&id)?)?,
        Request::TestPrint => {
            let settings = store.settings()?;
            let width = (settings.width_mm * settings.dpmm as f64).round() as u32;
            let height = (settings.height_mm * settings.dpmm as f64).round() as u32;
            let data = format!("^XA^PW{width}^LL{height}^FO24,24^A0N,32,32^FDLabelize Printer^FS^FO24,80^A0N,22,22^FDZPL to PDF / PNG^FS^FO24,130^BY2^BCN,80,Y,N,N^FD1234567890^FS^XZ");
            let path = service.paths.root.join("incoming/test.zpl");
            atomic_write(&path, data.as_bytes())?;
            serde_json::to_value(store.import_zpl(&path, "Labelize test print", "test")?)?
        }
        Request::Install => {
            let _configuration = service.configuration.lock().unwrap();
            anyhow::ensure!(
                !store.has_active_jobs()?,
                "Wait for active print jobs before changing printer registration"
            );
            let mut settings = store.settings()?;
            #[cfg(unix)]
            start_adapter(service, &settings)?;
            platform::install_queue(&service.paths, &settings)?;
            settings.setup_complete = true;
            store.save_settings(&settings)?;
            *service.platform.lock().unwrap() = platform::diagnostics(&service.paths);
            serde_json::to_value(snapshot(service)?)?
        }
        Request::Uninstall => {
            let _configuration = service.configuration.lock().unwrap();
            anyhow::ensure!(
                !store.has_active_jobs()?,
                "Wait for active print jobs before removing the queue"
            );
            platform::uninstall_queue(&service.paths)?;
            let mut settings = store.settings()?;
            settings.setup_complete = false;
            settings.autostart = false;
            store.save_settings(&settings)?;
            *service.platform.lock().unwrap() = platform::diagnostics(&service.paths);
            serde_json::to_value(snapshot(service)?)?
        }
        Request::Shutdown => {
            let _configuration = service.configuration.lock().unwrap();
            anyhow::ensure!(
                !store.has_active_jobs()?,
                "Wait for active jobs before stopping the background service"
            );
            if let Some(mut adapter) = service.adapter.lock().unwrap().take() {
                let _ = adapter.kill();
                let _ = adapter.wait();
            }
            serde_json::json!({ "stopping": true })
        }
        Request::Watch => unreachable!(),
    })
}

fn available_port(preferred: u16, excluded: u16) -> Result<u16> {
    for port in preferred..preferred.saturating_add(100) {
        if port != excluded && TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return Ok(port);
        }
    }
    bail!("No available localhost port near {preferred}")
}

fn bind_raw(settings: &mut Settings) -> Result<TcpListener> {
    if settings.setup_complete {
        return Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, settings.raw_port))?);
    }
    for port in settings.raw_port..settings.raw_port.saturating_add(100) {
        if port == settings.ipp_port {
            continue;
        }
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            settings.raw_port = port;
            return Ok(listener);
        }
    }
    bail!("Cannot bind a local RAW print port")
}

fn receive_raw(listener: TcpListener, service: Arc<Service>) {
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(error) => {
                eprintln!("RAW accept: {error}");
                continue;
            }
        };
        if active.fetch_add(1, Ordering::SeqCst) >= 4 {
            active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let paths = service.paths.clone();
        let active = active.clone();
        thread::spawn(move || {
            if let Err(error) = spool_raw(stream, paths) {
                eprintln!("RAW job: {error:#}");
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
    }
    service.raw_listening.store(false, Ordering::SeqCst);
}

fn spool_raw(mut stream: TcpStream, paths: AppPaths) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(RAW_IDLE_TIMEOUT_SECONDS)))?;
    let store = Store::open(paths.clone())?;
    let mut job = store.create(
        "Windows / RAW print",
        "raw-tcp",
        PrintJobInput::Zpl {
            file: "input.zpl".into(),
        },
        JobState::Receiving,
    )?;
    let result = (|| -> Result<()> {
        let path = paths.job_dir(&job.id)?.join("input.zpl");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        private_file(&path)?;
        let mut buffer = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let count = stream
                .read(&mut buffer)
                .context("RAW reception interrupted or idle for 30 seconds")?;
            if count == 0 {
                break;
            }
            total += count as u64;
            anyhow::ensure!(total <= MAX_INPUT_BYTES, "RAW job exceeds 64 MiB");
            file.write_all(&buffer[..count])?;
        }
        anyhow::ensure!(total > 0, "Empty RAW connection");
        file.sync_all()?;
        job.transport_complete = true;
        job.state = JobState::Queued;
        store.put(&job)?;
        Ok(())
    })();
    if let Err(error) = result {
        store.transition(&job.id, JobState::Failed, Some(format!("{error:#}")))?;
        return Err(error);
    }
    Ok(())
}

fn schedule(paths: AppPaths) {
    loop {
        let result = (|| -> Result<()> {
            let store = Store::open(paths.clone())?;
            if let Some(mut job) = store.queued()? {
                job.state = JobState::Processing;
                job.attempts += 1;
                job.updated_at = chrono::Utc::now().to_rfc3339();
                store.put(&job)?;
                if let Err(error) = process_job(&paths, &store, &job) {
                    store.transition(&job.id, JobState::Failed, Some(format!("{error:#}")))?;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Scheduler: {error:#}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn process_job(paths: &AppPaths, store: &Store, job: &JobRecord) -> Result<()> {
    let dir = paths.job_dir(&job.id)?;
    let stderr = fs::File::create(dir.join("worker-error.log"))?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--data-dir")
        .arg(&paths.root)
        .args(["worker", &job.id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(WORKER_TIMEOUT_SECONDS);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                let mut log = String::new();
                fs::File::open(dir.join("worker-error.log"))?
                    .take(65536)
                    .read_to_string(&mut log)?;
                bail!("Render worker failed ({status}): {}", log.trim());
            }
            let receipt: OutputReceipt =
                serde_json::from_slice(&fs::read(dir.join("receipt.json"))?)?;
            store.record_receipt(&job.id, &receipt)?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Rendering exceeded 300 seconds; original input is retained");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn worker(paths: &AppPaths, id: &str) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(paths.job_dir(id)?.join("render.lock"))?;
    lock.lock_exclusive()?;
    let store = Store::open(paths.clone())?;
    let job = store.job(id)?;
    let receipt = output::render_job(paths, &job)?;
    atomic_write(
        &paths.job_dir(id)?.join("receipt.json"),
        &serde_json::to_vec(&receipt)?,
    )?;
    Ok(())
}

fn start_adapter(service: &Service, settings: &Settings) -> Result<()> {
    #[cfg(unix)]
    {
        let mut adapter = service.adapter.lock().unwrap();
        if let Some(mut old) = adapter.take() {
            let _ = old.kill();
            let _ = old.wait();
        }
        let executable = platform::sibling_binary("labelize-printer-ipp")?;
        anyhow::ensure!(
            executable.is_file(),
            "PAPPL adapter not found at {}",
            executable.display()
        );
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(service.paths.root.join("ipp.log"))?;
        let ready_file = service.paths.root.join("ipp-ready");
        if ready_file.exists() {
            fs::remove_file(&ready_file)?;
        }
        let child = Command::new(executable)
            .arg("--port")
            .arg(settings.ipp_port.to_string())
            .arg("--data-dir")
            .arg(&service.paths.root)
            .arg("--daemon")
            .arg(std::env::current_exe()?)
            .arg("--ready-file")
            .arg(&ready_file)
            .arg("--parent-pid")
            .arg(std::process::id().to_string())
            .arg("--width-mm")
            .arg(settings.width_mm.to_string())
            .arg("--height-mm")
            .arg(settings.height_mm.to_string())
            .arg("--dpmm")
            .arg(settings.dpmm.to_string())
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()?;
        *adapter = Some(child);
        for _ in 0..50 {
            if adapter.as_mut().unwrap().try_wait()?.is_some() {
                bail!(
                    "PAPPL exited; see {}",
                    service.paths.root.join("ipp.log").display()
                );
            }
            if fs::read_to_string(&ready_file)
                .is_ok_and(|s| s.trim() == adapter.as_ref().unwrap().id().to_string())
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        bail!("PAPPL did not become ready");
    }
    #[cfg(windows)]
    {
        let _ = (service, settings);
        Ok(())
    }
}
