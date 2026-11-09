//! Bounded CPU-side previews. Inactive documents still evict their GPU textures.
use anyhow::{ensure, Context, Result};
use gpui::RenderImage;
use labelize_printer_core::document::PreviewPage;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewKey {
    // Each render owns a new temporary directory, so revisions cannot alias.
    pub directory: PathBuf,
    pub page: usize,
}

#[derive(Debug)]
pub struct PreviewImages {
    pub page: PreviewPage,
    pub image: Arc<RenderImage>,
    pub thumbnails: Vec<(usize, Arc<RenderImage>)>,
}
impl PreviewImages {
    pub fn images(&self) -> impl Iterator<Item = &Arc<RenderImage>> {
        std::iter::once(&self.image).chain(self.thumbnails.iter().map(|(_, image)| image))
    }
    fn bytes(&self) -> usize {
        self.images()
            .map(|image| image.as_bytes(0).map_or(0, <[u8]>::len))
            .sum()
    }
}

/// Shared across all tabs, with an LRU byte budget rather than an unbounded per-tab cache.
pub struct PreviewCache {
    limit: usize,
    bytes: usize,
    entries: VecDeque<(PreviewKey, Arc<PreviewImages>)>,
}
impl Default for PreviewCache {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024)
    }
}
impl PreviewCache {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: 0,
            entries: VecDeque::new(),
        }
    }
    pub fn get(&mut self, key: &PreviewKey) -> Option<Arc<PreviewImages>> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let images = entry.1.clone();
        self.entries.push_back(entry);
        Some(images)
    }
    pub fn insert(&mut self, key: PreviewKey, images: Arc<PreviewImages>) {
        if let Some(index) = self.entries.iter().position(|(k, _)| k == &key) {
            self.bytes -= self.entries.remove(index).unwrap().1.bytes();
        }
        let bytes = images.bytes();
        if bytes > self.limit {
            return;
        }
        while self.bytes + bytes > self.limit {
            self.bytes -= self.entries.pop_front().unwrap().1.bytes();
        }
        self.bytes += bytes;
        self.entries.push_back((key, images));
    }
    pub fn remove_directory(&mut self, directory: &Path) {
        self.entries.retain(|(key, images)| {
            if key.directory == directory {
                self.bytes -= images.bytes();
                false
            } else {
                true
            }
        });
    }
}

/// Decode off the UI thread. Only a cache miss needs disk access or resizing.
pub fn load(
    directory: PathBuf,
    pages: Vec<PreviewPage>,
    page: usize,
    cancel: Arc<AtomicBool>,
) -> Result<Arc<PreviewImages>> {
    static DECODE: Mutex<()> = Mutex::new(());
    let _decode = DECODE.lock().unwrap_or_else(|e| e.into_inner());
    ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
    fn display(image: image::DynamicImage) -> Arc<RenderImage> {
        let mut pixels = image.into_rgba8();
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
    }
    let active = pages.get(page).context("Preview page is unavailable")?;
    let decoded = image::open(directory.join(&active.file))?;
    ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
    // Reuse the decoded active page for its thumbnail instead of reading it twice.
    let thumbnail = display(decoded.thumbnail(120, 176));
    let image = display(if decoded.width() <= 2048 && decoded.height() <= 2048 {
        decoded
    } else {
        decoded.thumbnail(2048, 2048)
    });
    let start = page.saturating_sub(2).min(pages.len().saturating_sub(5));
    let mut thumbnails = Vec::new();
    for (i, p) in pages.iter().enumerate().skip(start).take(5) {
        ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
        let small = if i == page {
            thumbnail.clone()
        } else {
            display(image::open(directory.join(&p.file))?.thumbnail(120, 176))
        };
        thumbnails.push((i, small));
    }
    ensure!(!cancel.load(Ordering::Acquire), "Preview cancelled");
    Ok(Arc::new(PreviewImages {
        page: active.clone(),
        image,
        thumbnails,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str) -> PreviewPage {
        PreviewPage {
            file: name.into(),
            width: 8,
            height: 4,
            width_mm: 1.,
            height_mm: 0.5,
        }
    }
    fn key(directory: &str, page: usize) -> PreviewKey {
        PreviewKey {
            directory: directory.into(),
            page,
        }
    }
    fn images() -> Arc<PreviewImages> {
        Arc::new(PreviewImages {
            page: page("page-1.png"),
            image: Arc::new(RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::new(8, 4),
            )])),
            thumbnails: Vec::new(),
        })
    }
    #[test]
    fn switching_tabs_reuses_pixels_without_reading_pngs_again() {
        let dir = tempfile::tempdir().unwrap();
        let page = page("page.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([20, 40, 60, 255]))
            .save(dir.path().join(&page.file))
            .unwrap();
        let decoded = load(
            dir.path().into(),
            vec![page],
            0,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(&decoded.image.as_bytes(0).unwrap()[..4], &[60, 40, 20, 255]);
        assert_eq!(decoded.image.as_bytes(0).unwrap().len(), 8 * 4 * 4);
        assert_eq!(decoded.thumbnails.len(), 1);
        let cached_key = PreviewKey {
            directory: dir.path().into(),
            page: 0,
        };
        let mut cache = PreviewCache::default();
        cache.insert(cached_key.clone(), decoded.clone());
        // A tab switch must have a synchronous hit even when the source PNG is unavailable.
        std::fs::remove_file(dir.path().join("page.png")).unwrap();
        for _ in 0..100 {
            let hit = cache.get(&cached_key).unwrap();
            assert!(Arc::ptr_eq(&decoded, &hit));
            assert_eq!(hit.page.width_mm, 1.);
        }
        assert!(cache
            .get(&PreviewKey {
                page: 1,
                ..cached_key.clone()
            })
            .is_none());
        assert!(cache.get(&key("new-revision", 0)).is_none());
        cache.remove_directory(dir.path());
        assert!(cache.get(&cached_key).is_none());
        assert_eq!(cache.bytes, 0);
    }
    #[test]
    fn byte_budget_evicts_least_recent_tab_and_does_not_pin_closed_documents() {
        let image = images();
        let mut cache = PreviewCache::new(image.bytes() * 2);
        cache.insert(key("a", 0), image.clone());
        cache.insert(key("b", 0), image.clone());
        assert!(cache.get(&key("a", 0)).is_some());
        cache.insert(key("c", 0), image.clone());
        assert!(cache.get(&key("b", 0)).is_none());
        assert!(cache.get(&key("a", 0)).is_some());
        // Replacing a key doesn't double-charge its bytes.
        cache.insert(key("a", 0), image.clone());
        assert_eq!(cache.bytes, image.bytes() * 2);
        cache.remove_directory(Path::new("a"));
        assert_eq!(cache.bytes, image.bytes());
        let mut tiny = PreviewCache::new(image.bytes() - 1);
        tiny.insert(key("huge", 0), image);
        assert_eq!(tiny.bytes, 0);
    }
    #[test]
    fn cancelled_load_never_decodes_or_returns_an_image() {
        assert!(load(
            "missing".into(),
            vec![page("page.png")],
            0,
            Arc::new(AtomicBool::new(true))
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    }
}
