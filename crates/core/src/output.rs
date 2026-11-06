use crate::{paths::atomic_write, *};
use anyhow::{bail, Context, Result};
use flate2::{write::ZlibEncoder, Compression};
use image::GrayImage;
use labelize::{DrawerOptions, OutputOptions, ZplParseOptions, ZplParser};
use lopdf::{dictionary, Document, Object, Stream};
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub struct RenderedPage {
    pub pixels: GrayImage,
    pub width_mm: f64,
    pub height_mm: f64,
    pub dpi_x: f64,
    pub dpi_y: f64,
}

/// The only Labelize call site. Raster jobs never enter this branch.
pub fn render_job(paths: &AppPaths, job: &JobRecord) -> Result<OutputReceipt> {
    job.settings.validate()?;
    anyhow::ensure!(
        job.transport_complete,
        "Incomplete transport cannot be rendered"
    );
    let stamp = chrono::DateTime::parse_from_rfc3339(&job.created_at)?
        .format("%Y%m%d-%H%M%S")
        .to_string();
    let base = format!("{stamp}_{}", job.id);
    fs::create_dir_all(&job.settings.output_dir).context("Cannot create output directory")?;
    let final_dir = job.settings.output_dir.join(&base);
    // A complete receipt lets recovery finish the DB transaction after an output commit.
    if final_dir.exists() {
        let receipt: OutputReceipt = serde_json::from_slice(
            &fs::read(final_dir.join(".labelize-job.json"))
                .context("Output directory already exists without a valid receipt")?,
        )?;
        anyhow::ensure!(
            receipt.job_id == job.id && receipt.page_count > 0 && receipt.page_count <= MAX_PAGES,
            "Existing output belongs to another or invalid job"
        );
        anyhow::ensure!(
            receipt.files.len()
                == if job.settings.format == OutputFormat::Pdf {
                    1
                } else {
                    receipt.page_count
                },
            "Existing output receipt has an invalid file count"
        );
        anyhow::ensure!(
            receipt
                .files
                .iter()
                .all(|p| p.parent() == Some(final_dir.as_path()) && p.is_file()),
            "Previously committed output files are missing"
        );
        return Ok(receipt);
    }
    let temp_dir = job.settings.output_dir.join(format!(".{base}.partial"));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)?;
    }
    fs::create_dir(&temp_dir)?;
    let result = (|| -> Result<OutputReceipt> {
        let mut sink = PageSink::new(
            job.settings.format,
            temp_dir.clone(),
            final_dir.clone(),
            &base,
        );
        match &job.input {
            PrintJobInput::Zpl { file } => {
                let file = job_file(paths, &job.id, file)?;
                anyhow::ensure!(
                    fs::metadata(&file)?.len() <= MAX_INPUT_BYTES,
                    "ZPL exceeds 64 MiB"
                );
                let bytes = fs::read(file)?;
                let labels = ZplParser::with_dpmm(job.settings.dpmm)
                    .parse_with_options(
                        &bytes,
                        ZplParseOptions {
                            use_declared_dimensions: true,
                            max_labels: Some(MAX_PAGES),
                            require_complete_formats: true,
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                if labels.is_empty() {
                    bail!("The ZPL job did not contain any printable labels");
                }
                for label in labels {
                    let width_mm = label.label_width_mm.unwrap_or(job.settings.width_mm);
                    let height_mm = label.label_height_mm.unwrap_or(job.settings.height_mm);
                    let drawer = DrawerOptions {
                        label_width_mm: width_mm,
                        label_height_mm: height_mm,
                        dpmm: job.settings.dpmm,
                        antialias: false,
                        ..Default::default()
                    };
                    let png = labelize::render_label_with_output(
                        &label,
                        drawer,
                        OutputOptions::default(),
                        false,
                    )?;
                    let pixels =
                        image::load_from_memory_with_format(&png, image::ImageFormat::Png)?
                            .into_luma8();
                    sink.push(RenderedPage {
                        pixels,
                        width_mm,
                        height_mm,
                        dpi_x: job.settings.dpmm as f64 * 25.4,
                        dpi_y: job.settings.dpmm as f64 * 25.4,
                    })?;
                }
            }
            PrintJobInput::RasterPages { pages } => {
                anyhow::ensure!(
                    !pages.is_empty() && pages.len() <= MAX_PAGES,
                    "Invalid raster page count"
                );
                for page in pages {
                    let file = job_file(paths, &job.id, &page.file)?;
                    let (w, h) = image::ImageReader::open(&file)?
                        .with_guessed_format()?
                        .into_dimensions()?;
                    validate_pixels(w, h)?;
                    let mut reader = image::ImageReader::open(&file)?.with_guessed_format()?;
                    let mut limits = image::Limits::default();
                    limits.max_image_width = Some(32000);
                    limits.max_image_height = Some(32000);
                    limits.max_alloc = Some(MAX_PAGE_PIXELS * 4);
                    reader.limits(limits);
                    let pixels = reader.decode()?.into_luma8();
                    sink.push(RenderedPage {
                        pixels,
                        width_mm: page.width_mm,
                        height_mm: page.height_mm,
                        dpi_x: page.dpi_x,
                        dpi_y: page.dpi_y,
                    })?;
                }
            }
        }
        let receipt = sink.finish(&job.id)?;
        atomic_write(
            &temp_dir.join(".labelize-job.json"),
            &serde_json::to_vec_pretty(&receipt)?,
        )?;
        fs::rename(&temp_dir, &final_dir).context("Cannot commit output files")?;
        #[cfg(unix)]
        fs::File::open(&job.settings.output_dir)?.sync_all()?;
        Ok(receipt)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    result
}

fn job_file(paths: &AppPaths, id: &str, path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        path.components().count() == 1 && path.file_name().is_some() && !path.is_absolute(),
        "Invalid job file path"
    );
    Ok(paths.job_dir(id)?.join(path))
}

struct PageSink {
    format: OutputFormat,
    temp_dir: PathBuf,
    final_dir: PathBuf,
    base: String,
    document: Document,
    pages_id: (u32, u16),
    kids: Vec<Object>,
    files: Vec<PathBuf>,
    count: usize,
}

impl PageSink {
    fn new(format: OutputFormat, temp_dir: PathBuf, final_dir: PathBuf, base: &str) -> Self {
        let mut document = Document::with_version("1.4");
        let pages_id = document.new_object_id();
        Self {
            format,
            temp_dir,
            final_dir,
            base: base.into(),
            document,
            pages_id,
            kids: Vec::new(),
            files: Vec::new(),
            count: 0,
        }
    }
    fn push(&mut self, page: RenderedPage) -> Result<()> {
        validate_pixels(page.pixels.width(), page.pixels.height())?;
        validate_media(page.width_mm, page.height_mm)?;
        anyhow::ensure!(
            page.dpi_x.is_finite()
                && page.dpi_y.is_finite()
                && page.dpi_x > 0.0
                && page.dpi_y > 0.0,
            "Invalid page resolution"
        );
        self.count += 1;
        anyhow::ensure!(self.count <= MAX_PAGES, "Page count limit exceeded");
        if self.format == OutputFormat::Png {
            let name = format!("{}-{:04}.png", self.base, self.count);
            let file = fs::File::create(self.temp_dir.join(&name))?;
            let mut encoder = png::Encoder::new(
                BufWriter::new(&file),
                page.pixels.width(),
                page.pixels.height(),
            );
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_pixel_dims(Some(png::PixelDimensions {
                xppu: (page.dpi_x / 0.0254).round() as u32,
                yppu: (page.dpi_y / 0.0254).round() as u32,
                unit: png::Unit::Meter,
            }));
            let mut writer = encoder.write_header()?;
            writer.write_image_data(page.pixels.as_raw())?;
            writer.finish()?;
            file.sync_all()?;
            self.files.push(self.final_dir.join(name));
        } else {
            let width_pt = page.width_mm * 72.0 / 25.4;
            let height_pt = page.height_mm * 72.0 / 25.4;
            let mut compressor = ZlibEncoder::new(Vec::new(), Compression::default());
            compressor.write_all(page.pixels.as_raw())?;
            let image_id = self.document.add_object(Stream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => page.pixels.width() as i64,
                "Height" => page.pixels.height() as i64, "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8, "Filter" => "FlateDecode",
            }, compressor.finish()?));
            let content = format!("q {width_pt} 0 0 {height_pt} 0 0 cm /PageImage Do Q");
            let content_id = self
                .document
                .add_object(Stream::new(dictionary! {}, content.into_bytes()));
            let page_id = self.document.add_object(dictionary! {
                "Type" => "Page", "Parent" => self.pages_id,
                "MediaBox" => vec![0.into(), 0.into(), Object::Real(width_pt as f32), Object::Real(height_pt as f32)],
                "Contents" => content_id,
                "Resources" => dictionary! { "XObject" => dictionary! { "PageImage" => image_id } },
            });
            self.kids.push(page_id.into());
        }
        Ok(())
    }
    fn finish(mut self, id: &str) -> Result<OutputReceipt> {
        anyhow::ensure!(self.count > 0, "Cannot save an empty print job");
        if self.format == OutputFormat::Pdf {
            self.document.objects.insert(
                self.pages_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Pages", "Kids" => self.kids, "Count" => self.count as i64,
                }),
            );
            let catalog = self
                .document
                .add_object(dictionary! { "Type" => "Catalog", "Pages" => self.pages_id });
            self.document.trailer.set("Root", catalog);
            let name = format!("{}.pdf", self.base);
            let mut file = fs::File::create(self.temp_dir.join(&name))?;
            self.document.save_to(&mut file)?;
            file.sync_all()?;
            self.files.push(self.final_dir.join(name));
        }
        Ok(OutputReceipt {
            job_id: id.into(),
            page_count: self.count,
            files: self.files,
        })
    }
}
