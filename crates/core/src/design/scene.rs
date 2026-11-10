//! A shared native scene for designer PDF and scalable PNG output.
//! Text and shapes retain geometry; only imported bitmap objects remain images.
use super::*;
use ab_glyph::{point, Font, Glyph, OutlineCurve, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};
use tiny_skia::{FillRule, Mask, Paint, Path, PathBuilder, Pixmap, Rect, Transform};

mod pdf;

pub(super) struct Scene<'a> {
    design: &'a DesignDocument,
    dpmm: i32,
    layers: Vec<Layer>,
}

struct Layer {
    transform: Transform,
    bounds: Rect,
    ops: Vec<Op>,
}

enum Op {
    Path(Path, u8, FillRule),
    Text(TextRun),
    Image(RgbaImage),
}

struct TextRun {
    face: TextFont,
    text: String,
    glyphs: Vec<(char, Glyph)>,
    color: u8,
}

impl<'a> Scene<'a> {
    pub(super) fn new(design: &'a DesignDocument, dpmm: i32) -> Result<Self> {
        design.validate(dpmm)?;
        render::font(design, TextFont::Document)?;
        let mut layers = Vec::new();
        for (index, object) in design
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.style.visible)
        {
            layers.push(
                layer(design, object, dpmm)
                    .with_context(|| format!("Object {} ({})", index + 1, object.style.name))?,
            );
        }
        Ok(Self {
            design,
            dpmm,
            layers,
        })
    }

    pub(super) fn pdf(&self) -> Result<Vec<u8>> {
        pdf::encode(self)
    }

    pub(super) fn raster(&self, scale: u32, antialias: bool) -> Result<GrayImage> {
        ensure!(
            (1..=4).contains(&scale),
            "PNG scale must be between 1 and 4"
        );
        let w = render::canvas_extent(self.design.width_mm, self.dpmm) * scale;
        let h = render::canvas_extent(self.design.height_mm, self.dpmm) * scale;
        validate_pixels(w, h)?;
        let mut pixmap = Pixmap::new(w, h).context("Cannot allocate output canvas")?;
        pixmap.fill(tiny_skia::Color::WHITE);
        for layer in &self.layers {
            let transform =
                Transform::from_scale(scale as f32, scale as f32).pre_concat(layer.transform);
            let mut mask = Mask::new(w, h).context("Cannot allocate clip mask")?;
            mask.fill_path(
                &PathBuilder::from_rect(layer.bounds),
                FillRule::Winding,
                true,
                transform,
            );
            for op in &layer.ops {
                match op {
                    Op::Path(path, color, rule) => {
                        pixmap.fill_path(path, &paint(*color), *rule, transform, Some(&mask));
                    }
                    Op::Text(run) => {
                        let font = render::font(self.design, run.face)?;
                        for (_, glyph) in &run.glyphs {
                            if let Some(path) = glyph_path(&font, glyph) {
                                let local = Transform::from_row(
                                    glyph.scale.x / font.height_unscaled(),
                                    0.,
                                    0.,
                                    -glyph.scale.y / font.height_unscaled(),
                                    glyph.position.x,
                                    glyph.position.y,
                                );
                                pixmap.fill_path(
                                    &path,
                                    &paint(run.color),
                                    FillRule::Winding,
                                    transform.pre_concat(local),
                                    Some(&mask),
                                );
                            }
                        }
                    }
                    Op::Image(image) => {
                        let mut source = Pixmap::new(image.width(), image.height())
                            .context("Cannot allocate image")?;
                        for (out, p) in source.pixels_mut().iter_mut().zip(image.pixels()) {
                            *out =
                                tiny_skia::ColorU8::from_rgba(p[0], p[1], p[2], p[3]).premultiply();
                        }
                        pixmap.draw_pixmap(
                            0,
                            0,
                            source.as_ref(),
                            &tiny_skia::PixmapPaint::default(),
                            transform,
                            Some(&mask),
                        );
                    }
                }
            }
        }
        let pixels = pixmap
            .pixels()
            .iter()
            .map(|p| {
                if antialias {
                    p.red()
                } else if p.red() > 128 {
                    255
                } else {
                    0
                }
            })
            .collect();
        GrayImage::from_raw(w, h, pixels).context("Invalid rendered canvas")
    }
}

fn paint(gray: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(gray, gray, gray, 255);
    paint
}

fn layer(design: &DesignDocument, o: &DesignObject, dpmm: i32) -> Result<Layer> {
    let (w, h) = (
        (o.width * dpmm as f64).round() as u32,
        (o.height * dpmm as f64).round() as u32,
    );
    let (x, y) = (
        (o.x * dpmm as f64).round() as f32,
        (o.y * dpmm as f64).round() as f32,
    );
    let bounds =
        Rect::from_xywh(0., 0., w as f32, h as f32).context("Invalid object dimensions")?;
    let transform = match o.style.rotation {
        90 => Transform::from_row(0., 1., -1., 0., x + h as f32, y),
        180 => Transform::from_row(-1., 0., 0., -1., x + w as f32, y + h as f32),
        270 => Transform::from_row(0., -1., 1., 0., x, y + w as f32),
        _ => Transform::from_translate(x, y),
    };
    let mut ops = Vec::new();
    let color = if o.style.background == Background::Black {
        255
    } else {
        0
    };
    if o.style.background != Background::Transparent {
        ops.push(Op::Path(
            PathBuilder::from_rect(bounds),
            255 - color,
            FillRule::Winding,
        ));
    }
    match o.kind {
        ObjectKind::Text => add_text(&mut ops, design, o, &o.text, w, h, 0, dpmm, color)?,
        ObjectKind::Rectangle | ObjectKind::Ellipse | ObjectKind::Line => {
            ops.push(Op::Path(shape(o, bounds, dpmm)?, color, FillRule::EvenOdd));
        }
        ObjectKind::Image => {
            let pixels = render::object_pixels(o, design, dpmm)?;
            let image = RgbaImage::from_fn(w, h, |x, y| {
                let gray = pixels.get_pixel(x, y)[0];
                let alpha = if o.style.background == Background::Transparent && gray == 255 {
                    0
                } else {
                    255
                };
                let gray = if color == 255 { 255 - gray } else { gray };
                Rgba([gray, gray, gray, alpha])
            });
            ops.push(Op::Image(image));
        }
        _ => {
            let layout = symbols::layout(o, w, h, dpmm)?;
            if let Some(path) = symbol_path(&layout) {
                ops.push(Op::Path(path, color, FillRule::Winding));
            }
            if layout.text_height > 0 {
                let mut caption = o.clone();
                caption.style.horizontal = HorizontalAlign::Center;
                caption.style.vertical = VerticalAlign::Top;
                caption.style.wrap = false;
                add_text(
                    &mut ops,
                    design,
                    &caption,
                    &layout.caption,
                    w,
                    layout.text_height,
                    layout.text_y,
                    dpmm,
                    color,
                )?;
            }
        }
    }
    Ok(Layer {
        transform,
        bounds,
        ops,
    })
}

#[allow(clippy::too_many_arguments)]
fn add_text(
    ops: &mut Vec<Op>,
    design: &DesignDocument,
    o: &DesignObject,
    text: &str,
    w: u32,
    h: u32,
    offset_y: u32,
    dpmm: i32,
    color: u8,
) -> Result<()> {
    let font = render::font(design, o.style.font)?;
    let scale = PxScale::from((o.font_mm * dpmm as f64) as f32);
    let scaled = font.as_scaled(scale);
    for line in render::text_lines(w, h, text, o, &font, dpmm)? {
        let mut cursor = 0.;
        let mut previous = None;
        let mut glyphs = Vec::new();
        for ch in line.text.chars() {
            let id = scaled.glyph_id(ch);
            let glyph = id.with_scale_and_position(
                scale,
                point(
                    line.x as f32 + cursor,
                    (line.y + offset_y as i32) as f32 + scaled.ascent(),
                ),
            );
            cursor += scaled.h_advance(id);
            // Match the existing imageproc text layout, including its kerning order.
            if font.outline_glyph(glyph.clone()).is_some() {
                if let Some(prev) = previous {
                    cursor += scaled.kern(id, prev);
                }
                previous = Some(id);
            }
            glyphs.push((ch, glyph));
        }
        ops.push(Op::Text(TextRun {
            face: o.style.font,
            text: line.text,
            glyphs,
            color,
        }));
    }
    Ok(())
}

fn glyph_path(font: &ab_glyph::FontRef<'_>, glyph: &Glyph) -> Option<Path> {
    let outline = font.outline(glyph.id)?;
    let mut builder = PathBuilder::new();
    let mut last = None;
    for curve in outline.curves {
        let start = match curve {
            OutlineCurve::Line(a, _)
            | OutlineCurve::Quad(a, _, _)
            | OutlineCurve::Cubic(a, _, _, _) => a,
        };
        if last != Some(start) {
            if last.is_some() {
                builder.close();
            }
            builder.move_to(start.x, start.y);
        }
        last = Some(match curve {
            OutlineCurve::Line(_, b) => {
                builder.line_to(b.x, b.y);
                b
            }
            OutlineCurve::Quad(_, b, c) => {
                builder.quad_to(b.x, b.y, c.x, c.y);
                c
            }
            OutlineCurve::Cubic(_, b, c, d) => {
                builder.cubic_to(b.x, b.y, c.x, c.y, d.x, d.y);
                d
            }
        });
    }
    builder.close();
    builder.finish()
}

fn shape(o: &DesignObject, bounds: Rect, dpmm: i32) -> Result<Path> {
    let (w, h) = (bounds.width(), bounds.height());
    let stroke = (o.stroke_mm * dpmm as f64).round().max(1.) as f32;
    let mut builder = PathBuilder::new();
    if o.kind == ObjectKind::Line {
        let (a, b) = match o.style.line_direction {
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
        if a == b {
            builder.push_oval(bounds);
            return builder.finish().context("Invalid line");
        }
        builder.move_to(a.0, a.1);
        builder.line_to(b.0, b.1);
        return builder
            .finish()
            .and_then(|p| {
                p.stroke(
                    &tiny_skia::Stroke {
                        width: stroke,
                        line_cap: tiny_skia::LineCap::Round,
                        ..Default::default()
                    },
                    1.,
                )
            })
            .context("Invalid line");
    }
    let radius = (o.style.corner_radius_mm * dpmm as f64) as f32;
    if o.kind == ObjectKind::Ellipse {
        builder.push_oval(bounds);
    } else {
        rounded_rect(&mut builder, bounds, radius);
    }
    if !o.style.filled {
        if let Some(inner) = Rect::from_xywh(stroke, stroke, w - 2. * stroke, h - 2. * stroke) {
            if o.kind == ObjectKind::Ellipse {
                builder.push_oval(inner);
            } else {
                rounded_rect(&mut builder, inner, (radius - stroke).max(0.));
            }
        }
    }
    builder.finish().context("Invalid shape")
}

fn rounded_rect(b: &mut PathBuilder, rect: Rect, radius: f32) {
    let r = radius.min(rect.width().min(rect.height()) / 2.);
    if r == 0. {
        b.push_rect(rect);
        return;
    }
    let (x, y, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let c = r * 0.552_284_8;
    b.move_to(x + r, y);
    b.line_to(right - r, y);
    b.cubic_to(right - r + c, y, right, y + r - c, right, y + r);
    b.line_to(right, bottom - r);
    b.cubic_to(
        right,
        bottom - r + c,
        right - r + c,
        bottom,
        right - r,
        bottom,
    );
    b.line_to(x + r, bottom);
    b.cubic_to(x + r - c, bottom, x, bottom - r + c, x, bottom - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    b.close();
}

/// Barcode modules are rectangles, not a traced full-page image. Merge matching
/// runs vertically so linear bars and matrix columns remain compact in PDF.
fn symbol_path(layout: &symbols::SymbolLayout) -> Option<Path> {
    use std::collections::BTreeMap;
    let mut builder = PathBuilder::new();
    let mut active = BTreeMap::<(u32, u32), u32>::new();
    let sx = layout.module as f32;
    let sy = layout.target_h as f32 / layout.source.height() as f32;
    let mut flush = |(x, width): (u32, u32), from: u32, to: u32| {
        builder.push_rect(
            Rect::from_xywh(
                layout.x as f32 + x as f32 * sx,
                layout.y as f32 + from as f32 * sy,
                width as f32 * sx,
                (to - from) as f32 * sy,
            )
            .unwrap(),
        );
    };
    for y in 0..layout.source.height() {
        let mut next = BTreeMap::new();
        let mut x = 0;
        while x < layout.source.width() {
            if layout.source.get_pixel(x, y)[0] != 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < layout.source.width() && layout.source.get_pixel(x, y)[0] == 0 {
                x += 1;
            }
            let key = (start, x - start);
            next.insert(key, active.remove(&key).unwrap_or(y));
        }
        for (key, from) in active {
            flush(key, from, y);
        }
        active = next;
    }
    for (key, from) in active {
        flush(key, from, layout.source.height());
    }
    builder.finish()
}
