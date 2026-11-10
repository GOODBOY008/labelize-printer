use super::*;
use ab_glyph::FontRef;
use image::imageops::{self, FilterType};
use labelize::{
    barcodes,
    elements::{barcode_datamatrix::DatamatrixRatio, barcode_qr::QrErrorCorrectionLevel},
};

fn digits(text: &str, lengths: &[usize]) -> Result<()> {
    ensure!(
        text.bytes().all(|b| b.is_ascii_digit()) && lengths.contains(&text.len()),
        "Barcode requires only digits, with length {lengths:?}"
    );
    Ok(())
}
fn checked_gtin(text: &str, body_len: usize) -> Result<String> {
    digits(text, &[body_len, body_len + 1])?;
    let body = &text[..body_len];
    let sum: u32 = body
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    let check = b'0' + ((10 - sum % 10) % 10) as u8;
    ensure!(
        text.len() == body_len || text.as_bytes()[body_len] == check,
        "Invalid check digit; expected {}",
        check as char
    );
    Ok(format!("{body}{}", check as char))
}

pub(super) fn encode(o: &DesignObject) -> Result<(GrayImage, String)> {
    let text = &o.text;
    ensure!(!text.is_empty(), "Barcode data is empty");
    let mut caption = text.clone();
    let result = match o.kind {
        ObjectKind::Code128 => {
            ensure!(
                text.is_ascii() && text.len() <= 512,
                "Code 128 requires at most 512 ASCII bytes"
            );
            barcodes::code128::encode_auto(text, 1, 1)
        }
        ObjectKind::Code39 => {
            ensure!(
                text.len() <= 512
                    && text.bytes().all(|b| b.is_ascii_uppercase()
                        || b.is_ascii_digit()
                        || b" -.$/+%".contains(&b)),
                "Code 39 requires uppercase letters, digits, space or -.$/+%"
            );
            barcodes::code39::encode(text, 1, 3., 1)
        }
        ObjectKind::Ean13 => {
            caption = checked_gtin(text, 12)?;
            barcodes::ean13::encode(&caption, 1, 1).map(|s| s.image)
        }
        ObjectKind::Ean8 => {
            caption = checked_gtin(text, 7)?;
            barcodes::ean8::encode(&caption, 1, 1).map(|s| s.image)
        }
        ObjectKind::Upca => {
            caption = checked_gtin(text, 11)?;
            barcodes::upca::encode(&caption, 1, 1).map(|s| s.image)
        }
        ObjectKind::Itf => {
            ensure!(
                text.len() <= 512
                    && text.len().is_multiple_of(2)
                    && text.bytes().all(|b| b.is_ascii_digit()),
                "ITF requires an even number of digits (at most 512)"
            );
            barcodes::twooffive::encode(text, 1, 3., 1, false)
        }
        ObjectKind::Qr => barcodes::qrcode::encode(
            text,
            1,
            match o.style.qr_ecc {
                QrEcc::L => QrErrorCorrectionLevel::L,
                QrEcc::M => QrErrorCorrectionLevel::M,
                QrEcc::Q => QrErrorCorrectionLevel::Q,
                QrEcc::H => QrErrorCorrectionLevel::H,
            },
        ),
        ObjectKind::DataMatrix => barcodes::datamatrix::encode_with_ratio(
            text,
            1,
            0,
            0,
            Some(if o.style.matrix_shape == MatrixShape::Square {
                DatamatrixRatio::Square
            } else {
                DatamatrixRatio::Rectangular
            }),
        ),
        ObjectKind::Pdf417 => {
            ensure!(
                text.is_ascii(),
                "PDF417 currently requires ASCII data; use QR or Data Matrix for UTF-8"
            );
            barcodes::pdf417::encode(
                text,
                3,
                o.style.pdf417_security.into(),
                o.style.pdf417_columns.into(),
                0,
                false,
                1,
            )
        }
        ObjectKind::Aztec => barcodes::aztec::encode(text, 1, 0),
        _ => anyhow::bail!("Not a barcode object"),
    }
    .map_err(anyhow::Error::msg)?;
    let mut pixels = composite_white(&result);
    if o.kind == ObjectKind::Qr {
        // The engine includes four quiet modules. Apply the selected margin once.
        pixels =
            imageops::crop_imm(&pixels, 4, 4, pixels.width() - 8, pixels.height() - 8).to_image();
    } else if !o.kind.is_matrix() {
        // EAN/UPC includes guard extensions; data bars all share the first row.
        pixels = imageops::crop_imm(&pixels, 0, 0, pixels.width(), 1).to_image();
    }
    Ok((pixels, caption))
}

pub(super) fn draw(
    canvas: &mut GrayImage,
    o: &DesignObject,
    font: &FontRef<'_>,
    dpmm: i32,
) -> Result<()> {
    let layout = layout(o, canvas.width(), canvas.height(), dpmm)?;
    let scaled = imageops::resize(
        &layout.source,
        layout.source.width() * layout.module,
        layout.target_h,
        FilterType::Nearest,
    );
    imageops::overlay(canvas, &scaled, layout.x, layout.y);
    if layout.text_height > 0 {
        let mut text = GrayImage::from_pixel(canvas.width(), layout.text_height, Luma([255]));
        let mut text_object = o.clone();
        text_object.style.horizontal = HorizontalAlign::Center;
        text_object.style.vertical = VerticalAlign::Top;
        text_object.style.wrap = false;
        render::draw_text(&mut text, &layout.caption, &text_object, font, dpmm)?;
        imageops::overlay(canvas, &text, 0, layout.text_y as i64);
    }
    Ok(())
}

pub(super) struct SymbolLayout {
    pub source: GrayImage,
    pub caption: String,
    pub module: u32,
    pub target_h: u32,
    pub x: i64,
    pub y: i64,
    pub text_height: u32,
    pub text_y: u32,
}

/// The two backends share module sizing, quiet zones, caption and alignment.
pub(super) fn layout(o: &DesignObject, width: u32, height: u32, dpmm: i32) -> Result<SymbolLayout> {
    let (source, caption) = encode(o)?;
    let quiet = u32::from(if o.style.quiet_modules == 0 {
        o.kind.minimum_quiet()
    } else {
        o.style.quiet_modules
    });
    let hri = o.style.human_readable && !o.kind.is_matrix();
    let text_height = if hri {
        (o.font_mm * dpmm as f64).ceil() as u32
    } else {
        0
    };
    let gap = if hri {
        (0.5 * dpmm as f64).round() as u32
    } else {
        0
    };
    ensure!(
        text_height + gap < height,
        "Barcode caption leaves no room for bars"
    );
    let available_h = height - text_height - gap;
    let total_w = source.width() + quiet * 2;
    let total_h = source.height() + quiet * 2;
    let automatic = if o.kind.is_matrix() {
        (width / total_w).min(available_h / total_h)
    } else {
        width / total_w
    };
    let module = if o.style.module_mm == 0. {
        automatic
    } else {
        (o.style.module_mm * dpmm as f64).round() as u32
    };
    ensure!(module > 0 && module <= automatic, "Barcode does not fit at the selected module size; enlarge the object or use automatic sizing");
    let target_h = if o.kind.is_matrix() {
        source.height() * module
    } else {
        available_h
    };
    let x = render::horizontal((width - total_w * module) as i64, o.style.horizontal)
        + (quiet * module) as i64;
    let y = if o.kind.is_matrix() {
        render::vertical((available_h - total_h * module) as i64, o.style.vertical)
            + (quiet * module) as i64
    } else {
        0
    };
    Ok(SymbolLayout {
        source,
        caption,
        module,
        target_h,
        x,
        y,
        text_height,
        text_y: available_h + gap,
    })
}
