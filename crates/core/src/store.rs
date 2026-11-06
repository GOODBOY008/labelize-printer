use crate::{paths::private_dir, *};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::Path, time::Duration};

pub struct Store {
    connection: Connection,
    pub paths: AppPaths,
}

impl Store {
    pub fn open(paths: AppPaths) -> Result<Self> {
        let connection = Connection::open(paths.database())?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(
            version <= 1,
            "Database was created by a newer application version"
        );
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, state TEXT NOT NULL, created TEXT NOT NULL, body TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS jobs_state ON jobs(state,created);
            PRAGMA user_version=1; COMMIT;")?;
        }
        Ok(Self { connection, paths })
    }
    pub fn settings(&self) -> Result<Settings> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT body FROM settings WHERE id=1", [], |r| r.get(0))
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .unwrap_or_else(|| Ok(Settings::default()))
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        self.connection.execute("INSERT INTO settings(id,body) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",
            [serde_json::to_string(settings)?])?;
        Ok(())
    }
    pub fn put(&self, job: &JobRecord) -> Result<()> {
        let state = serde_json::to_string(&job.state)?;
        self.connection.execute(
            "INSERT INTO jobs(id,state,created,body) VALUES(?1,?2,?3,?4)
            ON CONFLICT(id) DO UPDATE SET state=excluded.state,body=excluded.body",
            params![job.id, state, job.created_at, serde_json::to_string(job)?],
        )?;
        Ok(())
    }
    pub fn job(&self, id: &str) -> Result<JobRecord> {
        let body: String = self
            .connection
            .query_row("SELECT body FROM jobs WHERE id=?1", [id], |r| r.get(0))
            .context("Print job not found")?;
        Ok(serde_json::from_str(&body)?)
    }
    pub fn jobs(&self, limit: usize) -> Result<Vec<JobRecord>> {
        let mut stmt = self
            .connection
            .prepare("SELECT body FROM jobs ORDER BY created DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn queued(&self) -> Result<Option<JobRecord>> {
        let body: Option<String> = self
            .connection
            .query_row(
                "SELECT body FROM jobs WHERE state='\"queued\"' ORDER BY created LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        body.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn has_active_jobs(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE state IN ('\"receiving\"','\"queued\"','\"processing\"'))",
            [], |r| r.get(0),
        )?)
    }
    pub fn create(
        &self,
        title: &str,
        source: &str,
        input: PrintJobInput,
        state: JobState,
    ) -> Result<JobRecord> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        private_dir(&self.paths.job_dir(&id)?)?;
        let settings = self.settings()?;
        settings.validate()?;
        let job = JobRecord {
            id,
            title: title.chars().take(200).collect(),
            source: source.into(),
            created_at: now.clone(),
            updated_at: now,
            state,
            input,
            settings,
            transport_complete: false,
            page_count: 0,
            output_files: Vec::new(),
            error: None,
            attempts: 0,
        };
        self.put(&job)?;
        Ok(job)
    }
    pub fn transition(
        &self,
        id: &str,
        state: JobState,
        error: Option<String>,
    ) -> Result<JobRecord> {
        let mut job = self.job(id)?;
        job.state = state;
        job.error = error;
        job.updated_at = Utc::now().to_rfc3339();
        self.put(&job)?;
        Ok(job)
    }
    pub fn import_zpl(&self, path: &Path, title: &str, source: &str) -> Result<JobRecord> {
        let meta = fs::metadata(path).context("Cannot read ZPL input")?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_INPUT_BYTES {
            bail!("ZPL input must be 1 byte to 64 MiB");
        }
        let mut job = self.create(
            title,
            source,
            PrintJobInput::Zpl {
                file: "input.zpl".into(),
            },
            JobState::Receiving,
        )?;
        let result = (|| -> Result<()> {
            let dest = self.paths.job_dir(&job.id)?.join("input.zpl");
            copy_bounded(path, &dest, MAX_INPUT_BYTES)?;
            job.state = JobState::Queued;
            job.transport_complete = true;
            self.put(&job)
        })();
        if let Err(e) = result {
            self.transition(
                &job.id,
                JobState::Failed,
                Some(format!("Input import failed: {e:#}")),
            )?;
            return Err(e);
        }
        Ok(job)
    }
    pub fn import_raster(&self, manifest_path: &Path) -> Result<JobRecord> {
        let data = fs::read(manifest_path)?;
        if data.len() > 1024 * 1024 {
            bail!("Raster manifest is too large");
        }
        let mut manifest: RasterManifest = serde_json::from_slice(&data)?;
        if (manifest.pages.is_empty() && manifest.error.is_none())
            || manifest.pages.len() > MAX_PAGES
        {
            bail!("Raster job must contain 1 to 1000 pages");
        }
        let parent = fs::canonicalize(
            manifest_path
                .parent()
                .context("Invalid manifest location")?,
        )?;
        // Only the local adapter's private incoming directory may supply raster paths.
        let incoming = fs::canonicalize(self.paths.root.join("incoming"))?;
        if !parent.starts_with(&incoming) {
            bail!("Raster manifest must be inside the private incoming directory");
        }
        let source = format!("ipp:{}", manifest.source_job_id);
        let mut job = if let Some(id) = &manifest.job_id {
            let job = self.job(id)?;
            anyhow::ensure!(
                job.source == source,
                "Raster receipt belongs to another task"
            );
            if job.state != JobState::Receiving {
                return Ok(job);
            }
            job
        } else {
            self.create(
                &manifest.title,
                &source,
                PrintJobInput::RasterPages { pages: Vec::new() },
                JobState::Receiving,
            )?
        };
        if let Some(error) = manifest.error {
            return self.transition(
                &job.id,
                JobState::Failed,
                Some(format!("{error}; retained input: {}", parent.display())),
            );
        }
        let result = (|| -> Result<()> {
            let mut total = 0u64;
            for (i, page) in manifest.pages.iter_mut().enumerate() {
                validate_media(page.width_mm, page.height_mm)?;
                if !page.dpi_x.is_finite()
                    || !page.dpi_y.is_finite()
                    || page.dpi_x <= 0.0
                    || page.dpi_y <= 0.0
                {
                    bail!("Invalid raster resolution");
                }
                let src = fs::canonicalize(parent.join(&page.file))?;
                if !src.starts_with(&parent) {
                    bail!("Raster page escapes the job directory");
                }
                total = total
                    .checked_add(fs::metadata(&src)?.len())
                    .context("Raster input size overflow")?;
                if total > MAX_INPUT_BYTES {
                    bail!("Raster job exceeds 64 MiB");
                }
                let local = format!("page-{:04}.pgm", i + 1);
                copy_bounded(
                    &src,
                    &self.paths.job_dir(&job.id)?.join(&local),
                    MAX_INPUT_BYTES,
                )?;
                page.file = local.into();
            }
            job.input = PrintJobInput::RasterPages {
                pages: manifest.pages,
            };
            job.state = JobState::Queued;
            job.transport_complete = true;
            self.put(&job)
        })();
        if let Err(e) = result {
            self.transition(
                &job.id,
                JobState::Failed,
                Some(format!("Raster import failed: {e:#}")),
            )?;
            return Err(e);
        }
        Ok(job)
    }
    pub fn retry(&self, id: &str) -> Result<JobRecord> {
        let job = self.job(id)?;
        if job.state != JobState::Failed {
            bail!("Only failed jobs can be retried");
        }
        if !job.transport_complete {
            bail!("Incomplete transport: resubmit the original print job");
        }
        self.transition(id, JobState::Queued, None)
    }
    pub fn recover(&self) -> Result<()> {
        let mut stmt = self
            .connection
            .prepare("SELECT body FROM jobs WHERE state IN ('\"receiving\"','\"processing\"')")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            let job: JobRecord = serde_json::from_str(&row?)?;
            if job.state == JobState::Receiving {
                self.transition(
                    &job.id,
                    JobState::Failed,
                    Some("Reception interrupted; resubmit the original print job".into()),
                )?;
            } else {
                self.transition(&job.id, JobState::Queued, None)?;
            }
        }
        Ok(())
    }
    pub fn record_receipt(&self, id: &str, receipt: &OutputReceipt) -> Result<JobRecord> {
        anyhow::ensure!(
            receipt.job_id == id && receipt.page_count > 0,
            "Invalid worker receipt"
        );
        let mut job = self.job(id)?;
        job.state = JobState::Saved;
        job.page_count = receipt.page_count;
        job.output_files = receipt.files.clone();
        job.updated_at = Utc::now().to_rfc3339();
        job.error = None;
        self.put(&job)?;
        Ok(job)
    }
}

fn copy_bounded(src: &Path, dest: &Path, maximum: u64) -> Result<()> {
    use std::io::{Read, Write};
    let mut input = fs::File::open(src)?.take(maximum + 1);
    let tmp = dest.with_extension("receiving");
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let copied = std::io::copy(&mut input, &mut output)?;
    anyhow::ensure!(
        copied > 0 && copied <= maximum,
        "Input exceeds size limit or is empty"
    );
    output.flush()?;
    output.sync_all()?;
    drop(output);
    fs::rename(tmp, dest)?;
    Ok(())
}
