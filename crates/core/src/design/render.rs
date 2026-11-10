use super::*;
use ab_glyph::{Font, FontRef, PxScale};
use image::imageops::{self, FilterType};

// Match Labelize's outer canvas rounding, including floating-point dot values
// that are a few ULPs above an integer. Object layout still snaps to whole dots.
pub(super) fn canvas_extent(mm: f64, dpmm: i32) -> u32 {
    let dots = mm * dpmm as f64;
    if (dots - dots.round()).abs() <= dots.abs() * f64::EPSILON * 4. {
        dots.round() as u32
    } else {
        dots.ceil() as u32
    }
}

pub(super) fn document(design: &DesignDocument, dpmm: i32) -> Result<GrayImage> {
    let mut canvas = GrayImage::from_pixel(
        (design.width_mm * dpmm as f64).round() as u32,
        (design.height_mm * dpmm as f64).round() as u32,
        Luma([255]),
    );
    // Validate an embedded font even if its last text object is hidden.
    font(design, TextFont::Document)?;
    for (i, object) in design
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.style.visible)
    {
        let mut pixels = object_pixels(object, design, dpmm)
            .with_context(|| format!("Object {} ({})", i + 1, object.style.name))?;
        if object.style.background == Background::Black {
            imageops::invert(&mut pixels);
        }
        let pixels = match object.style.rotation {
            90 => imageops::rotate90(&pixels),
            180 => imageops::rotate180(&pixels),
            270 => imageops::rotate270(&pixels),
            _ => pixels,
        };
        let x = (object.x * dpmm as f64).round() as u32;
        let y = (object.y * dpmm as f64).round() as u32;
        // Compose before bit packing: an opaque background must not erase the
        // padding bits at the right edge of another object.
        for (px, py, pixel) in pixels.enumerate_pixels() {
            if x + px < canvas.width() && y + py < canvas.height() {
                let target = canvas.get_pixel_mut(x + px, y + py);
                target[0] = if object.style.background == Background::Transparent {
                    target[0].min(pixel[0])
                } else {
                    pixel[0]
                };
            }
        }
    }
    Ok(canvas)
}

pub(super) fn font(design: &DesignDocument, face: TextFont) -> Result<FontRef<'_>> {
    FontRef::try_from_slice(font_bytes(design, face))
        .context("Invalid font; choose a TrueType/OpenType font")
}

pub(super) fn font_bytes(design: &DesignDocument, face: TextFont) -> &[u8] {
    use labelize::assets::*;
    match face {
        TextFont::Document if !design.font.is_empty() => &design.font,
        TextFont::Document | TextFont::Monospace => FONT_DEJAVU_SANS_MONO,
        TextFont::BoldMonospace => FONT_DEJAVU_SANS_MONO_BOLD,
        TextFont::CondensedBold => FONT_ZERO_SUBSTITUTE,
    }
}

pub(super) fn object_pixels(
    o: &DesignObject,
    design: &DesignDocument,
    dpmm: i32,
) -> Result<GrayImage> {
    let w = (o.width * dpmm as f64).round() as u32;
    let h = (o.height * dpmm as f64).round() as u32;
    let mut canvas = GrayImage::from_pixel(w, h, Luma([255]));
    match o.kind {
        ObjectKind::Text => draw_text(&mut canvas, &o.text, o, &font(design, o.style.font)?, dpmm)?,
        ObjectKind::Rectangle | ObjectKind::Ellipse | ObjectKind::Line => {
            let stroke = (o.stroke_mm * dpmm as f64).round().max(1.);
            let radius = o.style.corner_radius_mm * dpmm as f64;
            for (x, y, p) in canvas.enumerate_pixels_mut() {
                let (x, y, w, h) = (x as f64 + 0.5, y as f64 + 0.5, w as f64, h as f64);
                let inside = match o.kind {
                    ObjectKind::Rectangle => {
                        rounded_rect(x, y, w, h, radius)
                            && (o.style.filled
                                || !rounded_rect(
                                    x - stroke,
                                    y - stroke,
                                    w - 2. * stroke,
                                    h - 2. * stroke,
                                    (radius - stroke).max(0.),
                                ))
                    }
                    ObjectKind::Ellipse => {
                        ellipse(x, y, w, h)
                            && (o.style.filled
                                || !ellipse(
                                    x - stroke,
                                    y - stroke,
                                    w - 2. * stroke,
                                    h - 2. * stroke,
                                ))
                    }
                    _ => {
                        let (start, end) = match o.style.line_direction {
                            LineDirection::Horizontal => ((0., h / 2.), (w, h / 2.)),
                            LineDirection::Vertical => ((w / 2., 0.), (w / 2., h)),
                            LineDirection::Down => (
                                (stroke / 2., stroke / 2.),
                                (w - stroke / 2., h - stroke / 2.),
                            ),
                            LineDirection::Up => (
                                (stroke / 2., h - stroke / 2.),
                                (w - stroke / 2., stroke / 2.),
                            ),
                        };
                        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                        let t = (((x - start.0) * dx + (y - start.1) * dy)
                            / (dx * dx + dy * dy).max(f64::EPSILON))
                        .clamp(0., 1.);
                        (x - start.0 - t * dx).hypot(y - start.1 - t * dy) <= stroke / 2.
                    }
                };
                if inside {
                    *p = Luma([0]);
                }
            }
        }
        ObjectKind::Image => {
            let source = decode_image(&o.image)?;
            let sx = w as f64 / source.width() as f64;
            let sy = h as f64 / source.height() as f64;
            let (nw, nh) = match o.style.image_fit {
                ImageFit::Stretch => (w, h),
                fit => {
                    let scale = if fit == ImageFit::Contain {
                        sx.min(sy)
                    } else {
                        sx.max(sy)
                    };
                    (
                        (source.width() as f64 * scale).round().max(1.) as u32,
                        (source.height() as f64 * scale).round().max(1.) as u32,
                    )
                }
            };
            // Cover can otherwise create a huge intermediate for an extreme aspect ratio.
            validate_pixels(nw, nh)?;
            let mut scaled = imageops::resize(&source, nw, nh, FilterType::Nearest);
            for p in scaled.pixels_mut() {
                p[0] = if p[0] < o.style.image_threshold {
                    0
                } else {
                    255
                };
            }
            imageops::overlay(
                &mut canvas,
                &scaled,
                horizontal(w as i64 - nw as i64, o.style.horizontal),
                vertical(h as i64 - nh as i64, o.style.vertical),
            );
        }
        _ => symbols::draw(&mut canvas, o, &font(design, o.style.font)?, dpmm)?,
    }
    Ok(canvas)
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, radius: f64) -> bool {
    if w <= 0. || h <= 0. || x < 0. || y < 0. || x >= w || y >= h {
        return false;
    }
    let r = radius.min(w.min(h) / 2.);
    let dx = (r - x).max(x - (w - r)).max(0.);
    let dy = (r - y).max(y - (h - r)).max(0.);
    dx * dx + dy * dy <= r * r
}
fn ellipse(x: f64, y: f64, w: f64, h: f64) -> bool {
    w > 0. && h > 0. && ((x - w / 2.) / (w / 2.)).powi(2) + ((y - h / 2.) / (h / 2.)).powi(2) <= 1.
}
pub(super) fn horizontal(space: i64, align: HorizontalAlign) -> i64 {
    match align {
        HorizontalAlign::Left => 0,
        HorizontalAlign::Center => space / 2,
        HorizontalAlign::Right => space,
    }
}
pub(super) fn vertical(space: i64, align: VerticalAlign) -> i64 {
    match align {
        VerticalAlign::Top => 0,
        VerticalAlign::Middle => space / 2,
        VerticalAlign::Bottom => space,
    }
}

pub(super) fn draw_text(
    canvas: &mut GrayImage,
    text: &str,
    o: &DesignObject,
    font: &FontRef<'_>,
    dpmm: i32,
) -> Result<()> {
    for line in text_lines(canvas.width(), canvas.height(), text, o, font, dpmm)? {
        imageproc::drawing::draw_text_mut(
            canvas,
            Luma([0]),
            line.x,
            line.y,
            PxScale::from((o.font_mm * dpmm as f64) as f32),
            font,
            &line.text,
        );
    }
    Ok(())
}

pub(super) struct TextLine {
    pub text: String,
    pub x: i32,
    pub y: i32,
}

/// Shared wrapping and alignment for the legacy raster and vector backends.
pub(super) fn text_lines(
    width: u32,
    height: u32,
    text: &str,
    o: &DesignObject,
    font: &FontRef<'_>,
    dpmm: i32,
) -> Result<Vec<TextLine>> {
    let scale = PxScale::from((o.font_mm * dpmm as f64) as f32);
    ensure!(
        text.chars()
            .filter(|c| !matches!(c, '\n' | '\r'))
            .all(|c| font.glyph_id(c).0 != 0),
        "The font lacks a character in this text. Import a font that includes it."
    );
    let measure = |line: &str| imageproc::drawing::text_size(scale, font, line).0;
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        if !o.style.wrap || paragraph.is_empty() {
            lines.push(paragraph.to_owned());
            continue;
        }
        let mut current = String::new();
        for ch in paragraph.chars() {
            current.push(ch);
            if measure(&current) <= width {
                continue;
            }
            current.pop();
            ensure!(
                !current.is_empty(),
                "A character does not fit; enlarge the object or reduce the font size"
            );
            // Prefer word boundaries, with character wrapping for long words and CJK.
            if let Some((index, _)) = current
                .char_indices()
                .rev()
                .find(|(i, c)| *i > 0 && c.is_whitespace())
            {
                let remainder = current[index..].trim_start().to_owned();
                lines.push(current[..index].to_owned());
                current = remainder;
            } else {
                lines.push(std::mem::take(&mut current));
            }
            if !current.is_empty() || !ch.is_whitespace() {
                current.push(ch);
            }
        }
        lines.push(current);
    }
    let step = (scale.y as f64 * o.style.line_spacing).ceil() as u32;
    let text_height = if lines.is_empty() {
        0
    } else {
        (lines.len() as u32 - 1) * step + scale.y.ceil() as u32
    };
    ensure!(
        text_height <= height && lines.iter().all(|line| measure(line) <= width),
        "Text does not fit; enlarge the object or reduce the font size"
    );
    let top = vertical((height - text_height) as i64, o.style.vertical);
    Ok(lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| TextLine {
            x: horizontal((width - measure(&text)) as i64, o.style.horizontal) as i32,
            y: (top + i as i64 * step as i64) as i32,
            text,
        })
        .collect())
}
