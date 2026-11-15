//! Cached results (§6.5): files in the session folder named by the SHA-256 of exactly the inputs
//! the specification lists, and at most two decoded full-resolution photos in memory.
//!
//! The key is a pure function of the recipe, the source hash, the output DPI and `ALGO_VERSION`, so
//! a result made by an older algorithm is simply never looked up again.

use crate::error::{EngineError, fs_error};
use scancraft_core::ALGO_VERSION;
use scancraft_core::geom::Point;
use scancraft_core::image::{Plane, Rgb8};
use scancraft_core::recipe::{BendMode, FlattenMode, Half, Page, SideShape, SourceId};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The most decoded full-resolution photos kept in memory.
pub const MAX_DECODED_PHOTOS: usize = 2;

/// What a cached file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    DisplayCopy,
    LookPreview,
    FilterThumb,
    TrayThumb,
    FinalPage,
}

impl CacheKind {
    fn tag(self) -> u8 {
        match self {
            CacheKind::DisplayCopy => b'd',
            CacheKind::LookPreview => b'p',
            CacheKind::FilterThumb => b'f',
            CacheKind::TrayThumb => b't',
            CacheKind::FinalPage => b'P',
        }
    }
}

/// A point as two `i64`s of 1/64 px.
fn push_point(buf: &mut Vec<u8>, p: Point) {
    for v in [p.x, p.y] {
        buf.extend(((v * 64.0).round() as i64).to_le_bytes());
    }
}

fn push_side(buf: &mut Vec<u8>, side: &SideShape) {
    match side {
        SideShape::Straight => buf.push(b's'),
        SideShape::Spine => buf.push(b'S'),
        SideShape::Bent { handles } => {
            buf.push(b'b');
            handles.iter().for_each(|h| push_point(buf, *h));
        }
        SideShape::Traced { samples, .. } => {
            buf.push(b't');
            buf.extend((samples.len() as u32).to_le_bytes());
            for s in samples {
                push_point(buf, s.point);
                buf.extend(s.weight.to_bits().to_le_bytes());
            }
        }
    }
}

/// The SHA-256 of exactly the inputs §6.5 lists, as 64 hex characters.
///
/// In order: a version tag, the kind, the source hash, the four corners and four sides at 1/64 px,
/// the spread's half and spine, the Flatten setting, the fitted sheet, the quarter turns, the look,
/// the strength, the DPI and `ALGO_VERSION`.
pub fn cache_key(page: &Page, dpi: u32, kind: CacheKind) -> String {
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    buf.extend(b"scancraft-cache-v1");
    buf.push(kind.tag());
    buf.extend(page.source.0.as_bytes());
    page.corners.0.iter().for_each(|c| push_point(&mut buf, *c));
    page.sides.iter().for_each(|s| push_side(&mut buf, s));
    match &page.spread {
        None => buf.push(0),
        Some(s) => {
            buf.push(1);
            buf.push(if s.half == Half::Left { b'L' } else { b'R' });
            s.spine.iter().for_each(|p| push_point(&mut buf, *p));
        }
    }
    buf.push(match page.flatten {
        FlattenMode::Auto => b'A',
        FlattenMode::On => b'O',
        FlattenMode::Off => b'F',
    });
    match &page.sheet {
        None => buf.push(0),
        Some(fit) => {
            buf.push(1);
            buf.push(if fit.sheet.mode == BendMode::Across { b'a' } else { b'd' });
            let s = &fit.sheet;
            for v in s.rotation.iter().chain(&s.translation).chain(std::iter::once(&s.chord)).chain(&s.profile).chain(std::iter::once(&s.focal)) {
                buf.extend(v.to_bits().to_le_bytes());
            }
        }
    }
    buf.push(page.quarter_turns);
    buf.push(page.look as u8);
    buf.push(page.strength);
    buf.extend(dpi.to_le_bytes());
    buf.extend(ALGO_VERSION.to_le_bytes());
    Sha256::digest(&buf).iter().map(|b| format!("{b:02x}")).collect()
}

/// The key of a photo's display copy: the photo and its source, nothing of any page.
pub fn display_key(source: &SourceId, origin_index: u32) -> String {
    let mut buf = b"scancraft-cache-v1d".to_vec();
    buf.extend(source.0.as_bytes());
    buf.extend(origin_index.to_le_bytes());
    buf.extend(ALGO_VERSION.to_le_bytes());
    Sha256::digest(&buf).iter().map(|b| format!("{b:02x}")).collect()
}

/// Which decoded photo: a source file and the page or frame of it.
pub type PhotoKey = (SourceId, u32);

/// The session folder's cache and the decoded-photo LRU; shared with the jobs.
pub struct Cache {
    dir: PathBuf,
    photos: Mutex<Vec<(PhotoKey, Arc<Rgb8>)>>,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Result<Cache, EngineError> {
        std::fs::create_dir_all(&dir).map_err(|e| fs_error("the cache folder could not be made", e))?;
        Ok(Cache { dir, photos: Mutex::new(Vec::new()) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        // Keys are hex digests the engine makes; refuse anything else.
        (!key.is_empty() && key.bytes().all(|b| b.is_ascii_hexdigit())).then(|| self.dir.join(format!("{key}.rgb")))
    }

    /// Reads a cached image: an 8-byte header (width, height, little-endian), then RGB bytes.
    pub fn get(&self, key: &str) -> Option<Arc<Rgb8>> {
        let bytes = std::fs::read(self.path(key)?).ok()?;
        let (head, body) = (bytes.get(..8)?, bytes.get(8..)?);
        let w = u32::from_le_bytes(head[..4].try_into().ok()?);
        let h = u32::from_le_bytes(head[4..].try_into().ok()?);
        // A header that promises more than the file holds is a damaged file, not an allocation.
        if (w as usize).checked_mul(h as usize)?.checked_mul(3)? != body.len() {
            return None;
        }
        let (pixels, _) = body.as_chunks::<3>();
        Plane::from_vec(w, h, pixels.to_vec()).ok().map(Arc::new)
    }

    /// Writes a cached image through `craft-atomic`.
    pub fn put(&self, key: &str, image: &Rgb8) -> Result<(), EngineError> {
        let path = self.path(key).ok_or_else(|| EngineError::msg("a cache key must be a hex digest"))?;
        let mut bytes = Vec::with_capacity(8 + image.data().len() * 3);
        bytes.extend(image.width().to_le_bytes());
        bytes.extend(image.height().to_le_bytes());
        bytes.extend(image.data().iter().flatten());
        craft_atomic::atomic_write(&path, &bytes).map_err(|e| fs_error("the cache could not be written", e))
    }

    /// Writes any small piece of data (a detection result) under a key.
    pub fn put_bytes(&self, key: &str, bytes: &[u8]) -> Result<(), EngineError> {
        let path = self.path(key).ok_or_else(|| EngineError::msg("a cache key must be a hex digest"))?.with_extension("json");
        craft_atomic::atomic_write(&path, bytes).map_err(|e| fs_error("the cache could not be written", e))
    }

    pub fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)?.with_extension("json")).ok()
    }

    /// Removes every cached file.
    pub fn clear(&self) -> Result<(), EngineError> {
        for entry in std::fs::read_dir(&self.dir).map_err(|e| fs_error("the cache could not be cleared", e))?.filter_map(Result::ok) {
            if entry.path().is_file() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        if let Ok(mut photos) = self.photos.lock() {
            photos.clear();
        }
        Ok(())
    }

    /// The decoded photo, moved to the front of the LRU.
    pub fn photo(&self, key: &PhotoKey) -> Option<Arc<Rgb8>> {
        let mut photos = self.photos.lock().ok()?;
        let at = photos.iter().position(|(k, _)| k == key)?;
        let entry = photos.remove(at);
        let image = entry.1.clone();
        photos.push(entry);
        Some(image)
    }

    /// Keeps a decoded photo, dropping the least recently used past the limit.
    pub fn put_photo(&self, key: PhotoKey, image: Arc<Rgb8>) {
        if let Ok(mut photos) = self.photos.lock() {
            photos.retain(|(k, _)| *k != key);
            photos.push((key, image));
            while photos.len() > MAX_DECODED_PHOTOS {
                photos.remove(0);
            }
        }
    }

    pub fn decoded_photos(&self) -> usize {
        self.photos.lock().map_or(0, |p| p.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::geom::Quad;
    use scancraft_core::recipe::{CornerOrigin, Look, PageId, PhotoId, Sheet, SheetFit, SpreadHalf, TurnOrigin};

    fn page() -> Page {
        Page {
            id: PageId(1),
            photo: PhotoId(1),
            source: SourceId("abcd".into()),
            corners: Quad([Point::new(10.0, 10.0), Point::new(90.0, 12.0), Point::new(88.0, 120.0), Point::new(8.0, 118.0)]),
            corners_from: CornerOrigin::Manual,
            sides: [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight],
            spread: None,
            flatten: FlattenMode::Auto,
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: TurnOrigin::Auto,
            look: Look::ColorDocument,
            strength: 60,
            text: None,
        }
    }

    fn key(p: &Page) -> String {
        cache_key(p, 300, CacheKind::FinalPage)
    }

    fn sheet() -> SheetFit {
        use scancraft_core::camera::FocalSource;
        use scancraft_core::recipe::{FlattenDecision, FlattenReason};
        SheetFit {
            sheet: Sheet {
                mode: BendMode::Across,
                rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                translation: [0.0, 0.0, 1.5],
                chord: 0.7,
                profile: [0.01; 10],
                focal: 0.8,
                focal_source: FocalSource::Fitted,
                cost: 3.0,
            },
            decision: FlattenDecision {
                flattened: true,
                reason: FlattenReason::Straighter,
                d95: 0.01,
                plane_cost: 5.0,
                curved_cost: 3.0,
                straightness_corners: None,
                straightness_flat: None,
                lines_corners: 0,
                lines_flat: 0,
            },
        }
    }

    #[test]
    fn the_cache_key_changes_with_every_input() {
        let base = key(&page());
        assert_eq!(base.len(), 64);
        assert_eq!(base, key(&page()));
        let mut seen = std::collections::BTreeSet::from([base.clone()]);
        let mut variants: Vec<String> = Vec::new();
        let mut with = |f: &dyn Fn(&mut Page)| {
            let mut p = page();
            f(&mut p);
            variants.push(key(&p));
        };
        with(&|p| p.source = SourceId("abce".into()));
        with(&|p| p.corners.0[2].x += 1.0);
        with(&|p| p.sides[1] = SideShape::Bent { handles: [Point::new(1.0, 1.0); 3] });
        with(&|p| p.sides[3] = SideShape::Spine);
        with(&|p| p.sides[0] = SideShape::Traced { samples: vec![], bow: 0.0, curved: false });
        with(&|p| {
            p.spread = Some(SpreadHalf {
                half: Half::Left,
                spine: [Point::new(1.0, 0.0), Point::new(1.0, 9.0)],
                spread_corners: page().corners,
                confidence: 1.0,
            })
        });
        with(&|p| p.flatten = FlattenMode::On);
        with(&|p| p.sheet = Some(sheet()));
        with(&|p| p.quarter_turns = 1);
        with(&|p| p.look = Look::Grayscale);
        with(&|p| p.strength = 61);
        variants.push(cache_key(&page(), 200, CacheKind::FinalPage));
        variants.push(cache_key(&page(), 300, CacheKind::LookPreview));
        for v in variants {
            assert!(seen.insert(v.clone()), "two inputs gave the key {v}");
        }
        // A different sheet parameter changes the key too, and the spread's other half does.
        let (mut a, mut b) = (page(), page());
        a.sheet = Some(sheet());
        let mut s2 = sheet();
        s2.sheet.profile[4] += 0.001;
        b.sheet = Some(s2);
        assert_ne!(key(&a), key(&b));
        // Things that are not inputs don't change it: the id, the recognised text, how the corners came.
        let mut c = page();
        c.id = PageId(7);
        c.corners_from = CornerOrigin::NotFound;
        c.text = Some(scancraft_core::recipe::RecognizedText { engine: "x".into(), engine_version: "1".into(), lines: vec![] });
        assert_eq!(key(&c), base);
    }

    #[test]
    fn the_corners_are_rounded_to_a_sixty_fourth_of_a_pixel() {
        let base = key(&page());
        let mut a = page();
        a.corners.0[0].x += 1.0 / 128.0 - 1e-6;
        assert_eq!(key(&a), base);
        let mut b = page();
        b.corners.0[0].x += 1.0 / 64.0;
        assert_ne!(key(&b), base);
    }

    #[test]
    fn a_cached_image_comes_back_unchanged() {
        let dir = std::env::temp_dir().join(format!("scancraft-engine-cache-{}", std::process::id()));
        let cache = Cache::new(dir.clone()).unwrap();
        let image: Rgb8 = Plane::from_vec(3, 2, (0..6u8).map(|i| [i, i * 2, 255 - i]).collect()).unwrap();
        let k = key(&page());
        assert!(cache.get(&k).is_none());
        cache.put(&k, &image).unwrap();
        assert_eq!(*cache.get(&k).unwrap(), image);
        // A damaged file and a key that isn't a digest are misses, not panics.
        std::fs::write(dir.join(format!("{k}.rgb")), [1, 0, 0, 0, 255, 255, 255, 255, 9]).unwrap();
        assert!(cache.get(&k).is_none());
        assert!(cache.get("../escape").is_none() && cache.get("").is_none());
        assert!(cache.put("../escape", &image).is_err());
        cache.put_bytes(&k, b"{}").unwrap();
        assert_eq!(cache.get_bytes(&k).unwrap(), b"{}");
        cache.clear().unwrap();
        assert!(cache.get_bytes(&k).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cache_holds_two_photos_and_drops_the_oldest() {
        let dir = std::env::temp_dir().join(format!("scancraft-engine-lru-{}", std::process::id()));
        let cache = Cache::new(dir.clone()).unwrap();
        let img = |v: u8| Arc::new(Plane::new(2, 2, [v, v, v]).unwrap());
        let k = |s: &str| (SourceId(s.to_string()), 0u32);
        cache.put_photo(k("a"), img(1));
        cache.put_photo(k("b"), img(2));
        // Using "a" makes "b" the least recently used.
        assert!(cache.photo(&k("a")).is_some());
        cache.put_photo(k("c"), img(3));
        assert_eq!(cache.decoded_photos(), 2);
        assert!(cache.photo(&k("b")).is_none());
        assert!(cache.photo(&k("a")).is_some() && cache.photo(&k("c")).is_some());
        // The same source's other page is a different photo.
        assert!(cache.photo(&(SourceId("a".into()), 1)).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
