use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PAGES: usize = 1000;
pub const MAX_PAGE_PIXELS: u64 = 16 * 1024 * 1024;
pub const RAW_IDLE_TIMEOUT_SECONDS: u64 = 30;
pub const WORKER_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Pdf,
    Png,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub output_dir: PathBuf,
    pub format: OutputFormat,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpmm: i32,
    pub raw_port: u16,
    pub ipp_port: u16,
    pub autostart: bool,
    pub setup_complete: bool,
    pub windows_driver: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let output_dir = directories::UserDirs::new()
            .and_then(|d| d.document_dir().map(|p| p.to_path_buf()))
            .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Labelize");
        Self {
            output_dir,
            format: OutputFormat::Pdf,
            width_mm: 102.0,
            height_mm: 152.0,
            dpmm: 8,
            raw_port: 9100,
            ipp_port: 8631,
            autostart: true,
            setup_complete: false,
            windows_driver: None,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !self.output_dir.is_absolute() {
            bail!("Output directory must be an absolute path");
        }
        if ![8, 12].contains(&self.dpmm) {
            bail!("Resolution must be 8 or 12 dots/mm");
        }
        validate_media(self.width_mm, self.height_mm)?;
        validate_pixels(
            (self.width_mm * self.dpmm as f64).ceil() as u32,
            (self.height_mm * self.dpmm as f64).ceil() as u32,
        )?;
        if self.raw_port < 1024 || self.ipp_port < 1024 || self.raw_port == self.ipp_port {
            bail!("Choose distinct RAW and IPP ports between 1024 and 65535");
        }
        Ok(())
    }
}

pub fn validate_media(width: f64, height: f64) -> Result<()> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > 4000.0
        || height > 4000.0
    {
        bail!("Invalid physical page dimensions");
    }
    Ok(())
}

pub fn validate_pixels(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > 32000
        || height > 32000
        || u64::from(width) * u64::from(height) > MAX_PAGE_PIXELS
    {
        bail!("Page exceeds the 32000-dot / 16-Mipixel canvas limit");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPage {
    pub file: PathBuf,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi_x: f64,
    pub dpi_y: f64,
}

/// PAPPL supplies final physical page order. Copies have already been expanded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterManifest {
    #[serde(default)]
    pub job_id: Option<String>,
    pub title: String,
    pub source_job_id: String,
    #[serde(default)]
    pub error: Option<String>,
    pub pages: Vec<RasterPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrintJobInput {
    Zpl { file: PathBuf },
    RasterPages { pages: Vec<RasterPage> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Receiving,
    Queued,
    Processing,
    Saved,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub state: JobState,
    pub source: String,
    pub input: PrintJobInput,
    pub settings: Settings,
    pub transport_complete: bool,
    pub page_count: usize,
    pub output_files: Vec<PathBuf>,
    pub error: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputReceipt {
    pub job_id: String,
    pub page_count: usize,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlatformStatus {
    pub queue_installed: bool,
    pub autostart_installed: bool,
    pub ipp_running: bool,
    pub windows_drivers: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub settings: Settings,
    pub jobs: Vec<JobRecord>,
    pub raw_listening: bool,
    pub platform: PlatformStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    Status,
    RefreshPlatform,
    SaveSettings { settings: Settings },
    SubmitZpl { path: PathBuf, title: String },
    SubmitRaster { manifest: PathBuf },
    BeginRaster { title: String, source: String },
    AbortRaster { id: String, error: String },
    Job { id: String },
    Retry { id: String },
    TestPrint,
    Install,
    Uninstall,
    Shutdown,
    Watch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedRequest {
    pub token: String,
    pub request: Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn success(value: impl Serialize) -> Self {
        Self {
            ok: true,
            data: Some(serde_json::to_value(value).expect("serializable response")),
            error: None,
        }
    }
    pub fn error(error: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }
    }
    pub fn decode<T: serde::de::DeserializeOwned>(self) -> Result<T> {
        if !self.ok {
            bail!(
                "{}",
                self.error.unwrap_or_else(|| "Unknown daemon error".into())
            );
        }
        Ok(serde_json::from_value(
            self.data.unwrap_or(serde_json::Value::Null),
        )?)
    }
}
