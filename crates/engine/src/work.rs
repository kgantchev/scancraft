//! The engine's side of the job runner: what jobs are made for, and what happens to their results.

use crate::cache::{CacheKind, cache_key, display_key};
use crate::error::EngineError;
use crate::jobs::{CancellationToken, Job, JobDone, JobMeta, JobPriority};
use crate::pipeline::{
    Ctx, DetectedPhoto, ExportReport, FILTER_THUMB_LONG_SIDE, PREVIEW_LONG_SIDE, TRAY_THUMB_LONG_SIDE, crop_square, detect_photo, display_copy,
    export_pdf, load_photo, page_geometry, photo_key, recognize_page, render_small, unfinished_note,
};
use crate::{Detected, Engine, PhotoStatus, Target};
use scancraft_core::engine_api::{ImageKey, LoupeCrop, Screen};
use scancraft_core::geom::Point;
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{CornerOrigin, FlattenMode, Look, Page, PageId, PhotoId, PhotoItem, RecognizedText, TurnOrigin};
use std::path::PathBuf;
use std::sync::Arc;

/// What a job hands back.
pub(crate) enum JobOutput {
    Detected { photo: PhotoId, found: Box<DetectedPhoto> },
    Image { key: ImageKey, cache_key: String, image: Arc<Rgb8>, notes: Vec<String> },
    Text { page: PageId, for_key: String, text: Option<RecognizedText> },
    Upright { page: PageId, for_key: String, turns: Option<u8>, notes: Vec<String> },
    Exported { report: ExportReport, path: PathBuf },
    Decoded,
}

/// The images kept in memory at most.
const MAX_IMAGES: usize = 96;
/// The loupe: 1.5× the area the loupe shows, at most 1024 px a side, cut again when the node moves a quarter of the crop's side (§6.5).
const LOUPE_AREA_FACTOR: f64 = 1.5;
const LOUPE_MAX_SIDE: f64 = 1024.0;
const LOUPE_RECENTRE: f64 = 0.25;

/// A request's identity (FNV-1a over its parts), to tell when the same one is already running.
pub(crate) fn tag_of(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in parts.join("\u{1}").bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    // 0 means "no tag".
    hash.max(1)
}

/// The key a page's recognised text was made for: the page as it is rendered at 300 DPI.
pub(crate) fn text_key(page: &Page) -> String {
    cache_key(page, 300, CacheKind::FinalPage)
}

impl Engine {
    pub(crate) fn ctx(&self) -> Ctx {
        Ctx { cache: self.cache.clone(), sources: self.session.sources_dir(), services: self.services.clone(), settings: self.settings.get().clone() }
    }

    fn submit_job(&mut self, priority: JobPriority, token: CancellationToken, mut meta: JobMeta, work: crate::jobs::JobWork<JobOutput>) {
        meta.generation = self.epoch;
        if meta.tag != 0 {
            self.inflight.insert(meta.tag);
        }
        self.jobs.submit(Job { priority, token, meta, work });
    }

    pub(crate) fn detect_inflight(&self, photo: PhotoId) -> bool {
        self.inflight.contains(&tag_of(&["detect", &photo.0.to_string()]))
    }

    /// Decodes a new photo and finds its page.
    pub(crate) fn submit_detect(&mut self, item: &PhotoItem) {
        let token = self.token(Target::Photo(item.id));
        let ctx = self.ctx();
        let item = item.clone();
        let meta = JobMeta { photo: Some(item.id), tag: tag_of(&["detect", &item.id.0.to_string()]), ..JobMeta::default() };
        let id = item.id;
        self.submit_job(
            JobPriority::Detection,
            token,
            meta,
            Box::new(move |token| detect_photo(&ctx, &item, token).map(|found| JobOutput::Detected { photo: id, found: Box::new(found) })),
        );
    }

    /// The image for `key` if it is ready; otherwise a job makes it and this returns `None`.
    pub(crate) fn request_image(&mut self, key: ImageKey) -> Option<Arc<Rgb8>> {
        let plan = self.image_plan(key)?;
        if let Some((k, image)) = self.images.get(&key)
            && *k == plan.cache_key
        {
            return Some(image.clone());
        }
        if let Some(image) = self.cache.get(&plan.cache_key) {
            self.store_image(key, plan.cache_key, image.clone());
            return Some(image);
        }
        let tag = tag_of(&["image", &plan.cache_key]);
        if self.inflight.contains(&tag) {
            return None;
        }
        let ctx = self.ctx();
        let token = match (&plan.page, key) {
            // A newer preview of a page replaces the one before it: dragging a slider asks for many.
            (Some(page), ImageKey::LookPreview(_)) => self.fresh_token(Target::Preview(page.id)),
            (Some(page), _) => self.token(Target::Page(page.id)),
            (None, _) => self.token(Target::Photo(plan.item.id)),
        };
        let meta = JobMeta { page: plan.page.as_ref().map(|p| p.id), photo: Some(plan.item.id), tag, ..JobMeta::default() };
        let (item, page, long_side, cache_key) = (plan.item, plan.page, plan.long_side, plan.cache_key);
        self.submit_job(
            plan.priority,
            token,
            meta,
            Box::new(move |token| {
                let loaded = load_photo(&ctx, &item.source, &item.origin, token)?;
                let (image, notes) = match &page {
                    Some(page) => render_small(&ctx, &loaded.image, &item, page, long_side, token)?,
                    None => (display_copy(&loaded.image)?, Vec::new()),
                };
                token.check()?;
                // The file cache is best effort: a full disk costs a re-render, not the image.
                let _ = ctx.cache.put(&cache_key, &image);
                Ok(JobOutput::Image { key, cache_key, image: Arc::new(image), notes })
            }),
        );
        None
    }

    fn image_plan(&self, key: ImageKey) -> Option<ImagePlan> {
        match key {
            ImageKey::DisplayCopy(photo) => {
                let item = self.session.photo(photo)?.clone();
                let cache_key = display_key(&item.source, crate::pipeline::photo_key(&item.source, &item.origin).1);
                Some(ImagePlan { cache_key, item, page: None, long_side: 0, priority: JobPriority::Preview })
            }
            ImageKey::LookPreview(page_id) => {
                let page = self.session.page(page_id)?.clone();
                let item = self.session.photo(page.photo)?.clone();
                Some(ImagePlan {
                    cache_key: cache_key(&page, PREVIEW_LONG_SIDE, CacheKind::LookPreview),
                    item,
                    page: Some(page),
                    long_side: PREVIEW_LONG_SIDE,
                    priority: JobPriority::Preview,
                })
            }
            ImageKey::FilterThumb(page_id, look) => {
                let mut page = self.session.page(page_id)?.clone();
                let item = self.session.photo(page.photo)?.clone();
                page.look = look;
                page.strength = look.default_strength().unwrap_or(0);
                Some(ImagePlan {
                    cache_key: cache_key(&page, FILTER_THUMB_LONG_SIDE, CacheKind::FilterThumb),
                    item,
                    page: Some(page),
                    long_side: FILTER_THUMB_LONG_SIDE,
                    priority: JobPriority::Render,
                })
            }
            ImageKey::TrayThumb(page_id) => {
                let page = self.session.page(page_id)?.clone();
                let item = self.session.photo(page.photo)?.clone();
                Some(ImagePlan {
                    cache_key: cache_key(&page, TRAY_THUMB_LONG_SIDE, CacheKind::TrayThumb),
                    item,
                    page: Some(page),
                    long_side: TRAY_THUMB_LONG_SIDE,
                    priority: JobPriority::Render,
                })
            }
        }
    }

    fn store_image(&mut self, key: ImageKey, cache_key: String, image: Arc<Rgb8>) {
        if self.images.insert(key, (cache_key, image)).is_none() {
            self.image_order.push(key);
        }
        while self.image_order.len() > MAX_IMAGES {
            let oldest = self.image_order.remove(0);
            self.images.remove(&oldest);
        }
    }

    /// Starts text recognition for a page in the background (lowest priority).
    pub(crate) fn submit_recognition(&mut self, page: &Page) {
        // The text is read whether or not the last export asked for it: a later export may.
        if self.services.recognizer.is_none() {
            return;
        }
        let Some(item) = self.session.photo(page.photo).cloned() else { return };
        let token = self.token(Target::Page(page.id));
        let ctx = self.ctx();
        let (page, languages) = (page.clone(), self.session.file().export.languages.clone());
        let for_key = text_key(&page);
        let meta = JobMeta { page: Some(page.id), photo: Some(page.photo), tag: tag_of(&["text", &for_key]), ..JobMeta::default() };
        let id = page.id;
        self.submit_job(
            JobPriority::Recognition,
            token,
            meta,
            Box::new(move |token| {
                let loaded = load_photo(&ctx, &item.source, &item.origin, token)?;
                let text = recognize_page(&ctx, &loaded.image, &item, &page, &languages, token)?;
                Ok(JobOutput::Text { page: id, for_key, text })
            }),
        );
    }

    /// A15 on the page, in the background: the page is turned upright when the vote is sure (§4.4).
    pub(crate) fn submit_upright(&mut self, page: &Page) {
        let Some(item) = self.session.photo(page.photo).cloned() else { return };
        let token = self.token(Target::Page(page.id));
        let ctx = self.ctx();
        let page = page.clone();
        let for_key = text_key(&page);
        let meta = JobMeta { page: Some(page.id), photo: Some(page.photo), tag: tag_of(&["upright", &for_key]), ..JobMeta::default() };
        let id = page.id;
        self.submit_job(
            JobPriority::Preview,
            token,
            meta,
            Box::new(move |token| {
                use scancraft_enhance::upright::upright;
                let loaded = load_photo(&ctx, &item.source, &item.origin, token)?;
                let mut grey = page.clone();
                grey.look = Look::Grayscale;
                grey.quarter_turns = 0;
                // The page as a 1024 px Grayscale picture (A15 step 1).
                let (image, mut notes) = render_small(&ctx, &loaded.image, &item, &grey, 1024, token)?;
                let pixels: Vec<u8> = image.data().iter().map(|p| p[0]).collect();
                let gray = scancraft_core::image::Plane::from_vec(image.width(), image.height(), pixels).map_err(EngineError::from)?;
                let turns = match upright(&gray, ctx.services.upright.as_deref()) {
                    Ok(u) => Some(u.quarter_turns % 4),
                    Err(e) => {
                        let e = EngineError::from(e);
                        notes.push(e.unfinished_step().map_or_else(|| format!("Turning the page upright failed: {e}"), unfinished_note));
                        None
                    }
                };
                Ok(JobOutput::Upright { page: id, for_key, turns, notes })
            }),
        );
    }

    /// Renders every tray page at the output density and writes the PDF, in a job.
    pub(crate) fn submit_export(&mut self, path: PathBuf) {
        let file = self.session.file();
        let pages: Vec<(PhotoItem, Page)> =
            file.pages.iter().filter_map(|p| file.photos.iter().find(|i| i.id == p.photo).map(|i| (i.clone(), p.clone()))).collect();
        let options = file.export.clone();
        let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Scan").to_string();
        let token = self.fresh_token(Target::Export);
        let ctx = self.ctx();
        let meta = JobMeta { tag: tag_of(&["export"]), ..JobMeta::default() };
        self.submit_job(
            JobPriority::Render,
            token,
            meta,
            Box::new(move |token| export_pdf(&ctx, &pages, &options, &title, &path, token).map(|report| JobOutput::Exported { report, path })),
        );
    }

    /// The loupe crop (§6.5): a square of the full-resolution photo, 1.5× the area shown.
    pub(crate) fn loupe_crop(&mut self, photo: PhotoId, centre: Point, side: f64) -> Option<LoupeCrop> {
        if !(side.is_finite() && side > 0.0) || !centre.is_finite() {
            return None;
        }
        let item = self.session.photo(photo)?.clone();
        // Reuse the last crop while the node stays within a quarter of its side of the crop's centre.
        if let Some(l) = &self.loupe
            && l.photo == photo
            && (l.side - side).abs() < 1e-9
            && l.centre.distance(centre) <= LOUPE_RECENTRE * f64::from(l.crop.image.width()) * l.crop.scale
        {
            return Some(l.crop.clone());
        }
        let decoded = self.cache.photo(&photo_key(&item.source, &item.origin));
        let Some(decoded) = decoded else {
            let tag = tag_of(&["decode", &photo.0.to_string()]);
            if !self.inflight.contains(&tag) {
                let (ctx, token) = (self.ctx(), self.token(Target::Photo(photo)));
                let meta = JobMeta { photo: Some(photo), tag, ..JobMeta::default() };
                self.submit_job(
                    JobPriority::Preview,
                    token,
                    meta,
                    Box::new(move |token| load_photo(&ctx, &item.source, &item.origin, token).map(|_| JobOutput::Decoded)),
                );
            }
            return None;
        };
        let wanted = side * LOUPE_AREA_FACTOR.sqrt();
        let crop_side = wanted.min(LOUPE_MAX_SIDE).round().max(1.0);
        let scale = wanted / crop_side;
        let (x0, y0) = (centre.x - wanted / 2.0, centre.y - wanted / 2.0);
        let image = Arc::new(crop_square(&decoded, x0, y0, crop_side as u32, scale).ok()?);
        let crop = LoupeCrop { image, origin: Point::new(x0, y0), scale };
        self.loupe = Some(crate::LoupeState { photo, side, centre, crop: crop.clone() });
        Some(crop)
    }

    /// Takes the finished jobs, applies them to the session and the caches, and moves the notices
    /// across; true when anything the view shows may have changed.
    pub(crate) fn apply_finished_jobs(&mut self) -> bool {
        let mut changed = false;
        for done in self.jobs.drain() {
            // What a job of an earlier session returns belongs to no page of this one.
            if done.meta.generation != self.epoch {
                continue;
            }
            if done.meta.tag != 0 {
                self.inflight.remove(&done.meta.tag);
            }
            changed |= self.apply_done(done);
        }
        if !self.pending_notices.is_empty() {
            self.visible_notices = std::mem::take(&mut self.pending_notices);
            changed = true;
        } else if !self.visible_notices.is_empty() {
            // A notice is shown for one poll after it appears; the interface keeps what it wants.
            self.visible_notices.clear();
            changed = true;
        }
        let _ = self.settings.tick(std::time::Instant::now());
        self.save_if_due();
        changed
    }

    fn apply_done(&mut self, done: JobDone<JobOutput>) -> bool {
        let meta = done.meta;
        let output = match done.outcome {
            Ok(output) => output,
            Err(EngineError::Cancelled) => {
                // Cancelling a job is not news, with one exception. An export is cancelled when
                // an undo restores a snapshot, and it must not stay marked as running: that
                // refused every later export. The screen is left as the restore set it.
                if meta.tag == tag_of(&["export"]) && self.exporting.take().is_some() {
                    self.notify("The PDF wasn't written, because the pages changed while it was being made.", false);
                    return true;
                }
                return false;
            }
            Err(e) => return self.job_failed(meta, e),
        };
        match output {
            JobOutput::Detected { photo, found } => self.apply_detected(photo, *found),
            JobOutput::Image { key, cache_key, image, notes } => {
                self.store_image(key, cache_key, image);
                for n in notes {
                    self.notify(n, false);
                }
                true
            }
            JobOutput::Text { page, for_key, text } => {
                let Some(p) = self.session.page_mut(page) else { return false };
                // A result made for an older recipe is dropped.
                if text_key(p) != for_key {
                    return false;
                }
                p.text = text;
                self.mark_dirty();
                true
            }
            JobOutput::Upright { page, for_key, turns, notes } => {
                for n in notes {
                    self.notify(n, false);
                }
                let Some(p) = self.session.page_mut(page) else { return false };
                if text_key(p) != for_key || p.turns_from != TurnOrigin::Auto || p.quarter_turns != 0 {
                    return false;
                }
                if let Some(t) = turns.filter(|t| *t != 0) {
                    p.quarter_turns = t;
                    p.text = None;
                    self.mark_dirty();
                }
                true
            }
            JobOutput::Exported { report, path } => self.export_finished(report, path),
            JobOutput::Decoded => true,
        }
    }

    fn job_failed(&mut self, meta: JobMeta, e: EngineError) -> bool {
        if meta.tag == tag_of(&["export"]) {
            if let Some(back) = self.exporting.take() {
                self.screen = back;
            }
            // A step that isn't finished stops the export, and the notice says which one.
            let why = e.unfinished_step().map_or_else(|| e.to_string(), unfinished_note);
            self.notify(format!("The PDF couldn't be written: {why}"), true);
            return true;
        }
        // A step that isn't finished is news, not a failure.
        if let Some(step) = e.unfinished_step() {
            self.notify(unfinished_note(step), false);
            return true;
        }
        let name = meta.photo.and_then(|p| self.names.get(&p).cloned());
        let is_detect = meta.tag == tag_of(&["detect", &meta.photo.map_or(0, |p| p.0).to_string()]);
        if is_detect && let Some(photo) = meta.photo {
            // A photo that can't be read is skipped, with its name and the reason (§4.8).
            self.drop_photo(photo);
            self.notify(format!("{}: {e}", name.unwrap_or_else(|| "A file".to_string())), true);
            return true;
        }
        self.notify(
            match name {
                Some(n) => format!("{n}: {e}"),
                None => e.to_string(),
            },
            true,
        );
        true
    }

    /// Removes a photo and every page cut from it.
    pub(crate) fn drop_photo(&mut self, photo: PhotoId) {
        let pages: Vec<PageId> = self.session.pages_of(photo).iter().map(|p| p.id).collect();
        for id in pages {
            self.cancel_page(id);
        }
        if let Some(t) = self.tokens.remove(&Target::Photo(photo)) {
            t.cancel();
        }
        self.session.file_mut().photos.retain(|p| p.id != photo);
        self.session.file_mut().pages.retain(|p| p.photo != photo);
        self.session.pending_mut().retain(|p| p.photo != photo);
        self.status.remove(&photo);
        self.detected.remove(&photo);
        if self.current_photo == Some(photo) {
            self.current_photo = None;
        }
        if matches!(self.screen, Screen::Crop(p) if p == photo) || self.current_page.is_some_and(|c| self.session.page(c).is_none()) {
            self.current_page = None;
            self.screen = Screen::Import;
        }
        self.mark_dirty();
    }

    fn apply_detected(&mut self, photo: PhotoId, found: DetectedPhoto) -> bool {
        let Some(item) = self.session.photo_mut(photo) else { return false };
        item.size = found.size;
        item.focal = found.focal;
        let item = item.clone();
        self.status.insert(photo, PhotoStatus::Ready);
        let detected = Detected { corners: found.corners, sides: found.sides.clone(), origin: found.origin };
        self.detected.insert(photo, detected.clone());
        for note in &found.notes {
            self.notify(note.clone(), false);
        }
        let wanting: Vec<PageId> =
            self.session.pending().iter().filter(|p| p.photo == photo && self.want_auto.contains(&p.id)).map(|p| p.id).collect();
        for id in wanting {
            self.want_auto.remove(&id);
            if let Err(e) = self.apply_detected_to(id, &detected) {
                self.notify(e.to_string(), true);
            }
        }
        if self.session.pages_of(photo).is_empty() {
            let page = self.new_page(&item, &found);
            self.session.pending_mut().push(page);
            if self.session.file().book_spreads {
                // A30 isn't finished: the spread is cut down the middle, and the user can move the spine.
                if self.split_pending(photo).is_ok() {
                    self.notify("Finding a book's spine isn't ready yet (A30): the photo is cut down the middle.", false);
                }
            }
        }
        if self.screen == Screen::Import && self.current_photo.is_none() {
            self.screen = Screen::Crop(photo);
            self.current_photo = Some(photo);
        }
        self.mark_dirty();
        true
    }

    /// A page for a photo whose outline was just found.
    pub(crate) fn new_page(&mut self, item: &PhotoItem, found: &DetectedPhoto) -> Page {
        let look = self.last_look;
        Page {
            id: self.session.allocate_page_id(),
            photo: item.id,
            source: item.source.clone(),
            corners: found.corners,
            corners_from: found.origin,
            sides: found.sides.clone(),
            spread: None,
            flatten: if self.settings.get().flatten_automatically && !matches!(found.origin, CornerOrigin::FullImage) {
                FlattenMode::Auto
            } else {
                FlattenMode::Off
            },
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: TurnOrigin::Auto,
            look,
            strength: look.default_strength().unwrap_or(0),
            text: None,
        }
    }

    fn export_finished(&mut self, report: ExportReport, path: PathBuf) -> bool {
        if let Some(back) = self.exporting.take() {
            self.screen = back;
        }
        self.session.file_mut().exported = true;
        self.mark_dirty();
        let _ = self.save_now();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("the PDF").to_string();
        let mut text = format!("Exported {} page{} to {name}.", report.pages, if report.pages == 1 { "" } else { "s" });
        if !report.not_searchable.is_empty() {
            let list: Vec<String> = report.not_searchable.iter().map(u32::to_string).collect();
            text.push_str(&format!(" These pages aren't searchable: {}.", list.join(", ")));
        }
        self.notify(text, false);
        for n in report.notes {
            self.notify(n, false);
        }
        true
    }

    /// The corners a photo starts with when no page was found: 3% inside its edges.
    pub(crate) fn fallback_corners(&self, photo: PhotoId) -> Option<scancraft_core::geom::Quad> {
        let item = self.session.photo(photo)?;
        (item.size != (0, 0)).then(|| crate::crop::not_found_corners(item.size))
    }

    /// The geometry of a page, for the view and the commands: `None` when the photo isn't decoded yet.
    pub(crate) fn geometry_of(&self, page: &Page) -> Option<crate::pipeline::PageGeometry> {
        let item = self.session.photo(page.photo)?;
        if item.size == (0, 0) {
            return None;
        }
        page_geometry(page, item.size, item.focal, self.settings.get()).ok()
    }
}

struct ImagePlan {
    cache_key: String,
    item: PhotoItem,
    page: Option<Page>,
    long_side: u32,
    priority: JobPriority,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use scancraft_core::{CoreError, Step};

    fn engine(name: &str) -> (Engine, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("scancraft-work-{name}-{}", std::process::id()));
        let mut options = crate::EngineOptions::new(dir.join("s"), Services::default());
        options.settings = craft_settings::Location::Memory;
        options.threads = 1;
        (Engine::open(options).unwrap(), dir)
    }

    fn failed(tag: u64, step: Step) -> JobDone<JobOutput> {
        JobDone { meta: JobMeta { tag, ..JobMeta::default() }, outcome: Err(EngineError::Core(CoreError::NotImplemented(step))) }
    }

    #[test]
    fn a_step_that_is_not_finished_becomes_a_notice_not_an_error() {
        let (mut engine, dir) = engine("unfinished");
        assert!(engine.apply_done(failed(tag_of(&["image", "x"]), Step::A16)));
        let notices = std::mem::take(&mut engine.pending_notices);
        assert_eq!(notices.len(), 1);
        assert!(!notices[0].error && notices[0].text.contains("isn't ready yet"), "{notices:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_export_says_so_and_returns_to_the_screen_it_came_from() {
        let (mut engine, dir) = engine("export");
        engine.exporting = Some(Screen::Import);
        engine.screen = Screen::Export;
        assert!(engine.apply_done(failed(tag_of(&["export"]), Step::A23)));
        assert_eq!(engine.screen, Screen::Import);
        assert!(engine.exporting.is_none());
        assert!(
            engine.pending_notices.iter().any(|n| n.error && n.text.contains("PDF") && n.text.contains("isn't ready yet")),
            "{:?}",
            engine.pending_notices
        );
        // A real failure is shown with its reason.
        engine.pending_notices.clear();
        engine.exporting = Some(Screen::Import);
        let done = JobDone { meta: JobMeta { tag: tag_of(&["export"]), ..JobMeta::default() }, outcome: Err(EngineError::msg("the disk is full")) };
        assert!(engine.apply_done(done));
        assert!(engine.pending_notices.iter().any(|n| n.error && n.text.contains("the disk is full")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancelled_export_lets_the_next_export_start() {
        // Undo restores a snapshot and cancels every job, the export's included. The export's
        // flag used to stay set, so every later export failed with "already being made".
        let (mut engine, dir) = engine("export-cancelled");
        engine.exporting = Some(Screen::Export);
        engine.screen = Screen::Import;
        let done = JobDone { meta: JobMeta { tag: tag_of(&["export"]), ..JobMeta::default() }, outcome: Err(EngineError::Cancelled) };
        assert!(engine.apply_done(done));
        assert!(engine.exporting.is_none(), "the flag for an export in progress is still set");
        // The screen is the one the undo restored, not the one the export came from.
        assert_eq!(engine.screen, Screen::Import);
        assert!(engine.pending_notices.iter().any(|n| n.text.contains("PDF")), "{:?}", engine.pending_notices);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_job_of_an_earlier_session_is_ignored() {
        let (mut engine, dir) = engine("epoch");
        engine.epoch = 3;
        let meta = JobMeta { generation: 2, tag: 5, ..JobMeta::default() };
        engine.jobs.submit(Job {
            priority: JobPriority::Render,
            token: CancellationToken::new(),
            meta,
            work: Box::new(|_| Err(EngineError::msg("old news"))),
        });
        assert!(engine.jobs.wait_idle(std::time::Duration::from_secs(10)));
        engine.apply_finished_jobs();
        assert!(engine.pending_notices.is_empty() && engine.visible_notices.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_requests_tag_is_never_zero_and_tells_requests_apart() {
        assert_ne!(tag_of(&["detect", "1"]), tag_of(&["detect", "2"]));
        assert_ne!(tag_of(&["detect", "1"]), tag_of(&["image", "1"]));
        assert_ne!(tag_of(&[]), 0);
    }
}
