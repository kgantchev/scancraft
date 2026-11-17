//! ScanCraft engine (specification §6.2–§6.6, L3).
//!
//! The engine owns one session: the photos and pages (the recipes), the undo stack, the job runner
//! and the caches. The interface draws from [`ViewState`] snapshots and changes things only by
//! dispatching commands (`EngineApi`). Pixel work runs in jobs; their results come back through
//! `poll`.
//!
//! Owners (build plan §4): WP6 owns everything except `flatten` (A29), which is WP11's.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cache;
mod commands;
pub mod crop;
pub mod curve;
pub mod error;
pub mod flatten;
pub mod jobs;
pub mod pipeline;
pub mod session;
pub mod settings;
pub mod undo;
mod view;
mod work;

use crate::cache::Cache;
use crate::error::EngineError;
use crate::jobs::{CancellationToken, JobRunner};
use crate::session::Session;
use crate::settings::{Settings, SettingsHolder};
use crate::undo::{Snapshot, UndoStack};
use crate::work::JobOutput;
use craft_settings::Location;
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ExportSummary, ImageKey, LoupeCrop, Notice, Screen, ViewState};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{CornerOrigin, Look, PageId, PhotoId, SideShape};
use scancraft_core::traits::{PageDetector, Recognizer, UprightVoter};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// The sessions kept by the cleanup (§6.6).
pub const KEEP_SESSIONS: usize = 10;
/// The session is saved at most this often (§6.6).
pub const SAVE_INTERVAL_MS: u64 = 1000;

/// The platform services the apps plug in (specification §6.1): Vision on macOS, ocrs in the
/// command-line tool, none in most tests.
#[derive(Clone, Default)]
pub struct Services {
    pub detector: Option<Arc<dyn PageDetector>>,
    pub recognizer: Option<Arc<dyn Recognizer>>,
    pub upright: Option<Arc<dyn UprightVoter>>,
}

/// How to open an engine.
pub struct EngineOptions {
    /// The folder of the current session; made if it doesn't exist.
    pub session_dir: PathBuf,
    pub services: Services,
    /// Where the settings live; `Location::Memory` for tests.
    pub settings: Location,
    /// Worker threads for pixel work; 0 for the number of cores.
    pub threads: usize,
    /// The folder holding every session, for restore and cleanup; by default the setting, else the
    /// parent of `session_dir`.
    pub sessions_root: Option<PathBuf>,
}

impl EngineOptions {
    pub fn new(session_dir: PathBuf, services: Services) -> EngineOptions {
        EngineOptions { session_dir, services, settings: Location::Auto, threads: 0, sessions_root: None }
    }
}

/// Where a job's token is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    Page(PageId),
    Photo(PhotoId),
    /// The live preview of a page: a newer request replaces the one before it.
    Preview(PageId),
    /// A29's decision for a page: it outlives the page's look changes, and a newer request replaces it.
    Flatten(PageId),
    /// A30's search for a photo's spine.
    Spine(PhotoId),
    /// The export.
    Export,
}

/// A photo's progress through detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhotoStatus {
    Pending,
    Ready,
}

/// The outline detection found, kept to restore with Auto-Detect.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Detected {
    pub corners: Quad,
    pub sides: [SideShape; 4],
    pub origin: CornerOrigin,
}

/// The last loupe crop, so it is cut again only when it must be (§6.5).
pub(crate) struct LoupeState {
    pub photo: PhotoId,
    pub side: f64,
    pub centre: Point,
    pub crop: LoupeCrop,
}

/// One session (§6.6).
pub struct Engine {
    pub(crate) session: Session,
    pub(crate) root: PathBuf,
    pub(crate) services: Services,
    pub(crate) settings: SettingsHolder,
    pub(crate) jobs: JobRunner<JobOutput>,
    pub(crate) cache: Arc<Cache>,
    pub(crate) undo: UndoStack,
    pub(crate) screen: Screen,
    pub(crate) current_photo: Option<PhotoId>,
    pub(crate) current_page: Option<PageId>,
    pub(crate) tokens: HashMap<Target, CancellationToken>,
    pub(crate) status: HashMap<PhotoId, PhotoStatus>,
    pub(crate) detected: HashMap<PhotoId, Detected>,
    pub(crate) images: HashMap<ImageKey, (String, Arc<Rgb8>)>,
    pub(crate) image_order: Vec<ImageKey>,
    pub(crate) inflight: HashSet<u64>,
    /// The tags of jobs that failed. An image that failed is not asked for again until its recipe
    /// changes (its tag changes with it): the interface asks for the same image every frame, and
    /// a retry on every frame raised the same error notice on every frame.
    pub(crate) failed: HashSet<u64>,
    pub(crate) pending_notices: Vec<Notice>,
    pub(crate) visible_notices: Vec<Notice>,
    /// A PDF is being written. The screen is not changed for it: the interface shows the export
    /// from the view's progress.
    pub(crate) exporting: bool,
    /// How many pages the running export has written, and how many it will write.
    pub(crate) export_done: Arc<std::sync::atomic::AtomicU32>,
    pub(crate) export_total: u32,
    /// The user asked `export.cancel`, so the export's cancellation is not the pages changing.
    pub(crate) export_cancelled: bool,
    /// The PDF the last export wrote.
    pub(crate) last_export: Option<ExportSummary>,
    /// Pages the user has looked at and left as they are, so they are no longer flagged.
    pub(crate) reviewed: HashSet<PageId>,
    /// Why a page's picture could not be made, for the page's state.
    pub(crate) page_failures: HashMap<PageId, String>,
    /// The tags of the background jobs a page has running, so the page can say it is still being
    /// prepared, and how many pages are still being read.
    pub(crate) flatten_tags: HashMap<PageId, u64>,
    pub(crate) upright_tags: HashMap<PageId, u64>,
    pub(crate) text_tags: HashMap<PageId, u64>,
    /// How many photos the import now running added, for its progress.
    pub(crate) batch_total: u32,
    pub(crate) restore_offer: Option<u32>,
    pub(crate) last_look: Look,
    pub(crate) loupe: Option<LoupeState>,
    /// Pages waiting for a fresh detection before Auto-Detect can be applied to them.
    pub(crate) want_auto: HashSet<PageId>,
    /// Counts the sessions this engine has had; what a job of an earlier one returns is ignored.
    pub(crate) epoch: u64,
    started: Instant,
    clock: Option<Box<dyn Fn() -> u64 + Send>>,
    last_save_ms: Option<u64>,
    dirty: bool,
    /// A file name that failed to import, so an error notice can name it.
    pub(crate) names: HashMap<PhotoId, String>,
}

impl Engine {
    /// Opens the session in `session_dir` (made if it doesn't exist). It cannot fail: when the
    /// folder can't be used, the engine works in a fresh one in the temporary folder and says why,
    /// and when even that can't be made it works without saving, and says so.
    pub fn new(session_dir: PathBuf, services: Services) -> Engine {
        let first = match Engine::open(EngineOptions::new(session_dir, services.clone())) {
            Ok(engine) => return engine,
            Err(e) => e,
        };
        let fallback = std::env::temp_dir().join(format!("scancraft-session-{}", std::process::id()));
        let mut options = EngineOptions::new(fallback.clone(), services.clone());
        options.settings = Location::Memory;
        let (mut engine, why) = match Engine::open(options) {
            Ok(engine) => (engine, format!("this session is kept in a temporary folder ({first})")),
            Err(second) => {
                let mut options = EngineOptions::new(fallback.clone(), services);
                options.settings = Location::Memory;
                (Engine::from_session(Session::detached(fallback), options), format!("this session can't be saved ({first}; {second})"))
            }
        };
        engine.notify(format!("The session folder couldn't be used: {why}."), true);
        engine
    }

    /// Opens a session, reporting why it can't be opened.
    pub fn open(options: EngineOptions) -> Result<Engine, EngineError> {
        let mut session = Session::open(options.session_dir.clone())?;
        let repairs = session.take_repairs();
        // A session saved by an earlier version kept the pages being corrected apart from the
        // document; now every page is in it.
        let moved = session.graduate_pending();
        let mut engine = Engine::from_session(session, options);
        for r in repairs {
            engine.notify(format!("The saved session was repaired: {r}."), false);
        }
        if moved > 0 {
            engine.mark_dirty();
        }
        engine.resume();
        engine.open_first_page();
        Ok(engine)
    }

    fn from_session(session: Session, options: EngineOptions) -> Engine {
        let settings = SettingsHolder::open(options.settings);
        let threads = if options.threads == 0 { std::thread::available_parallelism().map_or(2, |n| n.get()) } else { options.threads };
        let root = options
            .sessions_root
            .or_else(|| settings.get().sessions_dir.as_deref().map(crate::settings::path_from_setting))
            .or_else(|| session.dir().parent().map(PathBuf::from))
            .unwrap_or_else(|| session.dir().to_path_buf());
        let cache = Arc::new(Cache::new(session.cache_dir()).unwrap_or_else(|_| Cache::unchecked(session.cache_dir())));
        let restore_offer =
            session::list_sessions(&root).into_iter().find(|s| !s.exported && s.pages > 0 && s.dir != *session.dir()).map(|s| s.pages);
        let mut engine = Engine {
            session,
            root,
            services: options.services,
            settings,
            jobs: JobRunner::new(threads),
            cache,
            undo: UndoStack::new(),
            screen: Screen::Import,
            current_photo: None,
            current_page: None,
            tokens: HashMap::new(),
            status: HashMap::new(),
            detected: HashMap::new(),
            images: HashMap::new(),
            image_order: Vec::new(),
            inflight: HashSet::new(),
            failed: HashSet::new(),
            pending_notices: Vec::new(),
            visible_notices: Vec::new(),
            exporting: false,
            export_done: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            export_total: 0,
            export_cancelled: false,
            last_export: None,
            reviewed: HashSet::new(),
            page_failures: HashMap::new(),
            flatten_tags: HashMap::new(),
            upright_tags: HashMap::new(),
            text_tags: HashMap::new(),
            batch_total: 0,
            restore_offer,
            last_look: Look::ColorDocument,
            loupe: None,
            want_auto: HashSet::new(),
            epoch: 0,
            started: Instant::now(),
            clock: None,
            last_save_ms: None,
            dirty: false,
            names: HashMap::new(),
        };
        // A new session starts with Book Spreads as the last one left it (§4.3).
        if engine.session.file().photos.is_empty() && engine.settings.get().book_spreads {
            engine.session.file_mut().book_spreads = true;
        }
        engine
    }

    /// Picks up where a restored session left off: photos whose detection never finished are detected again.
    pub(crate) fn resume(&mut self) {
        let items: Vec<_> = self.session.file().photos.clone();
        self.batch_total = items.iter().filter(|item| item.size == (0, 0)).count() as u32;
        for item in items {
            if item.size == (0, 0) {
                self.status.insert(item.id, PhotoStatus::Pending);
                if !self.detect_inflight(item.id) {
                    self.submit_detect(&item);
                }
            } else {
                self.status.insert(item.id, PhotoStatus::Ready);
            }
            self.names.insert(item.id, item.name.clone());
        }
    }

    /// Whether a PDF is being written now, so the app asks before it quits (§4.8).
    pub fn is_exporting(&self) -> bool {
        self.exporting
    }

    pub fn session_dir(&self) -> &PathBuf {
        self.session.dir()
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn settings(&self) -> &Settings {
        self.settings.get()
    }

    /// Changes the settings; they are written shortly after.
    pub fn set_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        self.settings.update(f);
        // A new scans folder takes effect at once: the next new scan is made there, and the last
        // unfinished scan is looked for there. The scan that is open stays where it is.
        let chosen = self.settings.get().sessions_dir.as_deref().map(crate::settings::path_from_setting);
        if let Some(root) = chosen.filter(|root| *root != self.root) {
            self.restore_offer =
                session::list_sessions(&root).into_iter().find(|s| !s.exported && s.pages > 0 && s.dir != *self.session.dir()).map(|s| s.pages);
            self.root = root;
        }
    }

    /// Replaces the clock the undo coalescing and the save interval read, in milliseconds (for tests).
    pub fn set_clock(&mut self, clock: impl Fn() -> u64 + Send + 'static) {
        self.clock = Some(Box::new(clock));
    }

    pub(crate) fn now_ms(&self) -> u64 {
        match &self.clock {
            Some(clock) => clock(),
            None => self.started.elapsed().as_millis() as u64,
        }
    }

    /// Raises a notice for the user (§4.8); it appears with the next `poll`.
    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if !self.pending_notices.iter().any(|n| n.text == text) {
            self.pending_notices.push(Notice { text, error });
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Writes the session when something changed and a second has passed since the last write (§6.6).
    pub fn save_if_due(&mut self) {
        let now = self.now_ms();
        if self.dirty && self.last_save_ms.is_none_or(|t| now.saturating_sub(t) >= SAVE_INTERVAL_MS) {
            let _ = self.save_now();
        }
    }

    /// Writes the session now.
    pub fn save_now(&mut self) -> Result<(), EngineError> {
        self.last_save_ms = Some(self.now_ms());
        match self.session.save() {
            Ok(()) => {
                self.dirty = false;
                Ok(())
            }
            Err(e) => {
                self.notify(format!("The session couldn't be saved: {e}"), true);
                Err(e)
            }
        }
    }

    /// Saves everything now: the session and the settings (for quitting).
    pub fn flush(&mut self) {
        let _ = self.save_now();
        let _ = self.settings.flush();
    }

    /// The token for a target's jobs; a new one replaces (and cancels) the old.
    pub(crate) fn fresh_token(&mut self, target: Target) -> CancellationToken {
        if let Some(old) = self.tokens.insert(target, CancellationToken::new()) {
            old.cancel();
        }
        self.tokens.get(&target).cloned().unwrap_or_default()
    }

    pub(crate) fn token(&mut self, target: Target) -> CancellationToken {
        self.tokens.entry(target).or_default().clone()
    }

    /// Cancels the jobs of a page whose recipe changed.
    pub(crate) fn cancel_page(&mut self, page: PageId) {
        // A page whose picture could not be made gets another chance with its new recipe.
        self.page_failures.remove(&page);
        for target in [Target::Page(page), Target::Preview(page)] {
            if let Some(t) = self.tokens.remove(&target) {
                t.cancel();
            }
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for (_, t) in self.tokens.drain() {
            t.cancel();
        }
        self.inflight.clear();
        self.failed.clear();
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            file: self.session.file().clone(),
            pending: self.session.pending().to_vec(),
            screen: self.screen,
            current_photo: self.current_photo,
            current_page: self.current_page,
        }
    }

    pub(crate) fn restore(&mut self, snap: Snapshot) {
        *self.session.file_mut() = snap.file;
        *self.session.pending_mut() = snap.pending;
        self.screen = snap.screen;
        self.current_photo = snap.current_photo;
        self.current_page = snap.current_page;
        self.session.resync_ids();
        self.cancel_all();
        self.loupe = None;
        self.want_auto.clear();
        // A photo restored before its detection finished is detected again.
        self.resume();
        self.mark_dirty();
    }

    /// Waits for the job queue to empty, applying results as they come; true when it emptied in time.
    pub fn wait_idle(&mut self, timeout: std::time::Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll();
            if self.jobs.pending() == 0 {
                self.poll();
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel_all();
        self.flush();
    }
}

impl EngineApi for Engine {
    fn view(&self) -> ViewState {
        self.build_view()
    }

    fn dispatch(&mut self, command: Command) -> Result<(), String> {
        let result = self.execute(command);
        self.save_if_due();
        result.map_err(|e| e.to_string())
    }

    fn poll(&mut self) -> bool {
        self.apply_finished_jobs()
    }

    fn image(&mut self, key: ImageKey) -> Option<Arc<Rgb8>> {
        self.request_image(key)
    }

    fn loupe(&mut self, photo: PhotoId, centre: Point, side: f64) -> Option<LoupeCrop> {
        self.loupe_crop(photo, centre, side)
    }
}
