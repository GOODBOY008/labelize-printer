use labelize_printer_core::{
    design::{DesignDocument, DesignObject, ObjectKind},
    document::{self, DocumentKind, PreviewRequest},
    *,
};
use std::{fs, io::Cursor};

fn settings(path: &std::path::Path) -> Settings {
    Settings {
        output_dir: path.join("output"),
        width_mm: 50.,
        height_mm: 30.,
        autostart: false,
        ..Default::default()
    }
}
fn samples() -> Vec<(DocumentKind, &'static str, Vec<u8>)> {
    vec![
        (
            DocumentKind::Zpl,
            "label.zpl",
            b"^XA^PW400^LL240^FO10,10^A0N,24,24^FDHello^FS^PQ2^XZ".to_vec(),
        ),
        (
            DocumentKind::Epl,
            "label.epl",
            b"N\nq400\nQ240,0\nA10,10,0,3,1,1,N,\"Hello\"\nP1\n".to_vec(),
        ),
        (
            DocumentKind::Tspl,
            "label.tsc",
            b"SIZE 50 mm,30 mm\nCLS\nTEXT 10,10,\"3\",0,1,1,\"Hello\"\nPRINT 1\n".to_vec(),
        ),
        (
            DocumentKind::Cpcl,
            "label.cpcl",
            b"! 0 203 203 240 1\r\nPAGE-WIDTH 400\r\nTEXT 4 0 10 10 Hello\r\nPRINT\r\n".to_vec(),
        ),
        (
            DocumentKind::Dpl,
            "label.dpl",
            b"\x02L\rD11\r1911A1000100010Hello\rE\r".to_vec(),
        ),
    ]
}

#[test]
fn all_file_languages_preview_and_output_match_pixels_and_sizes() {
    let temp = tempfile::tempdir().unwrap();
    let options = settings(temp.path());
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    store.save_settings(&options).unwrap();
    for (kind, name, bytes) in samples() {
        let input = temp.path().join(name);
        fs::write(&input, &bytes).unwrap();
        assert_eq!(DocumentKind::detect(&input, &bytes), kind);
        let dir = temp.path().join(kind.name());
        let preview = document::preview(
            &PreviewRequest {
                input: input.clone(),
                kind,
                settings: options.clone(),
            },
            &dir,
        )
        .unwrap();
        assert_eq!(
            preview.len(),
            if kind == DocumentKind::Zpl { 2 } else { 1 },
            "{}",
            kind.name()
        );
        let mut output_options = options.clone();
        output_options.format = OutputFormat::Png;
        let job = store
            .import_document(&input, name, kind, output_options)
            .unwrap();
        // The import owns a byte-for-byte snapshot, even after the source disappears.
        fs::remove_file(&input).unwrap();
        assert_eq!(
            fs::read(paths.job_dir(&job.id).unwrap().join("input.label")).unwrap(),
            bytes
        );
        let receipt = output::render_job(&paths, &job).unwrap();
        assert_eq!(receipt.page_count, preview.len());
        for (page, file) in preview.iter().zip(receipt.files) {
            let actual = image::open(file).unwrap().into_luma8();
            let expected = image::open(dir.join(&page.file)).unwrap().into_luma8();
            assert_eq!(actual, expected);
            assert_eq!(actual.dimensions(), (400, 240), "{}", kind.name());
            assert!(actual.as_raw().contains(&0), "{} has no ink", kind.name());
            assert_eq!((page.width_mm, page.height_mm), (50., 30.));
        }
        assert_eq!(
            store.settings().unwrap(),
            options,
            "Document options must not overwrite queue settings"
        );
    }
}

#[test]
fn binary_graphics_are_not_utf8_decoded_or_treated_as_commands() {
    let temp = tempfile::tempdir().unwrap();
    let mut bytes = b"SIZE 10 mm,10 mm\nCLS\nBITMAP 0,0,1,2,0,".to_vec();
    bytes.extend_from_slice(&[0x00, 0xff]);
    bytes.extend_from_slice(b"\nPRINT 1\n");
    let mut pages = Vec::new();
    document::render(&bytes, DocumentKind::Tspl, &settings(temp.path()), |p| {
        pages.push(p);
        Ok(())
    })
    .unwrap();
    assert_eq!(pages[0].pixels.dimensions(), (80, 80));
    for x in 0..8 {
        assert_eq!(pages[0].pixels.get_pixel(x, 0)[0], 0);
        assert_eq!(pages[0].pixels.get_pixel(x, 1)[0], 255);
    }
}

fn portable_design() -> DesignDocument {
    let mut design = DesignDocument::default();
    let mut text = DesignObject::new(ObjectKind::Text);
    text.x = 4.;
    text.y = 4.;
    text.text = "Portable label".into();
    design.objects = vec![text];
    let mut qr = DesignObject::new(ObjectKind::Qr);
    qr.x = 4.;
    qr.y = 20.;
    qr.text = "Labelize portable template".into();
    design.objects.push(qr);
    let mut barcode = DesignObject::new(ObjectKind::Code128);
    barcode.x = 4.;
    barcode.y = 55.;
    design.objects.push(barcode);
    let mut border = DesignObject::new(ObjectKind::Rectangle);
    border.x = 2.;
    border.y = 2.;
    border.width = 96.;
    border.height = 140.;
    design.objects.push(border);
    let image = image::RgbaImage::from_fn(2, 2, |x, _| {
        if x == 0 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    });
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let mut picture = DesignObject::new(ObjectKind::Image);
    picture.image = design::normalize_image(&encoded.into_inner()).unwrap();
    picture.x = 60.;
    picture.y = 20.;
    picture.width = 20.;
    picture.height = 20.;
    design.objects.push(picture);
    design.font = labelize::assets::FONT_DEJAVU_SANS_MONO.to_vec();
    design
}

#[test]
fn portable_design_roundtrips_with_embedded_assets_and_pdf_dimensions() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let settings = settings(temp.path());
    store.save_settings(&settings).unwrap();
    let design = portable_design();
    let bytes = serde_json::to_vec(&design).unwrap();
    let restored: DesignDocument = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored, design);
    let input = temp.path().join("设计.labelize.json");
    fs::write(&input, &bytes).unwrap();
    assert_eq!(DocumentKind::detect(&input, &bytes), DocumentKind::Design);
    let mut pixels = None;
    document::render(&bytes, DocumentKind::Design, &settings, |p| {
        pixels = Some(p.pixels);
        Ok(())
    })
    .unwrap();
    let pixels = pixels.unwrap();
    assert_eq!(pixels.dimensions(), (816, 1216));
    // Encoders use transparent white modules. A QR must retain white finder
    // centres and a quiet zone instead of becoming an opaque black square.
    let qr = image::imageops::crop_imm(&pixels, 4 * 8, 20 * 8, 25 * 8, 25 * 8).to_image();
    let black = qr.as_raw().iter().filter(|&&p| p == 0).count();
    assert!(
        black > 1000 && black < 20000,
        "QR modules lost their transparency: {black}"
    );
    assert_eq!(qr.get_pixel(0, 0)[0], 255);
    assert_eq!(qr.get_pixel(6 * 4, 6 * 4)[0], 0);

    // Transparent pixels composite over white and imported assets remain embedded.
    assert_eq!(pixels.get_pixel(60 * 8, 20 * 8)[0], 255);
    assert_eq!(pixels.get_pixel(75 * 8, 20 * 8)[0], 0);
    let job = store
        .import_document(&input, "Portable template", DocumentKind::Design, settings)
        .unwrap();
    let receipt = output::render_job(&paths, &job).unwrap();
    let pdf = lopdf::Document::load(&receipt.files[0]).unwrap();
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 1);
    let bounds = pdf
        .get_dictionary(pages[&1])
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap();
    assert!((bounds[2].as_float().unwrap() as f64 - 102. * 72. / 25.4).abs() < 0.001);
}

#[test]
fn design_validation_rejects_unknown_versions_bad_geometry_and_missing_glyphs() {
    let mut design = DesignDocument {
        version: 999,
        ..Default::default()
    };
    assert!(design.to_label(8).is_err());
    design.version = 1;
    design.objects[0].x = f64::NAN;
    assert!(design.to_label(8).is_err());
    design.objects[0].x = 500.;
    assert!(design.to_label(8).is_err());
    design.objects[0].x = 5.;
    design.objects[0].text = "中文".into();
    assert!(format!("{:#}", design.to_label(8).unwrap_err()).contains("font lacks"));
    design.objects[0].text = "x".repeat(4000);
    assert!(design.to_label(8).is_err());
    design.objects[0] = DesignObject::new(ObjectKind::Qr);
    design.objects[0].width = 1.;
    design.objects[0].height = 1.;
    assert!(design.to_label(8).is_err());
}

#[test]
fn invalid_files_and_incomplete_zpl_cannot_generate_output() {
    let temp = tempfile::tempdir().unwrap();
    let options = settings(temp.path());
    assert!(document::parse(b"", DocumentKind::Zpl, &options).is_err());
    assert!(document::parse(b"^XA^FDunfinished^FS", DocumentKind::Zpl, &options).is_err());
    assert!(document::parse(b"^XA^PQ1001^XZ", DocumentKind::Zpl, &options).is_err());
    assert!(document::parse(b"{}", DocumentKind::Design, &options).is_err());
    assert!(document::read_input(temp.path()).is_err());
}
