//! Portable visual documents, independent of any printer command language.
use crate::{validate_media, validate_pixels};
use anyhow::{ensure, Context, Result};
use image::{GrayImage, Luma};
use labelize::elements::{
    graphic_field::{GraphicField, GraphicFieldFormat, GraphicFieldMode},
    label_element::LabelElement,
    label_position::LabelPosition,
};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

mod render;
mod scene;
mod symbols;

pub const TEMPLATE_VERSION: u32 = 2;

#[derive(Clone, Copy)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}
#[derive(Clone, Copy)]
pub enum LayerOrder {
    Back,
    Lower,
    Raise,
    Front,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Text,
    Code128,
    Code39,
    Ean13,
    Ean8,
    Upca,
    Itf,
    Qr,
    DataMatrix,
    Pdf417,
    Aztec,
    Rectangle,
    Ellipse,
    Line,
    Image,
}

impl ObjectKind {
    pub const BARCODES: [Self; 10] = [
        Self::Code128,
        Self::Code39,
        Self::Ean13,
        Self::Ean8,
        Self::Upca,
        Self::Itf,
        Self::Qr,
        Self::DataMatrix,
        Self::Pdf417,
        Self::Aztec,
    ];
    pub fn is_barcode(self) -> bool {
        Self::BARCODES.contains(&self)
    }
    pub fn is_matrix(self) -> bool {
        matches!(
            self,
            Self::Qr | Self::DataMatrix | Self::Pdf417 | Self::Aztec
        )
    }
    pub fn is_shape(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse | Self::Line)
    }
    pub fn has_content(self) -> bool {
        self == Self::Text || self.is_barcode()
    }
    pub fn minimum_quiet(self) -> u16 {
        match self {
            Self::Qr => 4,
            Self::DataMatrix => 1,
            Self::Pdf417 | Self::Aztec => 2,
            Self::Ean13 => 11,
            Self::Ean8 => 7,
            Self::Upca => 9,
            _ => 10,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAlign {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFont {
    #[default]
    Document,
    Monospace,
    BoldMonospace,
    CondensedBold,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    #[default]
    Transparent,
    White,
    Black,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Stretch,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineDirection {
    #[default]
    Horizontal,
    Vertical,
    Down,
    Up,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QrEcc {
    L,
    #[default]
    M,
    Q,
    H,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixShape {
    #[default]
    Square,
    Rectangle,
}

/// Version 1 templates omit this block; defaults preserve their appearance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObjectStyle {
    pub name: String,
    /// Clockwise quarter turns; width/height describe the unrotated object.
    pub rotation: u16,
    pub visible: bool,
    pub locked: bool,
    pub background: Background,
    pub horizontal: HorizontalAlign,
    pub vertical: VerticalAlign,
    pub font: TextFont,
    pub wrap: bool,
    pub line_spacing: f64,
    pub filled: bool,
    pub corner_radius_mm: f64,
    pub line_direction: LineDirection,
    pub image_fit: ImageFit,
    pub image_threshold: u8,
    /// Zero selects the largest integer-dot module that fits the object.
    pub module_mm: f64,
    /// Zero selects the recommended minimum for the symbology.
    pub quiet_modules: u16,
    pub human_readable: bool,
    pub qr_ecc: QrEcc,
    pub matrix_shape: MatrixShape,
    pub pdf417_columns: u8,
    pub pdf417_security: u8,
}
impl Default for ObjectStyle {
    fn default() -> Self {
        Self {
            name: String::new(),
            rotation: 0,
            visible: true,
            locked: false,
            background: Background::Transparent,
            horizontal: HorizontalAlign::Left,
            vertical: VerticalAlign::Top,
            font: TextFont::Document,
            wrap: false,
            line_spacing: 1.,
            filled: false,
            corner_radius_mm: 0.,
            line_direction: LineDirection::Horizontal,
            image_fit: ImageFit::Contain,
            image_threshold: 128,
            module_mm: 0.,
            quiet_modules: 0,
            human_readable: false,
            qr_ecc: QrEcc::M,
            matrix_shape: MatrixShape::Square,
            pdf417_columns: 0,
            pdf417_security: 2,
        }
    }
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
    #[serde(default)]
    pub style: ObjectStyle,
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
            width: if kind.is_matrix() && kind != ObjectKind::Pdf417 || kind == ObjectKind::Ellipse
            {
                25.
            } else {
                50.
            },
            height: match kind {
                ObjectKind::Text => 10.,
                ObjectKind::Line => 2.,
                _ => 25.,
            },
            text: match kind {
                ObjectKind::Code128 | ObjectKind::Itf => "1234567890",
                ObjectKind::Code39 => "LABEL-123",
                ObjectKind::Ean13 => "5901234123457",
                ObjectKind::Ean8 => "96385074",
                ObjectKind::Upca => "012345678905",
                ObjectKind::Qr => "https://labelize.dev",
                _ => "Labelize",
            }
            .into(),
            font_mm: 4.,
            stroke_mm: 0.5,
            style: ObjectStyle {
                background: if kind.is_barcode() {
                    Background::White
                } else {
                    Background::Transparent
                },
                ..Default::default()
            },
            image: Vec::new(),
        }
    }
    pub fn bounds_size(&self) -> (f64, f64) {
        if matches!(self.style.rotation, 90 | 270) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
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
            version: TEMPLATE_VERSION,
            width_mm: 102.,
            height_mm: 152.,
            objects: vec![DesignObject::new(ObjectKind::Text)],
            font: Vec::new(),
        }
    }
}

impl DesignDocument {
    pub fn align(&mut self, index: usize, alignment: Alignment) -> Result<()> {
        let o = self.objects.get_mut(index).context("Select an object")?;
        ensure!(!o.style.locked, "Unlock this object before changing it");
        let (w, h) = o.bounds_size();
        match alignment {
            Alignment::Left => o.x = 0.,
            Alignment::Center => o.x = (self.width_mm - w) / 2.,
            Alignment::Right => o.x = self.width_mm - w,
            Alignment::Top => o.y = 0.,
            Alignment::Middle => o.y = (self.height_mm - h) / 2.,
            Alignment::Bottom => o.y = self.height_mm - h,
        }
        Ok(())
    }
    pub fn reorder(&mut self, index: usize, order: LayerOrder) -> Result<usize> {
        let object = self.objects.get(index).context("Select an object")?;
        ensure!(
            !object.style.locked,
            "Unlock this object before changing its layer"
        );
        let target = match order {
            LayerOrder::Back => 0,
            LayerOrder::Lower => index.saturating_sub(1),
            LayerOrder::Raise => (index + 1).min(self.objects.len() - 1),
            LayerOrder::Front => self.objects.len() - 1,
        };
        let object = self.objects.remove(index);
        self.objects.insert(target, object);
        Ok(target)
    }
    pub fn rotate(&mut self, index: usize, rotation: u16) -> Result<()> {
        ensure!([0, 90, 180, 270].contains(&rotation), "Invalid rotation");
        let o = self.objects.get_mut(index).context("Select an object")?;
        ensure!(!o.style.locked, "Unlock this object before changing it");
        let (w, h) = if matches!(rotation, 90 | 270) {
            (o.height, o.width)
        } else {
            (o.width, o.height)
        };
        ensure!(
            w <= self.width_mm && h <= self.height_mm,
            "Rotated object does not fit the label"
        );
        o.style.rotation = rotation;
        o.x = o.x.min(self.width_mm - w);
        o.y = o.y.min(self.height_mm - h);
        Ok(())
    }
    pub fn validate(&self, dpmm: i32) -> Result<()> {
        ensure!(
            [1, TEMPLATE_VERSION].contains(&self.version),
            "Unsupported template version"
        );
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
            let (width, height) = o.bounds_size();
            ensure!(
                o.x.is_finite() && o.y.is_finite() && o.x >= 0. && o.y >= 0.,
                "Object position must be finite and nonnegative"
            );
            validate_media(o.width, o.height)?;
            ensure!(
                o.x + width <= self.width_mm + 0.00001 && o.y + height <= self.height_mm + 0.00001,
                "Object is outside the label"
            );
            ensure!(
                o.font_mm.is_finite() && o.font_mm > 0. && o.font_mm <= 100.,
                "Font size must be between 0 and 100 mm"
            );
            ensure!(
                !o.kind.is_shape()
                    || (o.stroke_mm.is_finite()
                        && o.stroke_mm > 0.
                        && o.stroke_mm <= o.width.min(o.height)),
                "Invalid border thickness"
            );
            let s = &o.style;
            ensure!(
                [0, 90, 180, 270].contains(&s.rotation),
                "Rotation must be 0, 90, 180 or 270 degrees"
            );
            ensure!(s.name.len() <= 256, "Object name exceeds 256 bytes");
            ensure!(
                s.line_spacing.is_finite() && (0.5..=5.).contains(&s.line_spacing),
                "Line spacing must be between 0.5 and 5"
            );
            ensure!(
                s.corner_radius_mm.is_finite()
                    && s.corner_radius_mm >= 0.
                    && s.corner_radius_mm <= o.width.min(o.height) / 2.,
                "Corner radius must fit the object"
            );
            ensure!(
                s.module_mm.is_finite() && (0.0..=10.).contains(&s.module_mm),
                "Module width must be 0 (automatic) to 10 mm"
            );
            ensure!(
                s.quiet_modules <= 100
                    && (!o.kind.is_barcode()
                        || s.quiet_modules == 0
                        || s.quiet_modules >= o.kind.minimum_quiet()),
                "Quiet zone is below the barcode minimum or exceeds 100 modules"
            );
            ensure!(
                s.pdf417_columns <= 30 && s.pdf417_security <= 8,
                "PDF417 requires 0–30 columns and error correction 0–8"
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
        let pixels = render::document(self, dpmm)?;
        let mut label = labelize::LabelInfo {
            label_width_mm: Some(self.width_mm),
            label_height_mm: Some(self.height_mm),
            ..Default::default()
        };
        label
            .elements
            .push(LabelElement::GraphicField(pack(&pixels, 0, 0)));
        Ok(label)
    }

    pub fn render_pixels(
        &self,
        dpmm: i32,
        mode: crate::RenderMode,
        scale: u32,
        antialias: bool,
    ) -> Result<GrayImage> {
        self.validate(dpmm)?;
        if mode == crate::RenderMode::Vector {
            scene::Scene::new(self, dpmm)?.raster(scale, antialias)
        } else {
            ensure!(scale == 1, "Raster output does not support PNG scaling");
            let mut pixels = render::document(self, dpmm)?;
            let size = (
                render::canvas_extent(self.width_mm, dpmm),
                render::canvas_extent(self.height_mm, dpmm),
            );
            if pixels.dimensions() != size {
                let mut canvas = GrayImage::from_pixel(size.0, size.1, Luma([255]));
                image::imageops::overlay(&mut canvas, &pixels, 0, 0);
                pixels = canvas;
            }
            if !antialias {
                for p in pixels.pixels_mut() {
                    p[0] = if p[0] < 128 { 0 } else { 255 };
                }
            }
            Ok(pixels)
        }
    }

    pub fn vector_pdf(&self, dpmm: i32) -> Result<Vec<u8>> {
        scene::Scene::new(self, dpmm)?.pdf()
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
