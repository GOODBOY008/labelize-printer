use image::{GrayImage, Luma};
use labelize_printer_core::{
    design::*,
    document::{self, DocumentKind},
    output, AppPaths, OutputFormat, Settings, Store,
};

fn render(design: &DesignDocument, dpmm: i32) -> anyhow::Result<GrayImage> {
    let mut pixels = None;
    document::render(
        &serde_json::to_vec(design)?,
        DocumentKind::Design,
        &Settings {
            output_dir: std::env::temp_dir(),
            dpmm,
            ..Default::default()
        },
        |p| {
            pixels = Some(p.pixels);
            Ok(())
        },
    )?;
    Ok(pixels.unwrap())
}
fn single(kind: ObjectKind) -> DesignDocument {
    DesignDocument {
        objects: vec![DesignObject::new(kind)],
        ..Default::default()
    }
}
fn ink(image: &GrayImage) -> usize {
    image.pixels().filter(|p| p[0] == 0).count()
}

#[test]
fn version_one_templates_keep_their_previous_release_pixels() {
    let legacy: DesignDocument =
        serde_json::from_slice(include_bytes!("fixtures/design-v1.labelize.json")).unwrap();
    assert_eq!(legacy.version, 1);
    assert_eq!(legacy.objects[0].style, ObjectStyle::default());
    let expected = image::load_from_memory(include_bytes!("fixtures/design-v1.png"))
        .unwrap()
        .into_luma8();
    assert_eq!(render(&legacy, 8).unwrap(), expected);
    let mut upgraded = legacy;
    upgraded.version = TEMPLATE_VERSION;
    let reopened: DesignDocument =
        serde_json::from_slice(&serde_json::to_vec(&upgraded).unwrap()).unwrap();
    assert_eq!(render(&reopened, 8).unwrap(), expected);
}

#[test]
fn every_barcode_roundtrips_through_a_decoder_at_both_resolutions_and_rotations() {
    use rxing::BarcodeFormat::*;
    let formats = [
        CODE_128,
        CODE_39,
        EAN_13,
        EAN_8,
        UPC_A,
        ITF,
        QR_CODE,
        DATA_MATRIX,
        PDF_417,
        AZTEC,
    ];
    for (kind, format) in ObjectKind::BARCODES.into_iter().zip(formats) {
        for dpmm in [8, 12] {
            for rotation in [0, 90, 180, 270] {
                let mut d = single(kind);
                d.objects[0].style.rotation = rotation;
                d.objects[0].style.human_readable = !kind.is_matrix();
                let page = render(&d, dpmm).unwrap();
                // Decode the printed object, including its quiet zone and caption.
                // Aztec's detector starts near the image center; a whole label may
                // contain many objects away from that point.
                let o = &d.objects[0];
                let (w, h) = o.bounds_size();
                let pixels = image::imageops::crop_imm(
                    &page,
                    (o.x * dpmm as f64).round() as u32,
                    (o.y * dpmm as f64).round() as u32,
                    (w * dpmm as f64).round() as u32,
                    (h * dpmm as f64).round() as u32,
                )
                .to_image();
                let decoded = rxing::helpers::detect_in_luma(
                    pixels.to_vec(),
                    pixels.width(),
                    pixels.height(),
                    Some(format),
                )
                .unwrap_or_else(|e| panic!("{kind:?} at {dpmm} dpmm / {rotation}°: {e:?}"));
                assert_eq!(
                    decoded.getText(),
                    d.objects[0].text,
                    "{kind:?} at {dpmm} dpmm / {rotation}°"
                );
            }
        }
    }
}

#[test]
fn barcode_options_and_strict_data_validation_affect_actual_output() {
    let mut d = single(ObjectKind::Qr);
    d.objects[0].text = "Labelize QR settings 0123456789".into();
    let normal = render(&d, 8).unwrap();
    d.objects[0].style.qr_ecc = QrEcc::H;
    assert_ne!(render(&d, 8).unwrap(), normal);
    d.objects[0].style.module_mm = 0.25;
    let small = render(&d, 8).unwrap();
    assert!(ink(&small) < ink(&normal));
    d.objects[0].style.module_mm = 10.;
    assert!(render(&d, 8).is_err());
    d.objects[0].style.module_mm = 0.;
    d.objects[0].style.quiet_modules = 3;
    assert!(render(&d, 8).is_err());
    for (kind, invalid) in [
        (ObjectKind::Ean13, "5901234123450"),
        (ObjectKind::Ean8, "96385070"),
        (ObjectKind::Upca, "012345678900"),
        (ObjectKind::Itf, "12345"),
        (ObjectKind::Itf, "12x4"),
        (ObjectKind::Code39, "lowercase"),
    ] {
        let mut d = single(kind);
        d.objects[0].text = invalid.into();
        assert!(render(&d, 8).is_err(), "Silently modified {kind:?} data");
    }
    let mut dm = single(ObjectKind::DataMatrix);
    dm.objects[0].style.matrix_shape = MatrixShape::Rectangle;
    assert!(ink(&render(&dm, 8).unwrap()) > 0);
    let mut pdf = single(ObjectKind::Pdf417);
    let normal = render(&pdf, 8).unwrap();
    pdf.objects[0].style.pdf417_columns = 4;
    pdf.objects[0].style.pdf417_security = 4;
    assert_ne!(render(&pdf, 8).unwrap(), normal);
}

#[test]
fn shapes_rotation_opacity_visibility_and_layer_order_are_pixel_exact() {
    let mut d = single(ObjectKind::Rectangle);
    d.width_mm = 30.;
    d.height_mm = 30.;
    d.objects[0].width = 20.;
    d.objects[0].height = 10.;
    d.objects[0].style.filled = true;
    let filled = render(&d, 8).unwrap();
    assert_eq!(ink(&filled), 160 * 80);
    d.objects[0].style.corner_radius_mm = 2.;
    let rounded = render(&d, 8).unwrap();
    assert!(ink(&rounded) < ink(&filled));
    assert_eq!(rounded.get_pixel(40, 40)[0], 255);
    d.objects[0].style.rotation = 90;
    let rotated = render(&d, 8).unwrap();
    assert_eq!(ink(&rotated), ink(&rounded));
    assert_eq!(rotated.get_pixel(115, 170)[0], 0);
    d.objects[0].style.visible = false;
    assert_eq!(ink(&render(&d, 8).unwrap()), 0);
    d.objects[0].style.visible = true;
    let mut cover = DesignObject::new(ObjectKind::Rectangle);
    cover.width = 5.;
    cover.height = 5.;
    cover.style.background = Background::White;
    d.objects.push(cover);
    let foreground = render(&d, 8).unwrap();
    assert_eq!(foreground.get_pixel(55, 55)[0], 255);
    d.reorder(1, LayerOrder::Back).unwrap();
    assert_eq!(render(&d, 8).unwrap().get_pixel(55, 55)[0], 0);
    for kind in [ObjectKind::Ellipse, ObjectKind::Line] {
        let mut d = single(kind);
        assert!(ink(&render(&d, 8).unwrap()) > 0);
        d.objects[0].style.filled = true;
        assert!(ink(&render(&d, 8).unwrap()) > 0);
    }
}

#[test]
fn text_wrap_alignment_fonts_line_spacing_and_black_background() {
    let mut d = single(ObjectKind::Text);
    d.objects[0].width = 20.;
    d.objects[0].height = 25.;
    d.objects[0].text = "Alpha beta gamma delta".into();
    assert!(render(&d, 8).is_err());
    d.objects[0].style.wrap = true;
    let left = render(&d, 8).unwrap();
    assert!(ink(&left) > 0);
    d.objects[0].style.horizontal = HorizontalAlign::Right;
    d.objects[0].style.vertical = VerticalAlign::Bottom;
    let right = render(&d, 8).unwrap();
    assert_ne!(left, right);
    d.objects[0].style.font = TextFont::BoldMonospace;
    assert!(ink(&render(&d, 8).unwrap()) > ink(&right));
    d.objects[0].style.background = Background::Black;
    let dark = render(&d, 8).unwrap();
    assert!(ink(&dark) > ink(&right));
    d.objects[0].style.line_spacing = 5.;
    assert!(render(&d, 8).is_err());
}

#[test]
fn image_fit_threshold_and_rotated_alignment_preserve_bounds() {
    let mut d = single(ObjectKind::Image);
    let image = GrayImage::from_fn(4, 2, |x, _| Luma([if x < 2 { 100 } else { 200 }]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    d.objects[0].image = bytes.into_inner();
    d.objects[0].width = 20.;
    d.objects[0].height = 20.;
    let contain = render(&d, 8).unwrap();
    assert_eq!(ink(&contain), 80 * 80);
    d.objects[0].style.image_fit = ImageFit::Cover;
    d.objects[0].style.horizontal = HorizontalAlign::Center;
    assert_eq!(ink(&render(&d, 8).unwrap()), 80 * 160);
    d.objects[0].style.image_fit = ImageFit::Stretch;
    d.objects[0].style.image_threshold = 220;
    assert_eq!(ink(&render(&d, 8).unwrap()), 160 * 160);
    d.objects[0].width = 30.;
    d.rotate(0, 90).unwrap();
    d.align(0, Alignment::Right).unwrap();
    d.align(0, Alignment::Bottom).unwrap();
    assert_eq!((d.objects[0].x, d.objects[0].y), (82., 122.));
    d.objects[0].style.locked = true;
    let locked = d.clone();
    assert!(d.align(0, Alignment::Left).is_err());
    assert!(d.reorder(0, LayerOrder::Back).is_err());
    assert!(d.rotate(0, 180).is_err());
    assert_eq!(d, locked);
}

#[test]
fn extended_template_roundtrip_png_export_and_pdf_share_preview_pixels() {
    let temp = tempfile::tempdir().unwrap();
    let mut d = single(ObjectKind::Text);
    d.objects[0].text = "Portable\nDesign v2".into();
    d.objects[0].height = 25.;
    d.objects[0].style.name = "Product title".into();
    d.objects[0].style.font = TextFont::BoldMonospace;
    d.objects[0].style.horizontal = HorizontalAlign::Center;
    d.objects[0].style.locked = true;
    let mut dm = DesignObject::new(ObjectKind::DataMatrix);
    dm.y = 40.;
    dm.style.rotation = 90;
    dm.style.module_mm = 0.5;
    d.objects.push(dm);
    let bytes = serde_json::to_vec(&d).unwrap();
    let restored: DesignDocument = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored, d);
    let input = temp.path().join("design.labelize.json");
    std::fs::write(&input, bytes).unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    for format in [OutputFormat::Png, OutputFormat::Pdf] {
        let settings = Settings {
            output_dir: temp.path().join("output"),
            format,
            ..Default::default()
        };
        let job = store
            .import_document(&input, "designer", DocumentKind::Design, settings)
            .unwrap();
        let result = output::render_job(&paths, &job).unwrap();
        assert_eq!(result.page_count, 1);
        if format == OutputFormat::Png {
            assert_eq!(
                image::open(&result.files[0]).unwrap().to_luma8(),
                render(&d, 8).unwrap()
            );
        } else {
            let pdf = lopdf::Document::load(&result.files[0]).unwrap();
            assert_eq!(pdf.get_pages().len(), 1);
            let stream = pdf
                .objects
                .values()
                .find_map(|o| {
                    o.as_stream().ok().filter(|s| {
                        s.dict.get(b"Subtype").ok().and_then(|v| v.as_name().ok()) == Some(b"Image")
                    })
                })
                .unwrap();
            assert_eq!(stream.dict.get(b"Width").unwrap().as_i64().unwrap(), 816);
            assert_eq!(stream.dict.get(b"Height").unwrap().as_i64().unwrap(), 1216);
            assert_eq!(
                stream.decompressed_content().unwrap(),
                render(&d, 8).unwrap().to_vec()
            );
        }
    }
}
