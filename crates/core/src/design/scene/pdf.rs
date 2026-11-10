use super::*;
use krilla::{
    geom,
    page::PageSettings,
    paint::Fill,
    paint::FillRule as PdfRule,
    tagging::{ContentTag, SpanTag},
    text::{Font as PdfFont, GlyphId, KrillaGlyph},
    Document,
};

pub(super) fn encode(scene: &Scene<'_>) -> Result<Vec<u8>> {
    let mut document = Document::new();
    let mut fonts = Vec::<(TextFont, PdfFont)>::new();
    let mut page = document.start_page_with(
        PageSettings::from_wh(
            page_extent(scene.design.width_mm),
            page_extent(scene.design.height_mm),
        )
        .context("Invalid PDF page size")?,
    );
    let mut surface = page.surface();
    let scale = (72. / (25.4 * scene.dpmm as f64)) as f32;
    surface.push_transform(&geom::Transform::from_scale(scale, scale));
    for layer in &scene.layers {
        let t = layer.transform;
        surface.push_transform(&geom::Transform::from_row(
            t.sx, t.ky, t.kx, t.sy, t.tx, t.ty,
        ));
        surface.push_clip_path(
            &path(&PathBuilder::from_rect(layer.bounds))?,
            &PdfRule::NonZero,
        );
        for op in &layer.ops {
            match op {
                Op::Path(p, color, rule) => {
                    surface.set_fill(Some(Fill {
                        rule: if *rule == FillRule::EvenOdd {
                            PdfRule::EvenOdd
                        } else {
                            PdfRule::NonZero
                        },
                        ..fill(*color)
                    }));
                    surface.draw_path(&path(p)?);
                }
                Op::Image(image) => {
                    surface.draw_image(
                        krilla::image::Image::from_rgba8(
                            image.as_raw().clone(),
                            image.width(),
                            image.height(),
                        ),
                        geom::Size::from_wh(image.width() as f32, image.height() as f32)
                            .context("Invalid bitmap size")?,
                    );
                }
                Op::Text(run) => {
                    let font =
                        if let Some((_, font)) = fonts.iter().find(|(face, _)| *face == run.face) {
                            font.clone()
                        } else {
                            let font = PdfFont::new(
                                render::font_bytes(scene.design, run.face).to_vec().into(),
                                0,
                            )
                            .context("Cannot embed font in PDF")?;
                            fonts.push((run.face, font.clone()));
                            font
                        };
                    let metrics = render::font(scene.design, run.face)?;
                    let units = metrics.units_per_em().context("Font has no em size")?;
                    surface.set_fill(Some(fill(run.color)));
                    surface.start_tagged(ContentTag::Span(
                        SpanTag::empty().with_actual_text(Some(&run.text)),
                    ));
                    for (ch, glyph) in &run.glyphs {
                        let mut utf8 = [0; 4];
                        let text = ch.encode_utf8(&mut utf8);
                        surface.draw_glyphs(
                            geom::Point::from_xy(glyph.position.x, glyph.position.y),
                            &[KrillaGlyph {
                                glyph_id: GlyphId::new(glyph.id.0 as u32),
                                text_range: 0..text.len(),
                                x_advance: metrics.h_advance_unscaled(glyph.id) / units,
                                y_advance: 0.,
                                x_offset: 0.,
                                y_offset: 0.,
                                location: None,
                            }],
                            font.clone(),
                            text,
                            glyph.scale.y * units / metrics.height_unscaled(),
                            false,
                        );
                    }
                    surface.end_tagged();
                }
            }
        }
        surface.pop();
        surface.pop();
    }
    surface.pop();
    surface.finish();
    page.finish();
    document
        .finish()
        .map_err(|e| anyhow::anyhow!("Vector PDF: {e:?}"))
}

fn fill(gray: u8) -> Fill {
    Fill {
        paint: krilla::color::rgb::Color::new(gray, gray, gray).into(),
        ..Fill::default()
    }
}

// Do not round an exact dot boundary upward into an extra viewer pixel.
fn page_extent(mm: f64) -> f32 {
    let exact = mm * 72. / 25.4;
    let rounded = exact as f32;
    if rounded as f64 > exact {
        rounded.next_down()
    } else {
        rounded
    }
}

fn path(path: &Path) -> Result<geom::Path> {
    let mut builder = geom::PathBuilder::new();
    for segment in path.segments() {
        match segment {
            tiny_skia::PathSegment::MoveTo(p) => builder.move_to(p.x, p.y),
            tiny_skia::PathSegment::LineTo(p) => builder.line_to(p.x, p.y),
            tiny_skia::PathSegment::QuadTo(c, p) => builder.quad_to(c.x, c.y, p.x, p.y),
            tiny_skia::PathSegment::CubicTo(a, b, p) => {
                builder.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y)
            }
            tiny_skia::PathSegment::Close => builder.close(),
        }
    }
    builder.finish().context("Invalid PDF path")
}
