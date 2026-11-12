//! Shared document types and input handling for jobs and previews.
use crate::{Settings, MAX_INPUT_BYTES};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(feature = "render")]
mod renderer;
#[cfg(feature = "render")]
pub use renderer::{parse, preview, render, vector_pdf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentKind {
    Zpl,
    Epl,
    Tspl,
    Cpcl,
    Dpl,
    Design,
}

impl DocumentKind {
    pub const LANGUAGES: [Self; 5] = [Self::Zpl, Self::Epl, Self::Tspl, Self::Cpcl, Self::Dpl];
    pub fn name(self) -> &'static str {
        match self {
            Self::Zpl => "ZPL",
            Self::Epl => "EPL",
            Self::Tspl => "TSPL",
            Self::Cpcl => "CPCL",
            Self::Dpl => "DPL",
            Self::Design => "Labelize",
        }
    }
}

pub fn read_input(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path).context("Cannot open label file")?;
    ensure!(file.metadata()?.is_file(), "Choose a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_INPUT_BYTES,
        "Input must be 1 byte to 64 MiB"
    );
    Ok(bytes)
}

#[derive(Serialize, Deserialize)]
pub struct PreviewRequest {
    pub input: PathBuf,
    pub kind: DocumentKind,
    pub settings: Settings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreviewPage {
    pub file: PathBuf,
    pub width_mm: f64,
    pub height_mm: f64,
    pub width: u32,
    pub height: u32,
}
