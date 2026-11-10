//! Shared file/preview/output pipeline. All printer languages use Labelize;
//! visual designs retain their own native text/shape scene in vector mode.
use crate::{design::DesignDocument, output::RenderedPage, *};
use anyhow::{ensure, Context, Result};
use labelize::{DrawerOptions, LabelInfo, OutputOptions, ZplParseOptions, ZplParser};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

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
    pub fn detect(path: &Path, bytes: &[u8]) -> Self {
        if path.file_name().is_some_and(|s| {
            s.to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".labelize.json")
        }) {
            return Self::Design;
        }
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
            && serde_json::from_slice::<DesignDocument>(bytes).is_ok()
        {
            return Self::Design;
        }
        match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "zpl" => Self::Zpl,
            "epl" => Self::Epl,
            "tspl" | "tsc" => Self::Tspl,
            "cpcl" => Self::Cpcl,
            "dpl" => Self::Dpl,
            _ => Self::from_engine(labelize::detect_language(bytes)),
        }
    }
    fn from_engine(language: labelize::Language) -> Self {
        match language {
            labelize::Language::Zpl => Self::Zpl,
            labelize::Language::Epl => Self::Epl,
            labelize::Language::Tspl => Self::Tspl,
            labelize::Language::Cpcl => Self::Cpcl,
            labelize::Language::Dpl => Self::Dpl,
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

pub fn parse(bytes: &[u8], kind: DocumentKind, settings: &Settings) -> Result<Vec<LabelInfo>> {
    settings.validate()?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_INPUT_BYTES,
        "Input must be 1 byte to 64 MiB"
    );
    let labels = match kind {
        DocumentKind::Design => {
            vec![serde_json::from_slice::<DesignDocument>(bytes)?.to_label(settings.dpmm)?]
        }
        DocumentKind::Zpl => ZplParser::with_dpmm(settings.dpmm)
            .parse_with_options(
                bytes,
                ZplParseOptions {
                    use_declared_dimensions: true,
                    max_labels: Some(MAX_PAGES),
                    require_complete_formats: true,
                },
            )
            .map_err(anyhow::Error::msg)?,
        other => labelize::parse_labels(
            labelize::Language::from_name(other.name()).unwrap(),
            bytes,
            settings.dpmm,
        )?,
    };
    ensure!(
        !labels.is_empty() && labels.len() <= MAX_PAGES,
        "Document must contain 1 to 1000 printable labels"
    );
    Ok(labels)
}

pub fn render(
    bytes: &[u8],
    kind: DocumentKind,
    settings: &Settings,
    mut consume: impl FnMut(RenderedPage) -> Result<()>,
) -> Result<()> {
    settings.validate()?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_INPUT_BYTES,
        "Input must be 1 byte to 64 MiB"
    );
    if kind == DocumentKind::Design
        && (settings.antialias || settings.render_mode == RenderMode::Vector)
    {
        let design: DesignDocument = serde_json::from_slice(bytes)?;
        design.validate(settings.dpmm)?;
        let scale = settings.output_scale();
        validate_pixels(
            (design.width_mm * settings.dpmm as f64).ceil() as u32 * scale,
            (design.height_mm * settings.dpmm as f64).ceil() as u32 * scale,
        )?;
        let pixels = design.render_pixels(
            settings.dpmm,
            settings.render_mode,
            scale,
            settings.antialias,
        )?;
        return consume(RenderedPage {
            pixels,
            width_mm: design.width_mm,
            height_mm: design.height_mm,
            dpi_x: settings.dpmm as f64 * 25.4 * scale as f64,
            dpi_y: settings.dpmm as f64 * 25.4 * scale as f64,
        });
    }
    for label in parse(bytes, kind, settings)? {
        let drawer = drawing_options(&label, settings)?;
        let (width_mm, height_mm) = (drawer.label_width_mm, drawer.label_height_mm);
        let scale = settings.output_scale();
        validate_pixels(
            (width_mm * settings.dpmm as f64).ceil() as u32 * scale,
            (height_mm * settings.dpmm as f64).ceil() as u32 * scale,
        )?;
        let png = labelize::render_label_with_output(
            &label,
            drawer,
            OutputOptions {
                mode: match settings.render_mode {
                    RenderMode::Raster => labelize::OutputMode::Raster,
                    RenderMode::Vector => labelize::OutputMode::Vector,
                },
                scale: scale as f64,
                ..Default::default()
            },
            false,
        )?;
        consume(RenderedPage {
            pixels: image::load_from_memory_with_format(&png, image::ImageFormat::Png)?
                .into_luma8(),
            width_mm,
            height_mm,
            dpi_x: settings.dpmm as f64 * 25.4 * scale as f64,
            dpi_y: settings.dpmm as f64 * 25.4 * scale as f64,
        })?;
    }
    Ok(())
}

fn drawing_options(label: &LabelInfo, settings: &Settings) -> Result<DrawerOptions> {
    let width_mm = label.label_width_mm.unwrap_or(settings.width_mm);
    let height_mm = label.label_height_mm.unwrap_or(settings.height_mm);
    validate_media(width_mm, height_mm)?;
    validate_pixels(
        (width_mm * settings.dpmm as f64).ceil() as u32,
        (height_mm * settings.dpmm as f64).ceil() as u32,
    )?;
    Ok(DrawerOptions {
        label_width_mm: width_mm,
        label_height_mm: height_mm,
        dpmm: settings.dpmm,
        antialias: settings.antialias,
        ..Default::default()
    })
}

/// Keep semantic drawing operations until the final PDF encoding step.
pub fn vector_pdf(
    bytes: &[u8],
    kind: DocumentKind,
    settings: &Settings,
) -> Result<(Vec<u8>, usize)> {
    settings.validate()?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_INPUT_BYTES,
        "Input must be 1 byte to 64 MiB"
    );
    if kind == DocumentKind::Design {
        let design: DesignDocument = serde_json::from_slice(bytes)?;
        return Ok((design.vector_pdf(settings.dpmm)?, 1));
    }
    let labels = parse(bytes, kind, settings)?;
    let scenes = labels
        .iter()
        .map(|label| {
            Ok(labelize::LabelScene::new(
                label,
                drawing_options(label, settings)?,
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((labelize::encode_scenes_pdf(&scenes)?, scenes.len()))
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

/// Invoked only in the standalone renderer process; the GUI never parses input.
pub fn preview(request: &PreviewRequest, directory: &Path) -> Result<Vec<PreviewPage>> {
    fs::create_dir_all(directory)?;
    let mut pages = Vec::new();
    let mut total_pixels = 0u64;
    render(
        &read_input(&request.input)?,
        request.kind,
        &request.settings,
        |page| {
            // Preview is a disk cache, bounded separately from export. Never keep
            // an entire multipage job's decoded pixels in the desktop process.
            total_pixels += u64::from(page.pixels.width()) * u64::from(page.pixels.height());
            ensure!(
                total_pixels <= 128 * 1024 * 1024,
                "Preview exceeds 128 Mi pixels; export the full document instead"
            );
            let file = PathBuf::from(format!("page-{}.png", pages.len() + 1));
            page.pixels.save(directory.join(&file))?;
            pages.push(PreviewPage {
                file,
                width_mm: page.width_mm,
                height_mm: page.height_mm,
                width: page.pixels.width(),
                height: page.pixels.height(),
            });
            Ok(())
        },
    )?;
    crate::paths::atomic_write(&directory.join("pages.json"), &serde_json::to_vec(&pages)?)?;
    Ok(pages)
}
