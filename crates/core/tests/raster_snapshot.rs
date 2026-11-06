use labelize_printer_core::*;
use std::fs;

#[test]
fn ipp_begin_freezes_settings_and_repeated_manifest_does_not_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(temp.path().join("state")).unwrap();
    let store = Store::open(paths.clone()).unwrap();
    let initial = Settings {
        output_dir: temp.path().join("original"),
        autostart: false,
        ..Default::default()
    };
    store.save_settings(&initial).unwrap();
    let job = store
        .create(
            "IPP task",
            "ipp:ipp-test",
            PrintJobInput::RasterPages { pages: vec![] },
            JobState::Receiving,
        )
        .unwrap();
    let mut next = initial.clone();
    next.output_dir = temp.path().join("changed");
    next.format = OutputFormat::Png;
    store.save_settings(&next).unwrap();
    let dir = paths.root.join("incoming/test");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("page.pgm"), b"P5\n1 1\n255\n\0").unwrap();
    let manifest = RasterManifest {
        job_id: Some(job.id.clone()),
        title: job.title.clone(),
        source_job_id: "ipp-test".into(),
        error: None,
        pages: vec![RasterPage {
            file: "page.pgm".into(),
            width_mm: 1.0,
            height_mm: 1.0,
            dpi_x: 25.4,
            dpi_y: 25.4,
        }],
    };
    let file = dir.join("manifest.json");
    fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let ready = store.import_raster(&file).unwrap();
    assert_eq!(ready.settings, initial);
    assert_eq!(ready.id, job.id);
    assert_eq!(ready.state, JobState::Queued);
    assert_eq!(store.import_raster(&file).unwrap().id, ready.id);
    assert_eq!(store.jobs(10).unwrap().len(), 1);
}
