use image::{GrayImage, Luma};
use labelize_printer_core::{
    design::*,
    document::{self, DocumentKind},
    output, AppPaths, OutputFormat, RenderMode, Settings, Store,
};
use std::{
    fs,
    io::{BufReader, Cursor},
    path::Path,
};

fn options(root: &Path) -> Settings {
    Settings {
        output_dir: root.join("output"),
        width_mm: 50.,
        height_mm: 30.,
        autostart: false,
        ..Default::default()
    }
}
fn pixels(bytes: &[u8], kind: DocumentKind, settings: &Settings) -> anyhow::Result<GrayImage> {
    let mut page = None;
    document::render(bytes, kind, settings, |p| {
        page = Some(p.pixels);
        Ok(())
    })?;
    Ok(page.unwrap())
}
fn has_gray(pixels: &GrayImage) -> bool {
    pixels.pixels().any(|p| p[0] != 0 && p[0] != 255)
}
fn pdf_images(pdf: &lopdf::Document) -> usize {
    pdf.objects
        .values()
        .filter(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image")
            })
        })
        .count()
}
fn native_pdf(pdf: &lopdf::Document, expected: &str) {
    assert_eq!(
        pdf_images(pdf),
        0,
        "Native text and shapes must not become a page bitmap"
    );
    assert!(pdf.objects.values().any(|o| o.as_dict().is_ok_and(|d| d
        .get(b"Type")
        .and_then(|v| v.as_name())
        .ok()
        == Some(b"Font"))));
    // lopdf inserts a newline for each positioned glyph and does not consume
    // ActualText spans. Check Unicode mapping here; viewer search/extraction is
    // verified separately with Poppler/PDFium.
    let text = pdf
        .extract_text(&pdf.get_pages().keys().copied().collect::<Vec<_>>())
        .unwrap()
        .replace('\n', "");
    assert!(
        text.contains(expected),
        "Missing searchable text {expected:?}: {text:?}"
    );
    let page = *pdf.get_pages().values().next().unwrap();
    let content = lopdf::content::Content::decode(&pdf.get_page_content(page)).unwrap();
    assert!(content
        .operations
        .iter()
        .any(|o| matches!(o.operator.as_str(), "TJ" | "Tj")));
    assert!(content
        .operations
        .iter()
        .any(|o| matches!(o.operator.as_str(), "f" | "f*")));
}

#[test]
fn legacy_settings_default_to_binary_raster_and_reject_invalid_scales() {
    let original = Settings::default();
    let mut json = serde_json::to_value(&original).unwrap();
    for key in ["render_mode", "antialias", "png_scale"] {
        json.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(serde_json::from_value::<Settings>(json).unwrap(), original);
    for scale in [0, 5, u32::MAX] {
        assert!(Settings {
            png_scale: scale,
            ..original.clone()
        }
        .validate()
        .is_err());
    }
}

#[test]
fn fractional_design_dimensions_keep_the_existing_outer_canvas() {
    let mut settings = Settings {
        output_dir: std::env::temp_dir(),
        ..Default::default()
    };
    for size in [50.03, 50.1, 50.125, 50.375] {
        let design = DesignDocument {
            width_mm: size,
            height_mm: size,
            objects: vec![],
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&design).unwrap();
        settings.antialias = false;
        settings.render_mode = RenderMode::Raster;
        let expected = pixels(&bytes, DocumentKind::Design, &settings).unwrap();
        settings.antialias = true;
        assert_eq!(
            pixels(&bytes, DocumentKind::Design, &settings).unwrap(),
            expected
        );
        settings.render_mode = RenderMode::Vector;
        assert_eq!(
            pixels(&bytes, DocumentKind::Design, &settings).unwrap(),
            expected
        );
    }
}

#[test]
fn antialias_preserves_gray_edges_in_png_and_raster_pdf() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let mut settings = options(temp.path());
    for (kind, bytes) in [
        (
            DocumentKind::Zpl,
            b"^XA^FO10,10^A0N,41,41^FDAa89^FS^XZ".to_vec(),
        ),
        (
            DocumentKind::Design,
            serde_json::to_vec(&DesignDocument::default()).unwrap(),
        ),
    ] {
        settings.antialias = false;
        let binary = pixels(&bytes, kind, &settings).unwrap();
        assert!(!has_gray(&binary));
        settings.antialias = true;
        let smooth = pixels(&bytes, kind, &settings).unwrap();
        assert!(has_gray(&smooth));
        assert_eq!(binary.dimensions(), smooth.dimensions());
        let file = temp.path().join("input");
        fs::write(&file, bytes).unwrap();
        let job = store
            .import_document(&file, "AA raster", kind, settings.clone())
            .unwrap();
        let receipt = output::render_job(&paths, &job).unwrap();
        let pdf = lopdf::Document::load(&receipt.files[0]).unwrap();
        let image = pdf
            .objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .find(|s| s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image"))
            .unwrap();
        assert_eq!(&image.decompressed_content().unwrap(), smooth.as_raw());
    }
}

#[test]
fn vector_pdf_keeps_native_text_paths_copies_sizes_and_atomic_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let mut settings = options(temp.path());
    settings.render_mode = RenderMode::Vector;
    settings.antialias = true;
    settings.png_scale = 4; // This must never scale PDF page geometry.
    store.save_settings(&settings).unwrap();
    let input = temp.path().join("multi.zpl");
    fs::write(&input, b"^XA^PW400^LL240^FO10,10^A0N,24,24^FDNative^FS^FO20,60^GB100,40,3^FS^PQ2^XZ^XA^PW800^LL1200^FO10,10^A0N,24,24^FDSecond^FS^XZ").unwrap();
    let job = store.import_zpl(&input, "Native labels", "test").unwrap();
    // Rendering owns a settings/input snapshot even if the originals change.
    fs::remove_file(input).unwrap();
    store.save_settings(&options(temp.path())).unwrap();
    let restored = store.job(&job.id).unwrap();
    assert_eq!(restored.settings, settings);
    let receipt = output::render_job(&paths, &restored).unwrap();
    assert_eq!((receipt.page_count, receipt.files.len()), (3, 1));
    let pdf = lopdf::Document::load(&receipt.files[0]).unwrap();
    native_pdf(&pdf, "Native");
    for (n, id) in pdf.get_pages() {
        let media = pdf
            .get_dictionary(id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        let (w, h) = if n <= 2 { (50., 30.) } else { (100., 150.) };
        assert!((media[2].as_float().unwrap() as f64 - w * 72. / 25.4).abs() < 0.001);
        assert!((media[3].as_float().unwrap() as f64 - h * 72. / 25.4).abs() < 0.001);
    }
    assert_eq!(
        output::render_job(&paths, &job).unwrap().files,
        receipt.files
    );
    assert_eq!(fs::read_dir(&settings.output_dir).unwrap().count(), 1);
}

#[test]
fn scalable_png_preserves_physical_size_and_preview_export_identity() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let samples = [
        (
            DocumentKind::Zpl,
            b"^XA^PW400^LL240^FO10,10^A0N,31,31^FDAa89^FS^XZ".to_vec(),
        ),
        (
            DocumentKind::Design,
            serde_json::to_vec(&DesignDocument::default()).unwrap(),
        ),
    ];
    for (kind, bytes) in samples {
        for scale in [1, 2, 4] {
            let settings = Settings {
                format: OutputFormat::Png,
                render_mode: RenderMode::Vector,
                antialias: true,
                png_scale: scale,
                ..options(temp.path())
            };
            let input = temp.path().join("input");
            fs::write(&input, &bytes).unwrap();
            let preview = document::preview(
                &document::PreviewRequest {
                    input: input.clone(),
                    kind,
                    settings: settings.clone(),
                },
                &temp.path().join("preview"),
            )
            .unwrap();
            let base = if kind == DocumentKind::Design {
                (816, 1216)
            } else {
                (400, 240)
            };
            assert_eq!(
                (preview[0].width, preview[0].height),
                (base.0 * scale, base.1 * scale)
            );
            let job = store
                .import_document(&input, "Scaled", kind, settings)
                .unwrap();
            let receipt = output::render_job(&paths, &job).unwrap();
            let file = &receipt.files[0];
            let decoder = png::Decoder::new(BufReader::new(fs::File::open(file).unwrap()))
                .read_info()
                .unwrap();
            let dims = decoder.info().pixel_dims.unwrap();
            assert_eq!((dims.xppu, dims.yppu), (8000 * scale, 8000 * scale));
            let exported = image::open(file).unwrap().into_luma8();
            assert!(has_gray(&exported));
            assert_eq!(
                exported,
                image::open(temp.path().join("preview/page-1.png"))
                    .unwrap()
                    .into_luma8()
            );
        }
    }
}

#[test]
fn vector_designer_embeds_searchable_unicode_and_keeps_only_imported_images_raster() {
    let mut d = DesignDocument {
        font: labelize::assets::FONT_DEJAVU_SANS_MONO_BOLD.to_vec(),
        ..Default::default()
    };
    d.objects[0].text = "Café Ω".into();
    d.objects[0].style.background = Background::Black;
    d.objects[0].style.rotation = 90;
    let mut ellipse = DesignObject::new(ObjectKind::Ellipse);
    ellipse.x = 30.;
    ellipse.y = 50.;
    ellipse.width = 20.;
    ellipse.height = 10.;
    d.objects.push(ellipse);
    let native = d.vector_pdf(8).unwrap();
    let pdf = lopdf::Document::load_mem(&native).unwrap();
    native_pdf(&pdf, "Café Ω");
    let mut picture = DesignObject::new(ObjectKind::Image);
    let mut encoded = Cursor::new(Vec::new());
    GrayImage::from_fn(4, 4, |x, y| Luma([if x == y { 0 } else { 255 }]))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    picture.image = encoded.into_inner();
    picture.x = 50.;
    picture.y = 90.;
    d.objects.push(picture);
    let pdf = lopdf::Document::load_mem(&d.vector_pdf(8).unwrap()).unwrap();
    assert!(pdf_images(&pdf) > 0);
    assert!(pdf
        .extract_text(&[1])
        .unwrap()
        .replace('\n', "")
        .contains("Café Ω"));
}

#[test]
fn vector_barcode_modules_match_raster_and_decode_after_rotation_and_scaling() {
    for kind in ObjectKind::BARCODES {
        for dpmm in [8, 12] {
            let mut d = DesignDocument {
                objects: vec![DesignObject::new(kind)],
                ..Default::default()
            };
            // No caption: native module geometry must match the established raster exactly.
            d.objects[0].style.human_readable = false;
            let raster = d.render_pixels(dpmm, RenderMode::Raster, 1, false).unwrap();
            let vector = d.render_pixels(dpmm, RenderMode::Vector, 1, false).unwrap();
            assert_eq!(
                raster, vector,
                "Module layout changed for {kind:?} / {dpmm}"
            );
            d.objects[0].style.human_readable = true;
            d.objects[0].style.rotation = 90;
            let scaled = d.render_pixels(dpmm, RenderMode::Vector, 2, true).unwrap();
            let o = &d.objects[0];
            let (w, h) = o.bounds_size();
            let dots = (dpmm * 2) as f64;
            let crop = image::imageops::crop_imm(
                &scaled,
                (o.x * dots) as u32,
                (o.y * dots) as u32,
                (w * dots) as u32,
                (h * dots) as u32,
            )
            .to_image();
            let decoded =
                rxing::helpers::detect_in_luma(crop.to_vec(), crop.width(), crop.height(), None)
                    .unwrap_or_else(|e| panic!("Vector {kind:?} / {dpmm}: {e:?}"));
            assert_eq!(decoded.getText(), o.text);
        }
    }
}

#[test]
fn scaled_canvas_limit_fails_before_any_output_commit() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let settings = Settings {
        format: OutputFormat::Png,
        render_mode: RenderMode::Vector,
        png_scale: 4,
        ..options(temp.path())
    };
    let input = temp.path().join("large.zpl");
    // The first page fits; the second exceeds the output limit only after scaling.
    fs::write(
        &input,
        b"^XA^PW400^LL240^FDfirst^FS^XZ^XA^PW4000^LL4000^FDlarge^FS^XZ",
    )
    .unwrap();
    let job = store
        .import_document(&input, "Too large", DocumentKind::Zpl, settings.clone())
        .unwrap();
    assert!(output::render_job(&paths, &job).is_err());
    assert_eq!(fs::read_dir(settings.output_dir).unwrap().count(), 0);
}
