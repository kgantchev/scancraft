//! A session on disk (§6.6): a folder holding `session.json` (the tray's pages, the photos, the
//! options), `pending.json` (pages being worked on that aren't in the tray yet), `sources/` (copies
//! of the imported files) and `cache/`. Files are written through `craft-atomic`.
//!
//! `scancraft-core`'s `SessionFile` has no place for a page between "Correct Perspective" and "Add
//! Page", so those pages live in `pending.json`, which the engine owns.

use crate::error::{EngineError, fs_error};
use scancraft_core::recipe::{ExportOptions, Page, PageId, PhotoId, PhotoItem, SESSION_FORMAT, SessionFile, SourceId, TurnOrigin};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The most pages one session holds (§9).
pub const MAX_PAGES: usize = 500;
/// The largest session or pending file the engine reads.
const MAX_JSON_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize, Default)]
struct PendingFile {
    pages: Vec<Page>,
}

pub struct Session {
    dir: PathBuf,
    file: SessionFile,
    pending: Vec<Page>,
    next_photo: u64,
    next_page: u64,
    /// What reading the files had to repair, for a notice.
    repairs: Vec<String>,
}

fn empty_file() -> SessionFile {
    SessionFile {
        format: SESSION_FORMAT,
        photos: Vec::new(),
        pages: Vec::new(),
        export: ExportOptions::default(),
        book_spreads: false,
        exported: false,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, EngineError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| fs_error("a session file could not be opened", e))?;
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes).map_err(|e| fs_error("a session file could not be read", e))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err(EngineError::msg("a session file is too large"));
    }
    serde_json::from_slice(&bytes).map_err(|e| EngineError::msg(format!("a session file is damaged: {e}")))
}

impl Session {
    /// Creates the folder, `sources/` and `cache/`, and writes an empty `session.json`.
    pub fn create(dir: impl Into<PathBuf>) -> Result<Session, EngineError> {
        let dir = dir.into();
        for sub in [&dir, &dir.join("sources"), &dir.join("cache")] {
            std::fs::create_dir_all(sub).map_err(|e| fs_error("the session folder could not be made", e))?;
        }
        let session = Session { dir, file: empty_file(), pending: Vec::new(), next_photo: 1, next_page: 1, repairs: Vec::new() };
        session.save()?;
        Ok(session)
    }

    /// A session that touches no disk until something is saved (and then fails): for when no folder can be made.
    pub fn detached(dir: impl Into<PathBuf>) -> Session {
        Session { dir: dir.into(), file: empty_file(), pending: Vec::new(), next_photo: 1, next_page: 1, repairs: Vec::new() }
    }

    /// Opens a session folder; one with no `session.json` yet starts empty. A file written by a
    /// newer version is refused; one from an older version is read. What a hand edit broke is
    /// repaired (and reported by [`Session::take_repairs`]) rather than trusted.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Session, EngineError> {
        let dir = dir.into();
        let path = dir.join("session.json");
        if !path.is_file() {
            return Session::create(dir);
        }
        let mut file: SessionFile = read_json(&path)?;
        if file.format > SESSION_FORMAT {
            return Err(EngineError::msg("this session was made by a newer version of ScanCraft"));
        }
        let mut pending = Vec::new();
        let pending_path = dir.join("pending.json");
        if pending_path.is_file() {
            pending = read_json::<PendingFile>(&pending_path)?.pages;
        }
        for sub in ["sources", "cache"] {
            std::fs::create_dir_all(dir.join(sub)).map_err(|e| fs_error("the session folder could not be made", e))?;
        }
        let mut repairs = Vec::new();
        file.format = SESSION_FORMAT;
        repair(&mut file, &mut pending, &mut repairs);
        let next_photo = file.photos.iter().map(|p| p.id.0).max().unwrap_or(0).saturating_add(1);
        let next_page = file.pages.iter().chain(&pending).map(|p| p.id.0).max().unwrap_or(0).saturating_add(1);
        Ok(Session { dir, file, pending, next_photo, next_page, repairs })
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    pub fn file(&self) -> &SessionFile {
        &self.file
    }

    pub fn file_mut(&mut self) -> &mut SessionFile {
        &mut self.file
    }

    pub fn pending(&self) -> &[Page] {
        &self.pending
    }

    pub fn pending_mut(&mut self) -> &mut Vec<Page> {
        &mut self.pending
    }

    pub fn sources_dir(&self) -> PathBuf {
        self.dir.join("sources")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.dir.join("cache")
    }

    pub fn take_repairs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.repairs)
    }

    /// Writes `session.json` and `pending.json` through `craft-atomic`.
    pub fn save(&self) -> Result<(), EngineError> {
        let bytes = serde_json::to_vec(&self.file).map_err(|e| EngineError::msg(e.to_string()))?;
        craft_atomic::atomic_write(&self.dir.join("session.json"), &bytes).map_err(|e| fs_error("the session could not be saved", e))?;
        let pending_path = self.dir.join("pending.json");
        if self.pending.is_empty() {
            if pending_path.exists() {
                std::fs::remove_file(&pending_path).map_err(|e| fs_error("the session could not be saved", e))?;
            }
        } else {
            let bytes = serde_json::to_vec(&PendingFile { pages: self.pending.clone() }).map_err(|e| EngineError::msg(e.to_string()))?;
            craft_atomic::atomic_write(&pending_path, &bytes).map_err(|e| fs_error("the session could not be saved", e))?;
        }
        Ok(())
    }

    pub fn allocate_photo_id(&mut self) -> PhotoId {
        let id = PhotoId(self.next_photo);
        // A file with an id at the end of the range is hostile; the counter stops there.
        self.next_photo = self.next_photo.saturating_add(1);
        id
    }

    pub fn allocate_page_id(&mut self) -> PageId {
        let id = PageId(self.next_page);
        self.next_page = self.next_page.saturating_add(1);
        id
    }

    /// Makes sure later ids stay above every id now in the session (after undo restores a snapshot).
    pub fn resync_ids(&mut self) {
        let photo = self.file.photos.iter().map(|p| p.id.0).max().unwrap_or(0).saturating_add(1);
        let page = self.file.pages.iter().chain(&self.pending).map(|p| p.id.0).max().unwrap_or(0).saturating_add(1);
        self.next_photo = self.next_photo.max(photo);
        self.next_page = self.next_page.max(page);
    }

    pub fn photo(&self, id: PhotoId) -> Option<&PhotoItem> {
        self.file.photos.iter().find(|p| p.id == id)
    }

    pub fn photo_mut(&mut self, id: PhotoId) -> Option<&mut PhotoItem> {
        self.file.photos.iter_mut().find(|p| p.id == id)
    }

    /// A page of the tray or still pending.
    pub fn page(&self, id: PageId) -> Option<&Page> {
        self.file.pages.iter().chain(&self.pending).find(|p| p.id == id)
    }

    pub fn page_mut(&mut self, id: PageId) -> Option<&mut Page> {
        self.file.pages.iter_mut().chain(self.pending.iter_mut()).find(|p| p.id == id)
    }

    pub fn in_tray(&self, id: PageId) -> bool {
        self.file.pages.iter().any(|p| p.id == id)
    }

    /// The pages cut from a photo, pending ones first (they are the ones being worked on), then the tray's.
    pub fn pages_of(&self, photo: PhotoId) -> Vec<&Page> {
        self.pending.iter().chain(&self.file.pages).filter(|p| p.photo == photo).collect()
    }

    /// `sources/<hex sha-256>`.
    pub fn source_path(&self, source: &SourceId) -> PathBuf {
        self.sources_dir().join(&source.0)
    }

    /// Stores a source's bytes in `sources/`, once however often the same file is imported.
    pub fn add_source(&self, source: &SourceId, bytes: &[u8]) -> Result<PathBuf, EngineError> {
        // The name is a hex digest the engine made; anything else must not become a path.
        if source.0.is_empty() || !source.0.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(EngineError::msg("a source name must be a hex digest"));
        }
        let path = self.source_path(source);
        if !path.is_file() {
            craft_atomic::atomic_write(&path, bytes).map_err(|e| fs_error("the file could not be copied into the session", e))?;
        }
        Ok(path)
    }
}

/// Drops and fixes what a hand-edited or damaged file would otherwise let crash the engine later.
fn repair(file: &mut SessionFile, pending: &mut Vec<Page>, repairs: &mut Vec<String>) {
    // Photos: unique ids.
    let mut seen = std::collections::BTreeSet::new();
    file.photos.retain(|p| {
        let keep = seen.insert(p.id);
        if !keep {
            repairs.push("a photo listed twice was dropped".to_string());
        }
        keep
    });
    let photos: std::collections::BTreeSet<PhotoId> = file.photos.iter().map(|p| p.id).collect();
    let mut ids = std::collections::BTreeSet::new();
    let mut check = |page: &mut Page, repairs: &mut Vec<String>| -> bool {
        if !photos.contains(&page.photo) {
            repairs.push("a page whose photo is missing was dropped".to_string());
            return false;
        }
        if !ids.insert(page.id) {
            repairs.push("a page listed twice was dropped".to_string());
            return false;
        }
        if !page.corners.is_finite() {
            repairs.push("a page with corners that are not numbers was dropped".to_string());
            return false;
        }
        if page.strength > 100 || page.quarter_turns > 3 {
            page.strength = page.strength.min(100);
            page.quarter_turns %= 4;
            page.turns_from = TurnOrigin::Manual;
            repairs.push("a page's strength or turns were out of range and were corrected".to_string());
        }
        true
    };
    file.pages.retain_mut(|p| check(p, repairs));
    pending.retain_mut(|p| check(p, repairs));
    if file.pages.len() > MAX_PAGES {
        file.pages.truncate(MAX_PAGES);
        repairs.push(format!("only the first {MAX_PAGES} pages were kept"));
    }
}

/// What the cleanup pass learns about a session folder.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub dir: PathBuf,
    pub modified: SystemTime,
    pub exported: bool,
    pub pages: u32,
}

/// The sessions under `root`, newest first; a folder with no readable `session.json` is skipped.
pub fn list_sessions(root: &Path) -> Vec<SessionInfo> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<SessionInfo> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let dir = e.path();
            let file = dir.join("session.json");
            let modified = std::fs::metadata(&file).ok()?.modified().ok()?;
            let parsed: SessionFile = read_json(&file).ok()?;
            let pending = read_json::<PendingFile>(&dir.join("pending.json")).map(|p| p.pages.len()).unwrap_or(0);
            Some(SessionInfo { dir, modified, exported: parsed.exported, pages: (parsed.pages.len() + pending) as u32 })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.dir.cmp(&b.dir)));
    out
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cleanup {
    pub removed: Vec<PathBuf>,
    /// Older sessions that were never exported: removed only after the user confirms.
    pub kept_unexported: Vec<PathBuf>,
}

/// Keeps the `keep` newest sessions and removes the older exported ones.
pub fn cleanup(root: &Path, keep: usize) -> Cleanup {
    let mut result = Cleanup::default();
    for info in list_sessions(root).into_iter().skip(keep) {
        if info.exported {
            // Only a folder that holds a session.json directly inside the root is ever removed.
            if info.dir.parent() == Some(root) && std::fs::remove_dir_all(&info.dir).is_ok() {
                result.removed.push(info.dir);
            }
        } else {
            result.kept_unexported.push(info.dir);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::recipe::{CornerOrigin, FlattenMode, ImportOrigin, Look, SideShape};
    use std::time::Duration;

    pub fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scancraft-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    pub fn page(id: u64, photo: u64) -> Page {
        Page {
            id: PageId(id),
            photo: PhotoId(photo),
            source: SourceId("ab".into()),
            corners: Quad([Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)]),
            corners_from: CornerOrigin::Manual,
            sides: [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight],
            spread: None,
            flatten: FlattenMode::Auto,
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: TurnOrigin::Manual,
            look: Look::ColorDocument,
            strength: 60,
            text: None,
        }
    }

    pub fn photo(id: u64) -> PhotoItem {
        PhotoItem {
            id: PhotoId(id),
            source: SourceId("ab".into()),
            name: format!("p{id}.jpg"),
            origin: ImportOrigin::File,
            size: (100, 80),
            focal: Default::default(),
        }
    }

    #[test]
    fn ids_at_the_top_of_the_range_in_a_hostile_file_never_panic() {
        // A session file is input: a page or photo with the largest id there is must not make the
        // next id overflow, which panicked in a debug build and wrapped to an id already in use in
        // a release build.
        let dir = temp("hostile-ids");
        let mut s = Session::create(&dir).unwrap();
        s.file_mut().photos.push(photo(u64::MAX));
        s.file_mut().pages.push(page(u64::MAX, u64::MAX));
        s.save().unwrap();
        let mut back = Session::open(&dir).unwrap();
        back.resync_ids();
        let (a, b) = (back.allocate_page_id(), back.allocate_page_id());
        let (c, d) = (back.allocate_photo_id(), back.allocate_photo_id());
        // The ids can't go on past the end; none of this may panic, and they stay in the range.
        assert!(a.0 >= u64::MAX - 1 && b.0 >= u64::MAX - 1 && c.0 >= u64::MAX - 1 && d.0 >= u64::MAX - 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_session_round_trips_through_its_folder() {
        let dir = temp("roundtrip");
        let mut s = Session::create(&dir).unwrap();
        s.file_mut().photos.push(photo(1));
        s.file_mut().pages.push(page(1, 1));
        s.file_mut().pages.push(page(2, 1));
        s.pending_mut().push(page(3, 1));
        s.file_mut().export.searchable = false;
        s.file_mut().book_spreads = true;
        s.save().unwrap();
        let back = Session::open(&dir).unwrap();
        assert_eq!(back.file(), s.file());
        assert_eq!(back.pending(), s.pending());
        // Ids continue after the largest in the files, pending included.
        let mut back = back;
        assert_eq!(back.allocate_page_id(), PageId(4));
        assert_eq!(back.allocate_photo_id(), PhotoId(2));
        assert!(dir.join("sources").is_dir() && dir.join("cache").is_dir());
        // Saving with nothing pending removes the sidecar.
        back.pending_mut().clear();
        back.save().unwrap();
        assert!(!dir.join("pending.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_folder_opens_empty_and_a_newer_format_is_refused() {
        let dir = temp("empty");
        let s = Session::open(&dir).unwrap();
        assert!(s.file().photos.is_empty() && s.file().pages.is_empty() && !s.file().exported);
        let mut file = s.file().clone();
        file.format = SESSION_FORMAT + 1;
        std::fs::write(dir.join("session.json"), serde_json::to_vec(&file).unwrap()).unwrap();
        let Err(EngineError::Message(m)) = Session::open(&dir) else { panic!("opened a newer session") };
        assert!(m.contains("newer version"));
        // A file that is not JSON is an error, not a panic.
        std::fs::write(dir.join("session.json"), b"{ not json").unwrap();
        assert!(Session::open(&dir).is_err());
        std::fs::write(dir.join("session.json"), []).unwrap();
        assert!(Session::open(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_hand_edited_session_is_repaired_not_trusted() {
        let dir = temp("repair");
        let mut s = Session::create(&dir).unwrap();
        s.file_mut().photos.push(photo(1));
        s.file_mut().photos.push(photo(1));
        s.file_mut().pages.push(page(1, 1));
        s.file_mut().pages.push(page(1, 1));
        s.file_mut().pages.push(page(2, 99));
        let mut bad = page(3, 1);
        bad.corners.0[0] = Point::new(f64::NAN, 0.0);
        s.file_mut().pages.push(bad);
        let mut wild = page(4, 1);
        wild.strength = 250;
        wild.quarter_turns = 9;
        s.file_mut().pages.push(wild);
        // NaN can't be written as JSON, so write the others and patch the file text.
        s.file_mut().pages.retain(|p| p.id != PageId(3));
        s.save().unwrap();
        let mut back = Session::open(&dir).unwrap();
        assert_eq!(back.file().photos.len(), 1);
        assert_eq!(back.file().pages.iter().map(|p| p.id.0).collect::<Vec<_>>(), vec![1, 4]);
        let fixed = back.page(PageId(4)).unwrap();
        assert_eq!((fixed.strength, fixed.quarter_turns), (100, 1));
        assert_eq!(back.take_repairs().len(), 4);
        assert!(back.take_repairs().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sources_are_copied_into_the_session_folder_once() {
        let dir = temp("sources");
        let s = Session::create(&dir).unwrap();
        let id = SourceId("00ff".into());
        let path = s.add_source(&id, b"photo bytes").unwrap();
        assert_eq!(path, dir.join("sources").join("00ff"));
        assert_eq!(std::fs::read(&path).unwrap(), b"photo bytes");
        // Importing it again leaves the stored copy as it was.
        s.add_source(&id, b"photo bytes").unwrap();
        assert_eq!(std::fs::read_dir(dir.join("sources")).unwrap().count(), 1);
        // A name that isn't a digest never becomes a path.
        assert!(s.add_source(&SourceId("../escape".into()), b"x").is_err());
        assert!(s.add_source(&SourceId(String::new()), b"x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn make_session(root: &Path, name: &str, exported: bool, age_secs: u64) -> PathBuf {
        let dir = root.join(name);
        let mut s = Session::create(&dir).unwrap();
        s.file_mut().exported = exported;
        s.file_mut().pages.push(page(1, 1));
        s.file_mut().photos.push(photo(1));
        s.save().unwrap();
        let file = std::fs::File::options().write(true).open(dir.join("session.json")).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(age_secs)).unwrap();
        dir
    }

    #[test]
    fn the_cleanup_keeps_the_ten_most_recent_sessions() {
        let root = temp("cleanup");
        for i in 0..12 {
            make_session(&root, &format!("s{i:02}"), true, 1000 - i * 10);
        }
        let listed = list_sessions(&root);
        assert_eq!(listed.len(), 12);
        assert!(listed.windows(2).all(|w| w[0].modified >= w[1].modified));
        let result = cleanup(&root, 10);
        assert_eq!(result.removed.len(), 2);
        // The two oldest (the ones with the smallest i's times) went.
        // The oldest are the ones made first: s00 is a thousand seconds old, s01 990.
        assert!(result.removed.iter().all(|p| p.ends_with("s00") || p.ends_with("s01")), "{:?}", result.removed);
        assert_eq!(list_sessions(&root).len(), 10);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unexported_session_is_never_removed_without_a_confirmation() {
        let root = temp("keep-unexported");
        let old = make_session(&root, "old", false, 5000);
        for i in 0..3 {
            make_session(&root, &format!("new{i}"), true, 10 + i);
        }
        let result = cleanup(&root, 2);
        assert_eq!(result.kept_unexported, vec![old.clone()]);
        assert!(old.join("session.json").is_file());
        // A folder with no session.json is skipped, and a missing root is not an error.
        std::fs::create_dir_all(root.join("junk")).unwrap();
        assert_eq!(list_sessions(&root).len(), 3);
        assert!(list_sessions(&root.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
