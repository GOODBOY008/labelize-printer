use labelize_printer_core::*;
use std::fs;

fn setup() -> (tempfile::TempDir, AppPaths, Store) {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    store
        .save_settings(&Settings {
            output_dir: temp.path().join("输出"),
            autostart: false,
            ..Default::default()
        })
        .unwrap();
    (temp, paths, store)
}

fn zpl(store: &Store, content: &[u8]) -> JobRecord {
    let file = store.paths.root.join("incoming/label.zpl");
    fs::write(&file, content).unwrap();
    store.import_zpl(&file, "中文任务", "test").unwrap()
}

#[test]
fn zpl_pdf_has_copies_and_independent_physical_page_sizes() {
    let (_temp, paths, store) = setup();
    let job = zpl(
        &store,
        b"^XA^PW400^LL240^FO10,10^FDone^FS^PQ2^XZ^XA^PW800^LL1200^FO10,10^FDtwo^FS^PQ1^XZ",
    );
    let receipt = output::render_job(&paths, &job).unwrap();
    assert_eq!(receipt.page_count, 3);
    assert_eq!(receipt.files.len(), 1);
    let pdf = lopdf::Document::load(&receipt.files[0]).unwrap();
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 3);
    for (number, id) in pages {
        let media = pdf
            .get_dictionary(id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        let (w, h) = if number <= 2 {
            (50.0, 30.0)
        } else {
            (100.0, 150.0)
        };
        assert!((media[2].as_float().unwrap() as f64 - w * 72.0 / 25.4).abs() < 0.001);
        assert!((media[3].as_float().unwrap() as f64 - h * 72.0 / 25.4).abs() < 0.001);
    }
    // Simulate a process crash after the atomic directory commit, before SQLite.
    let again = output::render_job(&paths, &job).unwrap();
    assert_eq!(again.files, receipt.files);
    store.record_receipt(&job.id, &again).unwrap();
    assert_eq!(store.job(&job.id).unwrap().state, JobState::Saved);
    assert_eq!(fs::read_dir(&job.settings.output_dir).unwrap().count(), 1);
}

#[test]
fn raster_preserves_pixels_page_order_and_resolution_without_zpl() {
    let (_temp, paths, store) = setup();
    let mut settings = store.settings().unwrap();
    settings.format = OutputFormat::Png;
    settings.render_mode = RenderMode::Vector;
    settings.antialias = true;
    settings.png_scale = 4;
    store.save_settings(&settings).unwrap();
    let dir = paths.root.join("incoming/raster");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("a.pgm"), b"P5\n3 2\n255\n\x00\x21\xff\x7f\x42\xc0").unwrap();
    fs::write(dir.join("b.pgm"), b"P5\n2 1\n255\n\x01\xfe").unwrap();
    let page = |file: &str| RasterPage {
        file: file.into(),
        width_mm: 25.4,
        height_mm: 12.7,
        dpi_x: 300.0,
        dpi_y: 203.0,
    };
    let manifest = RasterManifest {
        job_id: None,
        title: "Native raster".into(),
        source_job_id: "ipp-1".into(),
        error: None,
        pages: vec![page("a.pgm"), page("b.pgm"), page("a.pgm")],
    };
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let job = store.import_raster(&dir.join("manifest.json")).unwrap();
    let receipt = output::render_job(&paths, &job).unwrap();
    assert_eq!(receipt.page_count, 3);
    for (i, file) in receipt.files.iter().enumerate() {
        let img = image::open(file).unwrap().into_luma8();
        if i == 1 {
            assert_eq!(img.as_raw(), &[1, 254]);
        } else {
            assert_eq!(img.as_raw(), &[0, 33, 255, 127, 66, 192]);
        }
        let png = png::Decoder::new(std::io::BufReader::new(fs::File::open(file).unwrap()))
            .read_info()
            .unwrap();
        let dimensions = png.info().pixel_dims.unwrap();
        assert_eq!(dimensions.xppu, 11811);
        assert_eq!(dimensions.yppu, 7992);
    }
}

#[test]
fn failed_output_is_retryable_without_changing_settings_snapshot_or_id() {
    let (temp, paths, store) = setup();
    let blocked = temp.path().join("file-not-directory");
    fs::write(&blocked, b"occupied").unwrap();
    let mut settings = store.settings().unwrap();
    settings.output_dir = blocked.clone();
    store.save_settings(&settings).unwrap();
    let job = zpl(&store, b"^XA^FO0,0^FDtest^FS^XZ");
    assert!(output::render_job(&paths, &job).is_err());
    store
        .transition(&job.id, JobState::Failed, Some("blocked".into()))
        .unwrap();
    settings.dpmm = 12;
    store.save_settings(&settings).unwrap();
    let retry = store.retry(&job.id).unwrap();
    assert_eq!(retry.id, job.id);
    assert_eq!(retry.settings.dpmm, 8);
    fs::remove_file(blocked).unwrap();
    assert!(output::render_job(&paths, &retry).is_ok());
}

#[test]
fn interrupted_reception_fails_and_processing_recovers() {
    let (_temp, paths, store) = setup();
    let complete = zpl(&store, b"^XA^FO0,0^FDtest^FS^XZ");
    store
        .transition(&complete.id, JobState::Processing, None)
        .unwrap();
    let partial = store
        .create(
            "partial",
            "raw",
            PrintJobInput::Zpl {
                file: "input.zpl".into(),
            },
            JobState::Receiving,
        )
        .unwrap();
    drop(store);
    let store = Store::open(paths).unwrap();
    store.recover().unwrap();
    assert_eq!(store.job(&complete.id).unwrap().state, JobState::Queued);
    assert_eq!(store.job(&partial.id).unwrap().state, JobState::Failed);
    assert!(store.retry(&partial.id).is_err());
}

#[test]
fn limits_and_invalid_inputs_do_not_commit_output() {
    let (_temp, paths, store) = setup();
    for bytes in [
        b"^XA^FO0,0^FDtest^FS".as_slice(),
        b"^XA^FO0,0^FDtest^FS^PQ1001^XZ",
        b"^XA^PW32000^LL32000^FO0,0^FDtest^FS^XZ",
    ] {
        let job = zpl(&store, bytes);
        assert!(output::render_job(&paths, &job).is_err());
        assert_eq!(fs::read_dir(&job.settings.output_dir).unwrap().count(), 0);
    }
    let dir = paths.root.join("incoming/escape");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("manifest.json"), br#"{"title":"escape","source_job_id":"bad","pages":[{"file":"../../state.db","width_mm":10,"height_mm":10,"dpi_x":203,"dpi_y":203}]}"#).unwrap();
    assert!(store.import_raster(&dir.join("manifest.json")).is_err());
}
