//! Portable visual documents, independent of any printer command language.
use crate::{validate_media, validate_pixels};
use ab_glyph::{Font, FontRef, PxScale};
use anyhow::{ensure, Context, Result};
use image::{GrayImage, Luma};
use labelize::elements::{
    graphic_field::{GraphicField, GraphicFieldFormat, GraphicFieldMode},
    label_element::LabelElement,
    label_position::LabelPosition,
};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Text,
    Code128,
    Qr,
    Rectangle,
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignObject {
    pub kind: ObjectKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub text: String,
    pub font_mm: f64,
    pub stroke_mm: f64,
    /// Normalized PNG pixels travel with the template; no external file references.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "base64_bytes")]
    pub image: Vec<u8>,
}

impl DesignObject {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            kind,
            x: 5.,
            y: 5.,
            width: if kind == ObjectKind::Qr { 25. } else { 50. },
            height: if kind == ObjectKind::Text { 10. } else { 25. },
            text: match kind {
                ObjectKind::Code128 => "1234567890",
                ObjectKind::Qr => "https://labelize.dev",
                _ => "Labelize",
            }
            .into(),
            font_mm: 4.,
            stroke_mm: 0.5,
            image: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignDocument {
    pub version: u32,
    pub width_mm: f64,
    pub height_mm: f64,
    pub objects: Vec<DesignObject>,
    /// Optional user-supplied font embedded for portable CJK/non-Latin text.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "base64_bytes")]
    pub font: Vec<u8>,
}

impl Default for DesignDocument {
    fn default() -> Self {
        Self {
            version: 1,
            width_mm: 102.,
            height_mm: 152.,
            objects: vec![DesignObject::new(ObjectKind::Text)],
            font: Vec::new(),
        }
    }
}

impl DesignDocument {
    pub fn validate(&self, dpmm: i32) -> Result<()> {
        ensure!(self.version == 1, "Unsupported template version");
        ensure!(
            [8, 12].contains(&dpmm),
            "Resolution must be 8 or 12 dots/mm"
        );
        validate_media(self.width_mm, self.height_mm)?;
        validate_pixels(
            (self.width_mm * dpmm as f64).ceil() as u32,
            (self.height_mm * dpmm as f64).ceil() as u32,
        )?;
        ensure!(
            self.objects.len() <= 100,
            "A template can contain at most 100 objects"
        );
        ensure!(self.font.len() <= 32 * 1024 * 1024, "Font exceeds 32 MiB");
        for o in &self.objects {
            ensure!(
                o.x.is_finite() && o.y.is_finite() && o.x >= 0. && o.y >= 0.,
                "Object position must be finite and nonnegative"
            );
            validate_media(o.width, o.height)?;
            ensure!(
                o.x + o.width <= self.width_mm + 0.00001
                    && o.y + o.height <= self.height_mm + 0.00001,
                "Object is outside the label"
            );
            ensure!(
                o.font_mm.is_finite() && o.font_mm > 0. && o.font_mm <= 100.,
                "Font size must be between 0 and 100 mm"
            );
            ensure!(
                o.kind != ObjectKind::Rectangle
                    || (o.stroke_mm.is_finite()
                        && o.stroke_mm > 0.
                        && o.stroke_mm <= o.width.min(o.height)),
                "Invalid border thickness"
            );
            ensure!(
                o.text.len() <= 4096 && o.image.len() <= 8 * 1024 * 1024,
                "Object content is too large"
            );
            validate_pixels(
                (o.width * dpmm as f64).round() as u32,
                (o.height * dpmm as f64).round() as u32,
            )?;
        }
        Ok(())
    }

    pub fn to_label(&self, dpmm: i32) -> Result<labelize::LabelInfo> {
        self.validate(dpmm)?;
        let font = FontRef::try_from_slice(if self.font.is_empty() {
            labelize::assets::FONT_DEJAVU_SANS_MONO
        } else {
            &self.font
        })
        .context("Invalid font; choose a TrueType/OpenType font")?;
        let mut label = labelize::LabelInfo {
            label_width_mm: Some(self.width_mm),
            label_height_mm: Some(self.height_mm),
            ..Default::default()
        };
        for (i, o) in self.objects.iter().enumerate() {
            let pixels =
                render_object(o, &font, dpmm).with_context(|| format!("Object {}", i + 1))?;
            label.elements.push(LabelElement::GraphicField(pack(
                &pixels,
                (o.x * dpmm as f64).round() as i32,
                (o.y * dpmm as f64).round() as i32,
            )));
        }
        Ok(label)
    }
}

pub fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>> {
    let pixels = decode_image(bytes)?;
    let mut output = Cursor::new(Vec::new());
    pixels.write_to(&mut output, image::ImageFormat::Png)?;
    let result = output.into_inner();
    ensure!(result.len() <= 8 * 1024 * 1024, "Image exceeds 8 MiB");
    Ok(result)
}

fn decode_image(bytes: &[u8]) -> Result<GrayImage> {
    ensure!(bytes.len() <= 8 * 1024 * 1024, "Image exceeds 8 MiB");
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32000);
    limits.max_image_height = Some(32000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode()?.into_rgba8();
    validate_pixels(rgba.width(), rgba.height())?;
    Ok(composite_white(&rgba))
}

fn composite_white(rgba: &image::RgbaImage) -> GrayImage {
    GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let luma = (299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000;
        Luma([((luma * u32::from(p[3]) + 255 * (255 - u32::from(p[3]))) / 255) as u8])
    })
}

fn render_object(o: &DesignObject, font: &FontRef<'_>, dpmm: i32) -> Result<GrayImage> {
    let w = (o.width * dpmm as f64).round() as u32;
    let h = (o.height * dpmm as f64).round() as u32;
    let mut canvas = GrayImage::from_pixel(w, h, Luma([255]));
    match o.kind {
        ObjectKind::Text => {
            let scale = PxScale::from((o.font_mm * dpmm as f64) as f32);
            let line_height = scale.y.ceil() as u32;
            for (n, line) in o.text.lines().enumerate() {
                ensure!(
                    line.chars().all(|ch| font.glyph_id(ch).0 != 0),
                    "The font lacks a character in this text. Import a font that includes it."
                );
                let (tw, th) = imageproc::drawing::text_size(scale, font, line);
                ensure!(
                    tw <= w && n as u32 * line_height + th <= h,
                    "Text does not fit; enlarge the object or reduce the font size"
                );
                imageproc::drawing::draw_text_mut(
                    &mut canvas,
                    Luma([0]),
                    0,
                    (n as u32 * line_height) as i32,
                    scale,
                    font,
                    line,
                );
            }
        }
        ObjectKind::Rectangle => {
            let stroke = (o.stroke_mm * dpmm as f64).round().max(1.) as u32;
            for (x, y, p) in canvas.enumerate_pixels_mut() {
                if x < stroke
                    || y < stroke
                    || x >= w.saturating_sub(stroke)
                    || y >= h.saturating_sub(stroke)
                {
                    *p = Luma([0]);
                }
            }
        }
        ObjectKind::Image => {
            let source = decode_image(&o.image)?;
            let scale = (w as f64 / source.width() as f64).min(h as f64 / source.height() as f64);
            let image = image::imageops::resize(
                &source,
                (source.width() as f64 * scale).round().max(1.) as u32,
                (source.height() as f64 * scale).round().max(1.) as u32,
                image::imageops::FilterType::Nearest,
            );
            image::imageops::overlay(&mut canvas, &image, 0, 0);
        }
        ObjectKind::Code128 | ObjectKind::Qr => {
            ensure!(!o.text.is_empty(), "Barcode data is empty");
            let (source, quiet) = if o.kind == ObjectKind::Code128 {
                ensure!(
                    o.text.is_ascii() && o.text.len() <= 512,
                    "Code 128 requires at most 512 ASCII bytes"
                );
                (
                    labelize::barcodes::code128::encode_auto(&o.text, 1, 1)
                        .map_err(anyhow::Error::msg)?,
                    10,
                )
            } else {
                (
                    labelize::barcodes::qrcode::encode(
                        &o.text,
                        1,
                        labelize::elements::barcode_qr::QrErrorCorrectionLevel::M,
                    )
                    .map_err(anyhow::Error::msg)?,
                    0, // Labelize's QR encoder already includes its four-module quiet zone.
                )
            };
            let horizontal = w / (source.width() + quiet * 2);
            let module = if o.kind == ObjectKind::Qr {
                horizontal.min(h / (source.height() + quiet * 2))
            } else {
                horizontal
            };
            ensure!(
                module > 0,
                "Barcode does not fit at one pixel per module; enlarge the object"
            );
            let target_h = if o.kind == ObjectKind::Qr {
                source.height() * module
            } else {
                h
            };
            // Integer module scaling preserves scanability; never stretch a barcode.
            let source = composite_white(&source);
            let scaled = image::imageops::resize(
                &source,
                source.width() * module,
                target_h,
                image::imageops::FilterType::Nearest,
            );
            image::imageops::overlay(
                &mut canvas,
                &scaled,
                (quiet * module).into(),
                if o.kind == ObjectKind::Qr {
                    (quiet * module).into()
                } else {
                    0
                },
            );
        }
    }
    Ok(canvas)
}

fn pack(pixels: &GrayImage, x: i32, y: i32) -> GraphicField {
    let row_bytes = pixels.width().div_ceil(8) as usize;
    let mut data = vec![0; row_bytes * pixels.height() as usize];
    for (px, py, p) in pixels.enumerate_pixels() {
        if p[0] < 128 {
            data[py as usize * row_bytes + px as usize / 8] |= 0x80 >> (px % 8);
        }
    }
    GraphicField {
        reverse_print: Default::default(),
        position: LabelPosition {
            x,
            y,
            ..Default::default()
        },
        format: GraphicFieldFormat::Raw,
        data_bytes: data.len() as i32,
        total_bytes: data.len() as i32,
        row_bytes: row_bytes as i32,
        data,
        magnification_x: 1,
        magnification_y: 1,
        mode: GraphicFieldMode::Or,
        zero_bits_black: false,
    }
}

mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}
