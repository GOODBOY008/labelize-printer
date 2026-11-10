//! Crash-safe local workspace snapshots. Drafts are never written to source files.
use anyhow::{ensure, Context, Result};
use labelize_printer_core::{
    design::DesignDocument,
    document::DocumentKind,
    paths::{atomic_write, private_dir},
    Settings, MAX_INPUT_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// First nine fields retain their version-1 order for existing local drafts.
pub const DESIGN_FIELD_COUNT: usize = 17;

#[derive(Clone, Serialize, Deserialize)]
pub struct Draft {
    pub id: String,
    pub file: Option<PathBuf>,
    /// Digest of the last content read from / saved to the source path.
    pub fingerprint: Option<String>,
    pub kind: DocumentKind,
    pub design: Option<DesignDocument>,
    #[serde(skip)]
    pub source: Vec<u8>,
    #[serde(default)]
    source_blob: String,
    pub fields: Vec<String>,
    pub options: Settings,
    pub dirty: bool,
    pub revision: u64,
    pub selected: Option<usize>,
    pub page: usize,
    /// None means fit to available canvas; Some is a scale relative to 96 dpi.
    pub zoom: Option<f32>,
    pub source_mode: bool,
}

impl Draft {
    pub fn new(options: Settings) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            file: None,
            fingerprint: None,
            kind: DocumentKind::Design,
            design: Some(DesignDocument {
                width_mm: options.width_mm,
                height_mm: options.height_mm,
                ..DesignDocument::default()
            }),
            source: Vec::new(),
            source_blob: String::new(),
            fields: Vec::new(),
            options,
            dirty: true,
            revision: 0,
            selected: Some(0),
            page: 0,
            zoom: None,
            source_mode: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct Workspace {
    pub tabs: Vec<Draft>,
    pub active: Option<String>,
    pub recent: Vec<PathBuf>,
    pub setup_dismissed: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    tabs: Vec<String>,
    active: Option<String>,
    recent: Vec<PathBuf>,
    #[serde(default)]
    setup_dismissed: bool,
}

pub struct SessionStore {
    root: PathBuf,
    // Serializes close-time flushes with asynchronous autosaves. Older snapshots
    // cannot overwrite a newer close-time save, even if their worker starts late.
    generation: Mutex<u64>,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn remember_recent(recent: &mut Vec<PathBuf>, path: PathBuf) {
    let path = canonical_path(&path);
    recent.retain(|p| canonical_path(p) != path);
    recent.insert(0, path);
    recent.truncate(20);
}

/// A missing, replaced or modified source is a conflict, including a file
/// modified with the same length and modification time.
pub fn source_matches(path: &Path, expected: Option<&str>) -> bool {
    expected
        .is_some_and(|hash| read_bounded(path, MAX_INPUT_BYTES).is_ok_and(|b| digest(&b) == hash))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Workspace file exceeds size limit"
    );
    Ok(bytes)
}

impl SessionStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: root.join("workspace"),
            generation: Mutex::new(0),
        }
    }
    fn blob_path(&self, key: &str, extension: &str) -> Result<PathBuf> {
        ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid workspace blob name"
        );
        Ok(self.root.join(format!("{key}.{extension}")))
    }
    fn put(&self, bytes: &[u8], extension: &str) -> Result<String> {
        let key = digest(bytes);
        let path = self.blob_path(&key, extension)?;
        if !path.exists() {
            atomic_write(&path, bytes)?;
        }
        Ok(key)
    }
    fn get(&self, key: &str, extension: &str, limit: u64) -> Result<Vec<u8>> {
        let bytes = read_bounded(&self.blob_path(key, extension)?, limit)?;
        ensure!(
            digest(&bytes) == key,
            "Workspace snapshot checksum mismatch"
        );
        Ok(bytes)
    }
    pub fn save(&self, sequence: u64, workspace: Workspace) -> Result<()> {
        let mut saved = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        if sequence <= *saved {
            return Ok(());
        }
        private_dir(&self.root)?;
        let mut keys = Vec::new();
        for mut draft in workspace.tabs {
            ensure!(
                draft.source.len() as u64 <= MAX_INPUT_BYTES,
                "Draft exceeds 64 MiB"
            );
            draft.source_blob = self.put(&draft.source, "bin")?;
            keys.push(self.put(&serde_json::to_vec(&draft)?, "json")?);
        }
        let manifest = Manifest {
            version: 1,
            tabs: keys,
            active: workspace.active,
            recent: workspace.recent.into_iter().take(20).collect(),
            setup_dismissed: workspace.setup_dismissed,
        };
        let path = self.root.join("workspace.json");
        if let Ok(previous) = read_bounded(&path, 1024 * 1024) {
            // Never replace a usable recovery manifest with corrupt bytes.
            if serde_json::from_slice::<Manifest>(&previous).is_ok() {
                atomic_write(&self.root.join("workspace.previous.json"), &previous)?;
            }
        }
        atomic_write(&path, &serde_json::to_vec(&manifest)?)?;
        *saved = sequence;
        // Keep both complete generations. A crash at any earlier step leaves
        // the previous manifest and all its immutable blobs intact.
        self.collect_unreferenced();
        Ok(())
    }
    fn collect_unreferenced(&self) {
        let mut keep = HashSet::new();
        for name in ["workspace.json", "workspace.previous.json"] {
            let Ok(bytes) = fs::read(self.root.join(name)) else {
                continue;
            };
            let Ok(m) = serde_json::from_slice::<Manifest>(&bytes) else {
                return;
            };
            for key in m.tabs {
                let Ok(bytes) = self.get(&key, "json", MAX_INPUT_BYTES * 3) else {
                    return;
                };
                let Ok(d) = serde_json::from_slice::<Draft>(&bytes) else {
                    return;
                };
                keep.insert(format!("{key}.json"));
                keep.insert(format!("{}.bin", d.source_blob));
            }
        }
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some((key, ext)) = name.rsplit_once('.') {
                    if ["json", "bin"].contains(&ext)
                        && self.blob_path(key, ext).is_ok()
                        && !keep.contains(&name)
                    {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
    pub fn load(&self) -> (Workspace, Vec<String>) {
        let path = self.root.join("workspace.json");
        if !path.exists() {
            return (Workspace::default(), Vec::new());
        }
        let mut warnings = Vec::new();
        let manifest = |name: &str| -> Result<Manifest> {
            let m: Manifest =
                serde_json::from_slice(&read_bounded(&self.root.join(name), 1024 * 1024)?)?;
            ensure!(m.version == 1, "Unsupported workspace version");
            Ok(m)
        };
        let m = match manifest("workspace.json") {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!("Workspace recovery: {e:#}"));
                match manifest("workspace.previous.json") {
                    Ok(m) => m,
                    Err(e) => {
                        warnings.push(format!("Previous workspace: {e:#}"));
                        return (Workspace::default(), warnings);
                    }
                }
            }
        };
        let mut result = Workspace {
            active: m.active,
            recent: m.recent.into_iter().take(20).collect(),
            setup_dismissed: m.setup_dismissed,
            ..Workspace::default()
        };
        let mut ids = HashSet::new();
        for key in m.tabs {
            let load = || -> Result<Draft> {
                let mut draft: Draft =
                    serde_json::from_slice(&self.get(&key, "json", MAX_INPUT_BYTES * 3)?)?;
                uuid::Uuid::parse_str(&draft.id).context("Invalid document ID")?;
                draft.source = self.get(&draft.source_blob, "bin", MAX_INPUT_BYTES)?;
                draft.selected = draft.selected.filter(|&index| {
                    draft
                        .design
                        .as_ref()
                        .is_some_and(|d| index < d.objects.len())
                });
                ensure!(
                    [0, 9, DESIGN_FIELD_COUNT].contains(&draft.fields.len()),
                    "Invalid draft fields"
                );
                draft.zoom = draft
                    .zoom
                    .filter(|v| v.is_finite() && (0.1..=4.).contains(v));
                Ok(draft)
            };
            match load() {
                Ok(d) if ids.insert(d.id.clone()) => result.tabs.push(d),
                Ok(_) => warnings.push("Duplicate document in workspace".into()),
                Err(e) => warnings.push(format!("Could not restore draft {key}: {e:#}")),
            }
        }
        if !result
            .tabs
            .iter()
            .any(|d| Some(&d.id) == result.active.as_ref())
        {
            result.active = result.tabs.first().map(|d| d.id.clone());
        }
        (result, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_extended_design_properties_and_unfinished_numeric_fields() {
        use labelize_printer_core::design::*;
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        let mut draft = Draft::new(Settings::default());
        let mut code = DesignObject::new(ObjectKind::DataMatrix);
        code.style.name = "零件编号".into();
        code.style.rotation = 90;
        code.style.matrix_shape = MatrixShape::Rectangle;
        code.style.visible = false;
        code.style.locked = true;
        code.style.module_mm = 0.5;
        draft.design.as_mut().unwrap().objects.push(code);
        draft.selected = Some(1);
        draft.fields = vec![String::new(); DESIGN_FIELD_COUNT];
        draft.fields[12] = "-".into();
        store
            .save(
                1,
                Workspace {
                    active: Some(draft.id.clone()),
                    tabs: vec![draft.clone()],
                    ..Default::default()
                },
            )
            .unwrap();
        let (restored, warnings) = store.load();
        assert!(warnings.is_empty());
        assert_eq!(restored.tabs[0].design, draft.design);
        assert_eq!(restored.tabs[0].selected, Some(1));
        assert_eq!(restored.tabs[0].fields[12], "-");
        assert!(restored.tabs[0].dirty);
    }
    #[test]
    fn restores_invalid_edits_binary_data_and_independent_tab_settings() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        let mut a = Draft::new(Settings::default());
        a.fields = vec!["-".into(); 9];
        a.source = vec![0, 255, 1];
        a.page = 3;
        let mut b = Draft::new(Settings::default());
        b.options.dpmm = 12;
        b.options.render_mode = labelize_printer_core::RenderMode::Vector;
        b.options.antialias = true;
        b.options.png_scale = 3;
        b.zoom = Some(1.25);
        b.dirty = false;
        store
            .save(
                1,
                Workspace {
                    tabs: vec![a.clone(), b.clone()],
                    active: Some(b.id.clone()),
                    ..Workspace::default()
                },
            )
            .unwrap();
        let (loaded, errors) = store.load();
        assert!(errors.is_empty());
        assert_eq!(loaded.active, Some(b.id));
        assert_eq!(loaded.tabs[0].source, a.source);
        assert_eq!(loaded.tabs[0].fields[0], "-");
        assert!(loaded.tabs[0].dirty);
        assert_eq!(loaded.tabs[0].page, 3);
        assert_eq!(loaded.tabs[1].options.dpmm, 12);
        assert_eq!(loaded.tabs[0].options, a.options);
        assert_eq!(loaded.tabs[1].options, b.options);
        assert_eq!(loaded.tabs[1].zoom, Some(1.25));
    }
    #[test]
    fn latest_flush_wins_and_broken_manifest_recovers_previous_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        let a = Draft::new(Settings::default());
        store
            .save(
                1,
                Workspace {
                    tabs: vec![a.clone()],
                    ..Workspace::default()
                },
            )
            .unwrap();
        store.save(3, Workspace::default()).unwrap();
        store
            .save(
                2,
                Workspace {
                    tabs: vec![a.clone()],
                    ..Workspace::default()
                },
            )
            .unwrap();
        assert!(store.load().0.tabs.is_empty());
        fs::write(store.root.join("workspace.json"), b"{broken").unwrap();
        let (restored, warnings) = store.load();
        assert_eq!(restored.tabs[0].id, a.id);
        assert!(!warnings.is_empty());
    }
    #[test]
    fn a_corrupt_draft_does_not_discard_other_tabs() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        let a = Draft::new(Settings::default());
        let b = Draft::new(Settings::default());
        store
            .save(
                1,
                Workspace {
                    tabs: vec![a, b.clone()],
                    ..Workspace::default()
                },
            )
            .unwrap();
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(store.root.join("workspace.json")).unwrap()).unwrap();
        fs::write(
            store.blob_path(&manifest.tabs[0], "json").unwrap(),
            b"broken",
        )
        .unwrap();
        let (workspace, warnings) = store.load();
        assert_eq!(workspace.tabs.len(), 1);
        assert_eq!(workspace.tabs[0].id, b.id);
        assert_eq!(warnings.len(), 1);
    }
    #[test]
    fn rejects_blob_path_traversal_and_keeps_source_files_separate() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        assert!(store.blob_path("../../source", "bin").is_err());
        let file = temp.path().join("source.zpl");
        fs::write(&file, b"original").unwrap();
        let mut draft = Draft::new(Settings::default());
        draft.file = Some(file.clone());
        draft.source = b"unsaved".to_vec();
        store
            .save(
                1,
                Workspace {
                    tabs: vec![draft],
                    ..Workspace::default()
                },
            )
            .unwrap();
        assert_eq!(fs::read(file).unwrap(), b"original");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.root.join("workspace.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn write_failure_preserves_previous_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(temp.path().into());
        store.save(1, Workspace::default()).unwrap();
        fs::create_dir(store.root.join("workspace.previous.json")).unwrap();
        assert!(store
            .save(
                2,
                Workspace {
                    tabs: vec![Draft::new(Settings::default())],
                    ..Workspace::default()
                }
            )
            .is_err());
        assert!(store.load().0.tabs.is_empty());
    }
    #[test]
    fn conflicts_detect_content_changes_missing_paths_and_recent_dedup() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.zpl");
        fs::write(&file, b"old").unwrap();
        let fingerprint = digest(b"old");
        assert!(source_matches(&file, Some(&fingerprint)));
        fs::write(&file, b"new").unwrap();
        assert!(!source_matches(&file, Some(&fingerprint)));
        fs::remove_file(&file).unwrap();
        assert!(!source_matches(&file, Some(&fingerprint)));
        let mut recent = Vec::new();
        for i in 0..30 {
            remember_recent(&mut recent, temp.path().join(format!("{i}.zpl")));
        }
        remember_recent(&mut recent, temp.path().join("20.zpl"));
        assert_eq!(recent.len(), 20);
        assert!(recent[0].ends_with("20.zpl"));
    }
}
