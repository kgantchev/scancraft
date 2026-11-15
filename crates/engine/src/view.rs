//! The snapshot the interface draws from (§6.3): built from the session on every `view()` call.

use crate::crop::invalid_reason;
use crate::{Engine, PhotoStatus};
use scancraft_core::engine_api::{CropView, EnhanceView, OutlineView, Screen, TrayItem, ViewState};
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
            tray: file.pages.iter().enumerate().map(|(i, p)| TrayItem { page: p.id, number: i as u32 + 1, text_ready: p.text.is_some() }).collect(),
            book_spreads: file.book_spreads,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            busy: self.jobs.pending() > 0,
            notices: self.visible_notices.clone(),
            restore_offer: self.restore_offer,
        }
    }

    fn crop_view(&self, photo: PhotoId) -> Option<CropView> {
        let file = self.session.file();
        let index = file.photos.iter().position(|p| p.id == photo)?;
        let item = file.photos.get(index)?;
        let pages: Vec<&Page> = self.session.pending().iter().filter(|p| p.photo == photo).collect();
        let outlines: Vec<OutlineView> = pages.iter().map(|p| self.outline(p)).collect();
        let first = pages.first();
        Some(CropView {
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
        let others = self.session.pending().iter().any(|p| p.id != id);
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
            more_to_come: others || detecting,
        })
    }
}
