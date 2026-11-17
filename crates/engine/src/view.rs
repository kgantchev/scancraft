//! The snapshot the interface draws from (§6.3): built from the session on every `view()` call.

use crate::crop::{invalid_reason, review_reason};
use crate::{Engine, PhotoStatus};
use scancraft_core::engine_api::{CropView, EnhanceView, OutlineView, PageState, Progress, Screen, TrayItem, ViewState};
use scancraft_core::geom::Point;
use scancraft_core::recipe::{CornerOrigin, Page, PhotoId, TurnOrigin};
use scancraft_geom::sheet::unroll;

/// A page below this many dots per inch gets the low-resolution warning (§4.8).
const LOW_RESOLUTION_DPI: f64 = 150.0;

impl Engine {
    pub(crate) fn build_view(&self) -> ViewState {
        let file = self.session.file();
        ViewState {
            screen: self.screen,
            crop: match self.screen {
                Screen::Crop(photo) => self.crop_view(photo),
                _ => None,
            },
            enhance: match self.screen {
                Screen::Enhance(page) => self.enhance_view(page),
                _ => None,
            },
            tray: file
                .pages
                .iter()
                .enumerate()
                .map(|(i, p)| TrayItem { page: p.id, photo: p.photo, number: i as u32 + 1, text_ready: p.text.is_some(), state: self.page_state(p) })
                .collect(),
            book_spreads: file.book_spreads,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            busy: self.jobs.unclaimed() > 0,
            notices: self.visible_notices.clone(),
            restore_offer: self.restore_offer,
            importing: self.importing(),
            reading_text: self.reading_text(),
            exporting: self.exporting.then(|| Progress {
                done: self.export_done.load(std::sync::atomic::Ordering::Relaxed).min(self.export_total),
                total: self.export_total,
            }),
            last_export: self.last_export.clone(),
        }
    }

    /// Where a page is in its life: failed, still being prepared, flagged for a look, or ready.
    fn page_state(&self, page: &Page) -> PageState {
        if let Some(why) = self.page_failures.get(&page.id) {
            return PageState::Failed(why.clone());
        }
        if self.page_busy(page.id) {
            return PageState::Preparing;
        }
        if !self.reviewed.contains(&page.id)
            && let Some(reason) = review_reason(page)
        {
            return PageState::NeedsLook(reason);
        }
        PageState::Ready
    }

    /// The photos the import now running has read, of those it added; `None` when none is running.
    fn importing(&self) -> Option<Progress> {
        let waiting = self.status.values().filter(|s| **s == PhotoStatus::Pending).count() as u32;
        (waiting > 0).then(|| {
            let total = self.batch_total.max(waiting);
            Progress { done: total - waiting, total }
        })
    }

    /// The pages whose text is being read, of those in the document; `None` when all have it.
    fn reading_text(&self) -> Option<Progress> {
        let pages = &self.session.file().pages;
        let reading = pages.iter().filter(|p| self.reading_text_of(p.id)).count() as u32;
        (reading > 0).then(|| {
            let total = pages.len() as u32;
            Progress { done: total.saturating_sub(reading), total }
        })
    }

    fn crop_view(&self, photo: PhotoId) -> Option<CropView> {
        let file = self.session.file();
        let index = file.photos.iter().position(|p| p.id == photo)?;
        let item = file.photos.get(index)?;
        let pages: Vec<&Page> = self.session.pages_of(photo);
        let outlines: Vec<OutlineView> = pages.iter().map(|p| self.outline(p)).collect();
        let first = pages.first();
        let photos = file
            .photos
            .iter()
            .map(|p| {
                let page = self.session.pages_of(p.id).first().map(|q| q.id);
                scancraft_core::engine_api::PhotoTab { photo: p.id, page, name: p.name.clone(), added: page.is_some() }
            })
            .collect();
        Some(CropView {
            photos,
            photo,
            position: (index as u32 + 1, file.photos.len() as u32),
            photo_size: item.size,
            outlines,
            spine: pages.iter().find_map(|p| p.spread.as_ref().map(|s| s.spine)),
            // A30 isn't finished: nothing suggests a split yet.
            suggest_split: false,
            auto_detect: first.is_some_and(|p| matches!(p.corners_from, CornerOrigin::Detected { .. })),
            not_found: first.is_some_and(|p| matches!(p.corners_from, CornerOrigin::NotFound)),
        })
    }

    fn outline(&self, page: &Page) -> OutlineView {
        OutlineView {
            page: page.id,
            corners: page.corners,
            corners_from: page.corners_from,
            sides: page.sides.clone(),
            fitted: self.fitted_outline(page),
            half: page.spread.as_ref().map(|s| s.half),
            invalid: invalid_reason(&page.corners, &page.sides),
            flatten: page.flatten,
            flatten_decision: page.sheet.as_ref().map(|s| s.decision.clone()),
        }
    }

    /// The flattened page's outline in the photo (A28 step 6) as one closed polyline, when there is a fitted sheet.
    fn fitted_outline(&self, page: &Page) -> Option<Vec<Point>> {
        let fit = page.sheet.as_ref()?;
        let geometry = self.geometry_of(page)?;
        let sides = unroll(&fit.sheet, &geometry.camera).ok()?.outline().ok()?;
        let line: Vec<Point> = sides.into_iter().flatten().collect();
        (line.len() >= 3 && line.iter().all(|p| p.is_finite())).then_some(line)
    }

    fn enhance_view(&self, id: scancraft_core::recipe::PageId) -> Option<EnhanceView> {
        let page = self.session.page(id)?;
        let flattened_automatically =
            page.sheet.as_ref().is_some_and(|s| s.decision.flattened) && page.flatten == scancraft_core::recipe::FlattenMode::Auto;
        let low_resolution = match (page.proportions, page.page_size_mm) {
            (Some(p), Some((w_mm, _))) if w_mm > 0.0 => {
                let (w_px, _) = if page.quarter_turns % 2 == 1 { (p.native_size.1, p.native_size.0) } else { p.native_size };
                let width_mm = if page.quarter_turns % 2 == 1 { page.page_size_mm.map_or(w_mm, |s| s.1) } else { w_mm };
                width_mm > 0.0 && f64::from(w_px) / (width_mm / 25.4) < LOW_RESOLUTION_DPI
            }
            _ => false,
        };
        let detecting = self.status.values().any(|s| *s == PhotoStatus::Pending);
        Some(EnhanceView {
            page: id,
            look: page.look,
            strength: page.strength,
            quarter_turns: page.quarter_turns,
            turned_automatically: page.turns_from == TurnOrigin::Auto && page.quarter_turns != 0,
            flattened_automatically,
            half: page.spread.as_ref().map(|s| s.half),
            low_resolution,
            more_to_come: detecting,
        })
    }
}
