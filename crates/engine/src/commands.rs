//! The dispatcher (§6.3): every command by id, with the undo bookkeeping around it.
//!
//! A command checks everything it needs before it changes anything, so an error leaves the session
//! exactly as it was. Commands run by a program never open dialogs: where the interface asks a
//! question (an export with photos not yet added), the first call answers with an error that carries
//! the question and the next call goes through.

use crate::crop::{clamp_corner, full_image_corners, invalid_reason, move_side};
use crate::error::EngineError;
use crate::pipeline::page_geometry;
use crate::session::{self, MAX_PAGES, Session};
use crate::undo::UndoStep;
use crate::{Engine, KEEP_SESSIONS, PhotoStatus, Target};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::Screen;
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::recipe::{
    CornerOrigin, ExportOptions, FlattenMode, FocalInput, Half, ImportOrigin, Look, Page, PageId, PageOrder, PhotoId, PhotoItem, SideShape, SourceId,
    SpreadHalf, TurnOrigin,
};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// The largest file the engine reads into memory to import.
const MAX_IMPORT_BYTES: u64 = 1 << 30;
/// How many notices one import raises about files it couldn't use, before it sums up the rest.
const MAX_IMPORT_NOTICES: usize = 5;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Engine {
    /// Runs one command; on success the change is recorded for undo, on failure nothing changed.
    pub(crate) fn execute(&mut self, command: Command) -> Result<(), EngineError> {
        if !matches!(command, Command::ExportPdf { .. }) {
            self.export_confirmed = false;
        }
        match command {
            Command::EditUndo => {
                let now = self.snapshot();
                let snap = self.undo.undo(&now).ok_or_else(|| EngineError::msg("There is nothing to undo."))?;
                self.restore(snap);
                return Ok(());
            }
            Command::EditRedo => {
                let now = self.snapshot();
                let snap = self.undo.redo(&now).ok_or_else(|| EngineError::msg("There is nothing to redo."))?;
                self.restore(snap);
                return Ok(());
            }
            _ => {}
        }
        // Exporting and changing sessions are not undone: the PDF stays on disk, and the old
        // session's files live in the old session's folder.
        let recordable = !matches!(command, Command::ExportPdf { .. } | Command::SessionNew | Command::SessionRestore);
        let before = self.snapshot();
        match self.run(&command) {
            Ok(()) => {
                self.mark_dirty();
                if recordable {
                    let after = self.snapshot();
                    if after != before {
                        let at_ms = self.now_ms();
                        self.undo.record(UndoStep::new(command, &before, &after, at_ms));
                    }
                }
                Ok(())
            }
            Err(e) => {
                // Commands check before they change, so this only matters for one that failed half way.
                if self.snapshot() != before {
                    self.restore(before);
                }
                Err(e)
            }
        }
    }

    fn run(&mut self, command: &Command) -> Result<(), EngineError> {
        match command {
            Command::ImportAdd { paths } => self.import_add(paths),
            Command::ImportDiscard { item } => self.import_discard(*item),
            Command::CropSetCorner { page, corner, x, y } => self.crop_set_corner(*page, *corner, *x, *y),
            Command::CropMoveSide { page, side, offset } => self.crop_move_side(*page, *side, *offset),
            Command::CropBendSide { page, side, handles } => self.crop_bend_side(*page, *side, handles),
            Command::CropStraightenSide { page, side } => self.edit_outline(*page, |p| p.sides[side.index()] = SideShape::Straight, false),
            Command::CropSetFlatten { page, mode } => self.crop_set_flatten(*page, *mode),
            Command::CropBook { on } => self.crop_book(*on),
            Command::CropSplit { photo, on } => self.crop_split(*photo, *on),
            Command::CropSetSpine { photo, top, bottom } => self.crop_set_spine(*photo, *top, *bottom),
            Command::CropAuto { page } => self.crop_auto(*page),
            Command::CropFullImage { page } => self.crop_full_image(*page),
            Command::CropApply { photo } => self.crop_apply(*photo),
            Command::PageSetLook { page, look } => self.page_set_look(*page, *look),
            Command::PageSetStrength { page, value } => self.page_edit_look(*page, |p| p.strength = (*value).min(100)),
            Command::PageRotate { page, quarter_turns } => self.page_rotate(*page, *quarter_turns),
            Command::PageAdd { page } => self.page_add(*page),
            Command::PageDelete { page } => self.page_delete(*page),
            Command::PageMove { from, to } => self.page_move(*from, *to),
            Command::PageEdit { page } => self.page_edit(*page),
            Command::PageOpenCrop { page } => self.page_open_crop(*page),
            Command::ReferenceLabel { page, folder, category } => self.reference_label(*page, folder, category),
            Command::ExportPdf { path, options } => self.export_pdf_command(path, options),
            Command::SessionNew => self.session_new(),
            Command::SessionRestore => self.session_restore(),
            // Handled before the snapshot is taken.
            Command::EditUndo | Command::EditRedo => Ok(()),
        }
    }

    // ---- import ----

    fn import_add(&mut self, paths: &[String]) -> Result<(), EngineError> {
        if paths.is_empty() {
            return Err(EngineError::msg("There is nothing to import."));
        }
        let mut problems = Vec::new();
        let files = expand_paths(paths, &mut problems);
        let mut imported = 0usize;
        for file in files {
            if self.session.file().photos.len() >= MAX_PAGES {
                self.notify(format!("A session holds at most {MAX_PAGES} photos; the rest weren't imported."), true);
                break;
            }
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("A file").to_string();
            match self.import_file(&file, &name) {
                Ok(n) => imported += n,
                Err(why) => problems.push(format!("{name}: {why}")),
            }
        }
        let extra = problems.len().saturating_sub(MAX_IMPORT_NOTICES);
        for p in problems.iter().take(MAX_IMPORT_NOTICES) {
            self.notify(p.clone(), true);
        }
        if extra > 0 {
            self.notify(format!("{} more files couldn't be imported.", extra), true);
        }
        if imported == 0 {
            return Err(EngineError::msg(if problems.is_empty() {
                "Those files are already waiting to be corrected.".to_string()
            } else {
                "No file could be imported.".to_string()
            }));
        }
        Ok(())
    }

    /// Copies one file into the session and queues its pages; returns how many photos it added.
    fn import_file(&mut self, path: &Path, name: &str) -> Result<usize, String> {
        let bytes = read_capped(path)?;
        let kind = sniff(&bytes).ok_or_else(|| "this isn't a supported image or PDF.".to_string())?;
        let source = SourceId(scancraft_io::import::source_id(&bytes));
        let tiff_pages = if kind == Kind::Tiff { scancraft_io::import::tiff_page_count(&bytes) } else { 0 };
        // The first item of the file: the same file (or the same page of it) twice is imported once
        // while the first is still waiting to be corrected, so a file dropped twice in a row doesn't
        // make two photos. Once its page is in the tray (or was deleted), the file may be imported
        // again: the user may want another scan of it, and has no other way to get it back.
        let first = match kind {
            Kind::Pdf => ImportOrigin::PdfPage { index: 0 },
            Kind::Tiff if tiff_pages > 1 => ImportOrigin::TiffPage { index: 0 },
            _ => ImportOrigin::File,
        };
        let waiting = self.session.file().photos.iter().any(|p| {
            p.source == source
                && p.origin == first
                && (matches!(self.status.get(&p.id), Some(PhotoStatus::Pending)) || !self.pending_ids(p.id).is_empty())
        });
        if waiting {
            self.notify(format!("{name} is already waiting to be corrected."), false);
            return Ok(0);
        }
        // The source is copied into the session before it is read, so the bytes can be moved into the
        // PDF's reader. A file that turns out to be unreadable leaves nothing behind.
        let stored = self.session.source_path(&source);
        let existed = stored.exists();
        self.session.add_source(&source, &bytes).map_err(|e| e.to_string())?;
        let items: Vec<(ImportOrigin, String)> = match kind {
            Kind::Pdf => {
                let doc = match scancraft_io::pdf_import::PdfDocument::open(bytes) {
                    Ok(doc) => doc,
                    Err(e) => {
                        if !existed {
                            let _ = std::fs::remove_file(&stored);
                        }
                        return Err(e.to_string());
                    }
                };
                let count = doc.page_count() as usize;
                if count == 0 {
                    if !existed {
                        let _ = std::fs::remove_file(&stored);
                    }
                    return Err("the PDF has no pages.".to_string());
                }
                if count > MAX_PAGES {
                    self.notify(format!("{name} has {count} pages; only the first {MAX_PAGES} were imported."), false);
                }
                (0..count.min(MAX_PAGES))
                    .map(|index| (ImportOrigin::PdfPage { index: index as u32 }, format!("{name}, page {}", index + 1)))
                    .collect()
            }
            Kind::Tiff if tiff_pages > 1 => {
                if tiff_pages > MAX_PAGES {
                    self.notify(format!("{name} has {tiff_pages} pages; only the first {MAX_PAGES} were imported."), false);
                }
                (0..tiff_pages.min(MAX_PAGES))
                    .map(|index| (ImportOrigin::TiffPage { index: index as u32 }, format!("{name}, page {}", index + 1)))
                    .collect()
            }
            _ => vec![(ImportOrigin::File, name.to_string())],
        };
        let room = MAX_PAGES.saturating_sub(self.session.file().photos.len());
        let mut added = 0;
        for (origin, label) in items.into_iter().take(room) {
            let id = self.session.allocate_photo_id();
            let item = PhotoItem { id, source: source.clone(), name: label.clone(), origin, size: (0, 0), focal: FocalInput::default() };
            self.session.file_mut().photos.push(item.clone());
            self.names.insert(id, label);
            self.status.insert(id, PhotoStatus::Pending);
            self.submit_detect(&item);
            added += 1;
        }
        Ok(added)
    }

    fn import_discard(&mut self, photo: PhotoId) -> Result<(), EngineError> {
        if self.session.photo(photo).is_none() {
            return Err(EngineError::NoPhoto(photo));
        }
        // The pages already in the tray stay: they hold their own recipe and the source stays in the session.
        let pages: Vec<PageId> = self.session.pending().iter().filter(|p| p.photo == photo).map(|p| p.id).collect();
        for id in pages {
            self.cancel_page(id);
        }
        if let Some(t) = self.tokens.remove(&Target::Photo(photo)) {
            t.cancel();
        }
        self.session.pending_mut().retain(|p| p.photo != photo);
        let in_tray = self.session.file().pages.iter().any(|p| p.photo == photo);
        if !in_tray {
            self.session.file_mut().photos.retain(|p| p.id != photo);
            self.status.remove(&photo);
            self.detected.remove(&photo);
            self.names.remove(&photo);
        }
        if self.current_photo == Some(photo) {
            self.current_photo = None;
        }
        if matches!(self.screen, Screen::Crop(p) if p == photo) {
            self.screen = self.next_screen_after(None);
        }
        Ok(())
    }

    // ---- the outline ----

    /// The page and its photo's size, or the error that names what is missing.
    fn page_and_size(&self, id: PageId) -> Result<(Page, (u32, u32)), EngineError> {
        let page = self.session.page(id).ok_or(EngineError::NoPage(id))?.clone();
        let size = self.session.photo(page.photo).map_or((0, 0), |p| p.size);
        Ok((page, size))
    }

    /// Edits a page's outline: the recipe's derived parts are dropped and the page's jobs stop.
    fn edit_outline(&mut self, id: PageId, edit: impl FnOnce(&mut Page), manual: bool) -> Result<(), EngineError> {
        let page = self.session.page_mut(id).ok_or(EngineError::NoPage(id))?;
        edit(page);
        page.proportions = None;
        page.page_size_mm = None;
        page.sheet = None;
        if manual {
            page.corners_from = CornerOrigin::Manual;
        }
        self.cancel_page(id);
        self.loupe = None;
        Ok(())
    }

    fn crop_set_corner(&mut self, id: PageId, corner: Corner, x: f64, y: f64) -> Result<(), EngineError> {
        let (_, size) = self.page_and_size(id)?;
        let p = Point::new(x, y);
        if !p.is_finite() {
            return Err(EngineError::msg("That corner position isn't a number."));
        }
        let p = clamp_corner(p, size);
        self.edit_outline(id, |page| page.corners.0[corner.index()] = p, true)
    }

    fn crop_move_side(&mut self, id: PageId, side: Side, offset: f64) -> Result<(), EngineError> {
        let (page, size) = self.page_and_size(id)?;
        let moved = move_side(&page.corners, side, offset).ok_or_else(|| EngineError::msg("That side can't be moved there."))?;
        let moved = Quad(moved.0.map(|c| clamp_corner(c, size)));
        self.edit_outline(id, |p| p.corners = moved, true)
    }

    fn crop_bend_side(&mut self, id: PageId, side: Side, handles: &[Point; 3]) -> Result<(), EngineError> {
        self.page_and_size(id)?;
        if !handles.iter().all(|h| h.is_finite()) {
            return Err(EngineError::msg("That bend isn't a usable shape."));
        }
        let handles = *handles;
        // A bent side turns Flatten On and Auto-Detect off (§4.3); the outline is judged by the view, never blocked here.
        self.edit_outline(
            id,
            |p| {
                p.sides[side.index()] = SideShape::Bent { handles };
                p.flatten = FlattenMode::On;
            },
            true,
        )
    }

    fn crop_set_flatten(&mut self, id: PageId, mode: FlattenMode) -> Result<(), EngineError> {
        let (page, _) = self.page_and_size(id)?;
        let pdf = self.session.photo(page.photo).is_some_and(|p| matches!(p.origin, ImportOrigin::PdfPage { .. }));
        if mode != FlattenMode::Off && (pdf || matches!(page.corners_from, CornerOrigin::FullImage)) {
            return Err(EngineError::msg("Flatten isn't available for a page cut with Use Full Image or taken from a PDF."));
        }
        self.edit_outline(id, |p| p.flatten = mode, false)
    }

    fn crop_full_image(&mut self, id: PageId) -> Result<(), EngineError> {
        let (page, size) = self.page_and_size(id)?;
        if size == (0, 0) {
            return Err(EngineError::msg("This photo is still being read."));
        }
        if page.spread.is_some() {
            self.join_pending(page.photo)?;
        }
        self.edit_outline(
            id,
            |p| {
                p.corners = full_image_corners(size);
                p.sides = Default::default();
                p.corners_from = CornerOrigin::FullImage;
                p.flatten = FlattenMode::Off;
            },
            false,
        )
    }

    fn crop_auto(&mut self, id: PageId) -> Result<(), EngineError> {
        let (page, _) = self.page_and_size(id)?;
        let Some(found) = self.detected.get(&page.photo).cloned() else {
            // After a restore nothing was kept: detect again; the result is applied when it arrives.
            let item = self.session.photo(page.photo).cloned().ok_or(EngineError::NoPhoto(page.photo))?;
            self.want_auto.insert(id);
            if !self.detect_inflight(page.photo) {
                self.submit_detect(&item);
            }
            return Ok(());
        };
        self.apply_detected_to(id, &found)
    }

    /// Puts the detected outline on a page (Auto-Detect).
    pub(crate) fn apply_detected_to(&mut self, id: PageId, found: &crate::Detected) -> Result<(), EngineError> {
        let page = self.session.page(id).ok_or(EngineError::NoPage(id))?.clone();
        let (was_split, photo) = (page.spread.is_some(), page.photo);
        if was_split {
            self.join_pending(photo)?;
        }
        let (corners, sides, origin) = (found.corners, found.sides.clone(), found.origin);
        let flatten = if self.settings.get().flatten_automatically && !matches!(origin, CornerOrigin::FullImage) {
            FlattenMode::Auto
        } else {
            FlattenMode::Off
        };
        self.edit_outline(
            id,
            |p| {
                p.corners = corners;
                p.sides = sides;
                p.corners_from = origin;
                p.flatten = flatten;
            },
            false,
        )?;
        if was_split || self.session.file().book_spreads {
            self.split_pending(photo)?;
        }
        Ok(())
    }

    // ---- spreads ----

    fn pending_ids(&self, photo: PhotoId) -> Vec<PageId> {
        self.session.pending().iter().filter(|p| p.photo == photo).map(|p| p.id).collect()
    }

    fn tray_holds(&self, photo: PhotoId) -> bool {
        self.session.file().pages.iter().any(|p| p.photo == photo)
    }

    /// Cuts the photo's one pending page into a left and a right page down the middle. A30 isn't
    /// finished, so the spine is the straight line between the middle of the top and bottom sides.
    pub(crate) fn split_pending(&mut self, photo: PhotoId) -> Result<(), EngineError> {
        let ids = self.pending_ids(photo);
        let [id] = ids[..] else {
            // Nothing to cut, or already cut in two: there is no work to do.
            return Ok(());
        };
        let Some(index) = self.session.pending().iter().position(|p| p.id == id) else { return Ok(()) };
        let Some(whole) = self.session.pending().get(index).cloned() else { return Ok(()) };
        if whole.spread.is_some() {
            return Ok(());
        }
        if self.tray_holds(photo) {
            return Err(EngineError::msg("A page of this photo was already added, so it can't be split."));
        }
        let mid = |a: Point, b: Point| Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let q = whole.corners;
        let (top, bottom) = (mid(q.tl(), q.tr()), mid(q.bl(), q.br()));
        let straight = SideShape::Straight;
        let mut left = whole.clone();
        left.corners = Quad([q.tl(), top, bottom, q.bl()]);
        left.sides = [straight.clone(), SideShape::Spine, straight.clone(), whole.sides[3].clone()];
        let mut right = whole.clone();
        right.id = self.session.allocate_page_id();
        right.corners = Quad([top, q.tr(), q.br(), bottom]);
        right.sides = [straight.clone(), whole.sides[1].clone(), straight, SideShape::Spine];
        let half = |half| SpreadHalf { half, spine: [top, bottom], spread_corners: q, confidence: 0.0 };
        left.spread = Some(half(Half::Left));
        right.spread = Some(half(Half::Right));
        for p in [&mut left, &mut right] {
            p.proportions = None;
            p.page_size_mm = None;
            p.sheet = None;
        }
        self.cancel_page(whole.id);
        let pending = self.session.pending_mut();
        pending.remove(index);
        pending.insert(index, right);
        pending.insert(index, left);
        self.loupe = None;
        Ok(())
    }

    /// The reverse: the photo's two pending halves become one page again.
    pub(crate) fn join_pending(&mut self, photo: PhotoId) -> Result<(), EngineError> {
        let ids = self.pending_ids(photo);
        let halves: Vec<Page> = ids.iter().filter_map(|id| self.session.page(*id)).filter(|p| p.spread.is_some()).cloned().collect();
        let Some(left) = halves.iter().find(|p| p.spread.as_ref().is_some_and(|s| s.half == Half::Left)).cloned() else { return Ok(()) };
        let right = halves.iter().find(|p| p.spread.as_ref().is_some_and(|s| s.half == Half::Right)).cloned();
        let Some(spread) = left.spread.clone() else { return Ok(()) };
        let mut whole = left.clone();
        whole.corners = spread.spread_corners;
        whole.spread = None;
        let straight = SideShape::Straight;
        let outer = |s: Option<&SideShape>| match s {
            Some(SideShape::Spine) | None => SideShape::Straight,
            Some(other) => other.clone(),
        };
        whole.sides = [straight.clone(), outer(right.as_ref().map(|r| &r.sides[1])), straight, outer(left.sides.get(3))];
        whole.proportions = None;
        whole.page_size_mm = None;
        whole.sheet = None;
        for id in ids {
            self.cancel_page(id);
        }
        let pending = self.session.pending_mut();
        let at = pending.iter().position(|p| p.id == left.id).unwrap_or(0);
        pending.retain(|p| !(p.photo == photo && p.spread.is_some()));
        pending.insert(at.min(pending.len()), whole);
        self.loupe = None;
        Ok(())
    }

    fn crop_book(&mut self, on: bool) -> Result<(), EngineError> {
        self.session.file_mut().book_spreads = on;
        self.settings.update(|s| s.book_spreads = on);
        let photos: Vec<PhotoId> = self.session.file().photos.iter().map(|p| p.id).filter(|p| !self.tray_holds(*p)).collect();
        for photo in photos {
            if on {
                self.split_pending(photo).or_else(|e| if self.pending_ids(photo).is_empty() { Ok(()) } else { Err(e) })?
            } else {
                self.join_pending(photo)?
            }
        }
        if on {
            self.notify("Finding a book's spine isn't ready yet (A30): each photo is cut down the middle.", false);
        }
        Ok(())
    }

    fn crop_split(&mut self, photo: PhotoId, on: bool) -> Result<(), EngineError> {
        if self.session.photo(photo).is_none() {
            return Err(EngineError::NoPhoto(photo));
        }
        if on {
            if self.pending_ids(photo).is_empty() {
                return Err(EngineError::msg("This photo has no page to split."));
            }
            self.split_pending(photo)?;
            self.notify("Finding a book's spine isn't ready yet (A30): the photo is cut down the middle.", false);
            Ok(())
        } else {
            self.join_pending(photo)
        }
    }

    fn crop_set_spine(&mut self, photo: PhotoId, top: Point, bottom: Point) -> Result<(), EngineError> {
        if self.session.photo(photo).is_none() {
            return Err(EngineError::NoPhoto(photo));
        }
        if !top.is_finite() || !bottom.is_finite() {
            return Err(EngineError::msg("That spine position isn't a number."));
        }
        let size = self.session.photo(photo).map_or((0, 0), |p| p.size);
        let (top, bottom) = (clamp_corner(top, size), clamp_corner(bottom, size));
        let ids: Vec<PageId> = self.pending_ids(photo).into_iter().filter(|id| self.session.page(*id).is_some_and(|p| p.spread.is_some())).collect();
        if ids.is_empty() {
            return Err(EngineError::msg("This photo isn't split into two pages."));
        }
        for id in ids {
            self.edit_outline(
                id,
                |p| {
                    if let Some(s) = p.spread.as_mut() {
                        s.spine = [top, bottom];
                        let c = s.spread_corners;
                        p.corners = if s.half == Half::Left { Quad([c.tl(), top, bottom, c.bl()]) } else { Quad([top, c.tr(), c.br(), bottom]) };
                        s.confidence = 0.0;
                    }
                },
                true,
            )?;
        }
        Ok(())
    }

    // ---- Correct Perspective ----

    fn crop_apply(&mut self, photo: PhotoId) -> Result<(), EngineError> {
        let item = self.session.photo(photo).cloned().ok_or(EngineError::NoPhoto(photo))?;
        if item.size == (0, 0) {
            return Err(EngineError::msg("This photo is still being read."));
        }
        let mut ids = self.pending_ids(photo);
        if ids.is_empty() {
            // The photo has no page yet: one is cut from the outline that was found.
            let corners = self
                .detected
                .get(&photo)
                .map(|d| d.corners)
                .or_else(|| self.fallback_corners(photo))
                .ok_or_else(|| EngineError::msg("This photo is still being read."))?;
            let origin = self.detected.get(&photo).map_or(CornerOrigin::NotFound, |d| d.origin);
            let sides: [SideShape; 4] = self
                .detected
                .get(&photo)
                .map_or_else(|| [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight], |d| d.sides.clone());
            let page = Page {
                id: self.session.allocate_page_id(),
                photo,
                source: item.source.clone(),
                corners,
                corners_from: origin,
                sides,
                spread: None,
                flatten: FlattenMode::Off,
                sheet: None,
                proportions: None,
                page_size_mm: None,
                quarter_turns: 0,
                turns_from: TurnOrigin::Auto,
                look: self.last_look,
                strength: self.last_look.default_strength().unwrap_or(0),
                text: None,
            };
            self.session.pending_mut().push(page);
            ids = self.pending_ids(photo);
        }
        if self.session.file().pages.len() + self.session.pending().len() > MAX_PAGES {
            return Err(EngineError::msg(format!("A session holds at most {MAX_PAGES} pages.")));
        }
        let settings = self.settings.get().clone();
        // Everything is computed first, so an invalid outline changes nothing.
        let mut computed = Vec::new();
        for id in &ids {
            let mut page = self.session.page(*id).ok_or(EngineError::NoPage(*id))?.clone();
            if let Some(reason) = invalid_reason(&page.corners, &page.sides) {
                return Err(EngineError::msg(format!("The outline isn't valid: {reason}.")));
            }
            page.proportions = None;
            page.page_size_mm = None;
            let geometry = page_geometry(&page, item.size, item.focal, &settings)?;
            computed.push((*id, geometry.proportions, geometry.page_size_mm));
        }
        let pdf = matches!(item.origin, ImportOrigin::PdfPage { .. });
        let look = self.last_look;
        let mut to_flatten = Vec::new();
        for (id, proportions, size_mm) in computed {
            let Some(page) = self.session.page_mut(id) else { continue };
            page.proportions = Some(proportions);
            page.page_size_mm = Some(size_mm);
            page.sheet = None;
            page.look = look;
            page.strength = look.default_strength().unwrap_or(0);
            if page.flatten != FlattenMode::Off && !pdf && !matches!(page.corners_from, CornerOrigin::FullImage) {
                // A29 decides in the background, once the page is on screen: the fit and the two
                // straightenings take a moment, and the page shows with its corners meanwhile.
                to_flatten.push(id);
            }
        }
        for id in to_flatten {
            if let Some(page) = self.session.page(id).cloned() {
                self.submit_flatten(&page);
            }
        }
        // Spreads open in reading order (§4.4).
        let order = self.settings.get().book_page_order;
        let first = {
            let halves: Vec<&Page> = ids.iter().filter_map(|id| self.session.page(*id)).collect();
            let wanted = if order == PageOrder::RightToLeft { Half::Right } else { Half::Left };
            halves.iter().find(|p| p.spread.as_ref().is_some_and(|s| s.half == wanted)).or(halves.first()).map(|p| p.id)
        };
        let first = first.ok_or(EngineError::NoPhoto(photo))?;
        if self.settings.get().rotate_upright {
            let pages: Vec<Page> = ids
                .iter()
                .filter_map(|id| self.session.page(*id))
                .filter(|p| p.quarter_turns == 0 && p.turns_from == TurnOrigin::Auto)
                .cloned()
                .collect();
            for p in pages {
                self.submit_upright(&p);
            }
        }
        self.current_photo = Some(photo);
        self.current_page = Some(first);
        self.screen = Screen::Enhance(first);
        Ok(())
    }

    // ---- Enhance ----

    fn page_edit_look(&mut self, id: PageId, edit: impl FnOnce(&mut Page)) -> Result<(), EngineError> {
        let page = self.session.page_mut(id).ok_or(EngineError::NoPage(id))?;
        edit(page);
        // The recognised text belongs to the look it was read from only in as far as the page's pixels do.
        let look = page.look;
        self.last_look = look;
        Ok(())
    }

    fn page_set_look(&mut self, id: PageId, look: Look) -> Result<(), EngineError> {
        self.page_edit_look(id, |p| {
            p.look = look;
            p.strength = look.default_strength().unwrap_or(0);
        })
    }

    fn page_rotate(&mut self, id: PageId, quarter_turns: i8) -> Result<(), EngineError> {
        let page = self.session.page_mut(id).ok_or(EngineError::NoPage(id))?;
        page.quarter_turns = (i32::from(page.quarter_turns) + i32::from(quarter_turns)).rem_euclid(4) as u8;
        page.turns_from = TurnOrigin::Manual;
        // Text positions belong to the page as it was turned.
        page.text = None;
        self.cancel_page(id);
        Ok(())
    }

    fn page_add(&mut self, id: PageId) -> Result<(), EngineError> {
        if self.session.in_tray(id) {
            return Err(EngineError::msg("That page was already added."));
        }
        let index = self.session.pending().iter().position(|p| p.id == id).ok_or(EngineError::NoPage(id))?;
        if self.session.file().pages.len() >= MAX_PAGES {
            return Err(EngineError::msg(format!("A session holds at most {MAX_PAGES} pages.")));
        }
        let page = self.session.pending_mut().remove(index);
        let (photo, look, strength) = (page.photo, page.look, page.strength);
        self.session.file_mut().pages.push(page.clone());
        self.session.file_mut().exported = false;
        self.submit_recognition(&page);
        self.last_look = look;
        // The other half of a spread opens next, on the same look (§4.4).
        let other = self.session.pending().iter().find(|p| p.photo == photo).map(|p| p.id);
        if let Some(other) = other {
            if let Some(p) = self.session.page_mut(other) {
                p.look = look;
                p.strength = strength;
            }
            self.current_page = Some(other);
            self.screen = Screen::Enhance(other);
            return Ok(());
        }
        self.screen = self.next_screen_after(Some(photo));
        Ok(())
    }

    /// Where the user goes once a photo is done: the next photo with a page to correct, else the first screen.
    pub(crate) fn next_screen_after(&mut self, done: Option<PhotoId>) -> Screen {
        let photos: Vec<PhotoId> = self.session.file().photos.iter().map(|p| p.id).collect();
        let start = done.and_then(|d| photos.iter().position(|p| *p == d)).map_or(0, |i| i + 1);
        let next = photos.iter().cycle().skip(start).take(photos.len()).find(|p| Some(**p) != done && !self.pending_ids(**p).is_empty()).copied();
        match next {
            Some(photo) => {
                self.current_photo = Some(photo);
                Screen::Crop(photo)
            }
            None => {
                self.current_photo = None;
                self.current_page = None;
                Screen::Import
            }
        }
    }

    fn page_delete(&mut self, id: PageId) -> Result<(), EngineError> {
        if self.session.page(id).is_none() {
            return Err(EngineError::NoPage(id));
        }
        self.cancel_page(id);
        self.session.file_mut().pages.retain(|p| p.id != id);
        self.session.pending_mut().retain(|p| p.id != id);
        self.images.retain(|k, _| !image_of(k, id));
        self.image_order.retain(|k| !image_of(k, id));
        if matches!(self.screen, Screen::Enhance(p) if p == id) || self.current_page == Some(id) {
            self.current_page = None;
            self.screen = self.next_screen_after(None);
        }
        Ok(())
    }

    fn page_move(&mut self, from: u32, to: u32) -> Result<(), EngineError> {
        let n = self.session.file().pages.len();
        let (from, to) = (from as usize, to as usize);
        if from >= n || to >= n {
            return Err(EngineError::msg(format!("There is no place {} in a tray of {}.", from.max(to) + 1, plural(n, "page", "pages"))));
        }
        let pages = &mut self.session.file_mut().pages;
        let page = pages.remove(from);
        pages.insert(to, page);
        Ok(())
    }

    fn page_edit(&mut self, id: PageId) -> Result<(), EngineError> {
        let page = self.session.page(id).ok_or(EngineError::NoPage(id))?;
        let photo = page.photo;
        self.last_look = page.look;
        self.current_photo = Some(photo);
        self.current_page = Some(id);
        self.screen = Screen::Enhance(id);
        Ok(())
    }

    /// Opens the corners screen for a page, with that page's corners ("Back to Crop", §4.7).
    fn page_open_crop(&mut self, id: PageId) -> Result<(), EngineError> {
        let page = self.session.page(id).ok_or(EngineError::NoPage(id))?;
        let photo = page.photo;
        self.last_look = page.look;
        self.current_photo = Some(photo);
        self.current_page = Some(id);
        self.screen = Screen::Crop(photo);
        Ok(())
    }

    /// Writes the page's corners into a reference set's label file (specification §10.2).
    ///
    /// The label file is `reference.json` in the folder, with one record per photo: the file's
    /// name, the SHA-256 of its bytes, the category, the corners in photo pixels and the page's
    /// ratio. A record for a photo that is already there is replaced, so a photo can be labelled
    /// again after its corners were corrected.
    fn reference_label(&mut self, id: PageId, folder: &str, category: &str) -> Result<(), EngineError> {
        let category = category.trim();
        if category.is_empty() {
            return Err(EngineError::msg("Choose a category for the label."));
        }
        // The loader skips a record whose category is not one of the 18, so a label with any
        // other is refused here, where it can be put right, and not skipped later without a word.
        if !scancraft_core::recipe::REFERENCE_CATEGORIES.contains(&category) {
            return Err(EngineError::msg(format!("{category:?} is not one of the reference set's categories.")));
        }
        let folder = folder.trim();
        if folder.is_empty() {
            return Err(EngineError::msg("Choose the folder the reference set is in."));
        }
        let page = self.session.page(id).ok_or(EngineError::NoPage(id))?;
        let photo = self.session.photo(page.photo).ok_or(EngineError::NoPhoto(page.photo))?;
        // A PDF or TIFF page has no file of its own, and the pages of one file share its name.
        if !matches!(photo.origin, ImportOrigin::File) {
            return Err(EngineError::msg("A page of a PDF or TIFF can't be labelled: label the photo of it instead."));
        }
        // The label names the file and its hash, and the reference loader looks for both in the
        // folder. A label for a photo that isn't there, or has changed, would be skipped when the
        // set is read, without a word, so it is refused here, where the person can fix it.
        let in_folder = std::path::Path::new(folder).join(&photo.name);
        match read_capped(&in_folder) {
            Ok(bytes) if scancraft_io::import::source_id(&bytes) == photo.source.0 => {}
            Ok(_) => {
                return Err(EngineError::msg(format!("{} in that folder is not the photo that was imported.", photo.name)));
            }
            Err(_) => {
                return Err(EngineError::msg(format!("Put {} in that folder first, so the label can find it.", photo.name)));
            }
        }
        // Only what the person marked: the corners, and for a spread the spine. The page's ratio
        // is the pipeline's own estimate (A11), and writing it here would make the ratio gate
        // measure itself. A photo of a book spread is cut into two pages whose corners are halves
        // of it; what the photo's label holds is the whole spread's outline and its spine, which
        // both halves share, so labelling either half writes the same record.
        let (outline, spine) = match &page.spread {
            Some(half) => (half.spread_corners, Some(half.spine)),
            None => (page.corners, None),
        };
        let corners = outline.0.map(|p| [p.x, p.y]);
        let mut record = serde_json::json!({
            "file": photo.name,
            "sha256": photo.source.0,
            "category": category,
            "corners": corners,
        });
        if let (Some(spine), Some(object)) = (spine, record.as_object_mut()) {
            object.insert("spine".to_owned(), serde_json::json!(spine.map(|p| [p.x, p.y])));
        }
        let path = std::path::Path::new(folder).join("reference.json");
        let mut photos: Vec<serde_json::Value> = match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(|_| EngineError::msg("The label file is not text."))?;
                let value: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| EngineError::msg(format!("The label file could not be read: {e}")))?;
                match value.get("photos").and_then(serde_json::Value::as_array) {
                    Some(photos) => photos.clone(),
                    None => Vec::new(),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(EngineError::msg(format!("The label file could not be read: {e}"))),
        };
        // The photo's own record is replaced; the others are kept.
        let name = photo.name.as_str();
        photos.retain(|p| p.get("file").and_then(serde_json::Value::as_str) != Some(name));
        photos.push(record);
        let text = serde_json::to_string_pretty(&serde_json::json!({ "photos": photos })).map_err(|e| EngineError::msg(e.to_string()))?;
        craft_atomic::atomic_write(&path, text.as_bytes()).map_err(|e| EngineError::msg(format!("The label file could not be written: {e}")))?;
        self.notify(format!("{} was labelled as {category}.", photo.name), false);
        Ok(())
    }

    // ---- export ----

    fn export_pdf_command(&mut self, path: &str, options: &ExportOptions) -> Result<(), EngineError> {
        if self.exporting.is_some() {
            return Err(EngineError::msg("A PDF is already being made."));
        }
        if path.trim().is_empty() {
            return Err(EngineError::msg("Choose where to save the PDF."));
        }
        // The page on Screen 3 goes into the tray first (§4.4).
        if let Screen::Enhance(current) = self.screen
            && !self.session.in_tray(current)
        {
            self.page_add(current)?;
        }
        let added = self.session.file().pages.len();
        if added == 0 {
            return Err(EngineError::msg("There are no pages to export. Add a page first."));
        }
        let unadded = self.session.file().photos.iter().filter(|p| !self.tray_holds(p.id)).count();
        if unadded > 0 && !self.export_confirmed {
            self.export_confirmed = true;
            let (photos, verb) = (plural(unadded, "photo", "photos"), if unadded == 1 { "hasn't" } else { "haven't" });
            return Err(EngineError::msg(format!("{photos} {verb} been added. Export the {}?", plural(added, "added page", "added pages"))));
        }
        self.export_confirmed = false;
        self.session.file_mut().export = options.clone();
        self.exporting = Some(self.screen);
        self.screen = Screen::Export;
        self.submit_export(PathBuf::from(path));
        Ok(())
    }

    // ---- sessions ----

    fn session_new(&mut self) -> Result<(), EngineError> {
        self.flush();
        let root = self.root.clone();
        std::fs::create_dir_all(&root).map_err(|e| crate::error::fs_error("the sessions folder could not be made", e))?;
        let stamp = chrono::Local::now().format("Scan %Y-%m-%d at %H.%M.%S").to_string();
        let mut dir = root.join(&stamp);
        let mut n = 2;
        while dir.exists() {
            dir = root.join(format!("{stamp} {n}"));
            n += 1;
        }
        let session = Session::create(dir)?;
        self.replace_session(session);
        let book = self.settings.get().book_spreads;
        self.session.file_mut().book_spreads = book;
        let removed = session::cleanup(&root, KEEP_SESSIONS);
        if !removed.kept_unexported.is_empty() {
            self.notify(format!("{} older sessions were never exported and were kept.", removed.kept_unexported.len()), false);
        }
        self.restore_offer =
            session::list_sessions(&root).into_iter().find(|s| !s.exported && s.pages > 0 && s.dir != *self.session.dir()).map(|s| s.pages);
        Ok(())
    }

    fn session_restore(&mut self) -> Result<(), EngineError> {
        let root = self.root.clone();
        let here = self.session.dir().clone();
        let found = session::list_sessions(&root)
            .into_iter()
            .find(|s| !s.exported && s.pages > 0 && s.dir != here)
            .ok_or_else(|| EngineError::msg("There is no earlier session to restore."))?;
        self.flush();
        let mut session = Session::open(found.dir)?;
        let repairs = session.take_repairs();
        self.replace_session(session);
        for r in repairs {
            self.notify(format!("The saved session was repaired: {r}."), false);
        }
        self.resume();
        self.restore_offer = None;
        self.screen = match self.session.pending().first() {
            Some(p) => {
                self.current_photo = Some(p.photo);
                Screen::Crop(p.photo)
            }
            None => Screen::Import,
        };
        self.book_follow_session();
        Ok(())
    }

    fn book_follow_session(&mut self) {
        let on = self.session.file().book_spreads;
        self.settings.update(|s| s.book_spreads = on);
    }

    /// Swaps in another session: everything that belonged to the old one is dropped, and what its
    /// jobs still return is ignored (their epoch is old).
    fn replace_session(&mut self, session: Session) {
        self.cancel_all();
        let old = std::mem::replace(&mut self.session, session);
        // An empty session that was only just made isn't worth keeping on disk.
        if old.file().photos.is_empty() && old.file().pages.is_empty() && old.pending().is_empty() && old.dir().parent() == Some(self.root.as_path())
        {
            let _ = std::fs::remove_dir_all(old.dir());
        }
        self.epoch += 1;
        self.cache = std::sync::Arc::new(
            crate::cache::Cache::new(self.session.cache_dir()).unwrap_or_else(|_| crate::cache::Cache::unchecked(self.session.cache_dir())),
        );
        self.undo.clear();
        self.screen = Screen::Import;
        self.current_photo = None;
        self.current_page = None;
        self.status.clear();
        self.detected.clear();
        self.images.clear();
        self.image_order.clear();
        self.want_auto.clear();
        self.names.clear();
        self.exporting = None;
        self.export_confirmed = false;
        self.loupe = None;
        self.last_look = Look::ColorDocument;
        self.dirty_again();
    }

    fn dirty_again(&mut self) {
        self.mark_dirty();
        let _ = self.save_now();
    }
}

/// Whether an image key belongs to a page.
fn image_of(key: &scancraft_core::engine_api::ImageKey, page: PageId) -> bool {
    use scancraft_core::engine_api::ImageKey;
    match key {
        ImageKey::LookPreview(p) | ImageKey::FilterThumb(p, _) | ImageKey::TrayThumb(p) => *p == page,
        ImageKey::DisplayCopy(_) => false,
    }
}

// ---- reading files ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Image,
    Tiff,
    Pdf,
}

/// What a file's first bytes say it is; `None` for anything the engine can't import.
fn sniff(bytes: &[u8]) -> Option<Kind> {
    let head = |n: usize| bytes.get(..n);
    let window = bytes.get(..bytes.len().min(1024)).unwrap_or_default();
    if window.windows(5).any(|w| w == b"%PDF-") {
        return Some(Kind::Pdf);
    }
    if head(4).is_some_and(|h| h == b"II*\0" || h == b"MM\0*") {
        return Some(Kind::Tiff);
    }
    let image = head(3).is_some_and(|h| h == [0xFF, 0xD8, 0xFF])
        || head(8).is_some_and(|h| h == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || head(2).is_some_and(|h| h == b"BM")
        || head(6).is_some_and(|h| h == b"GIF87a" || h == b"GIF89a")
        || (head(4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        || (bytes.get(4..8) == Some(b"ftyp")
            && bytes
                .get(8..12)
                .is_some_and(|b| [b"heic", b"heix", b"hevc", b"mif1", b"msf1", b"heim", b"heis", b"avif"].iter().any(|k| b == k.as_slice())));
    image.then_some(Kind::Image)
}

fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("it couldn't be opened ({e})."))?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("it couldn't be read ({e})."))?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err("it is too large to import.".to_string());
    }
    if bytes.is_empty() {
        return Err("the file is empty.".to_string());
    }
    Ok(bytes)
}

/// The files to import: files as listed, and a folder's files in natural order (photo 2 before photo 10).
fn expand_paths(paths: &[String], problems: &mut Vec<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        if path.is_dir() {
            let mut inside: Vec<PathBuf> = match std::fs::read_dir(&path) {
                Ok(rd) => rd
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|f| f.is_file())
                    .filter(|f| !f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')))
                    .collect(),
                Err(e) => {
                    problems.push(format!("{}: the folder couldn't be read ({e}).", path.file_name().and_then(|n| n.to_str()).unwrap_or("A folder")));
                    continue;
                }
            };
            inside.sort_by(|a, b| {
                natural_cmp(
                    &a.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
                    &b.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
                )
            });
            out.extend(inside);
        } else {
            out.push(path);
        }
    }
    out
}

/// Orders names with their numbers as numbers.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits.trim_start_matches('0').to_string()
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let order = da.len().cmp(&db.len()).then_with(|| da.cmp(&db));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_with_their_numbers_as_numbers() {
        let mut names = vec!["img10", "img2", "img1", "IMG_0003", "img02", "a", "img"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["IMG_0003", "a", "img", "img1", "img2", "img02", "img10"]);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
        assert_eq!(natural_cmp("", ""), Ordering::Equal);
        // Very long numbers don't overflow anything.
        assert_eq!(natural_cmp(&"9".repeat(400), &"1".repeat(401)), Ordering::Less);
    }

    #[test]
    fn files_are_told_apart_by_their_first_bytes() {
        assert_eq!(sniff(b"%PDF-1.7\n"), Some(Kind::Pdf));
        assert_eq!(sniff(b"II*\0abcd"), Some(Kind::Tiff));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Kind::Image));
        assert_eq!(sniff(b"just text"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Kind::Image));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), Some(Kind::Image));
    }
}
