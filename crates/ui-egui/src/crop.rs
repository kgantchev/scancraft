//! Crop (§4.3): the original photo with the page's outline, the handles and the loupe, and the
//! keyboard model. Its controls are in the inspector.

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, Sense, Ui, Vec2};

use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EngineApi, ViewState};
use scancraft_core::geom::{Point, Side};
use scancraft_core::recipe::{Detector, FlattenMode, FlattenReason};

use crate::canvas::{self, Canvas};
use crate::state::{Selection, UiState};
use crate::theme;

/// One photo pixel, the step of an arrow key (§4.3).
pub const STEP: f64 = 1.0;
/// The step with Shift held (§4.3).
pub const STEP_SHIFT: f64 = 10.0;
/// The step with Option held (§4.3).
pub const STEP_OPTION: f64 = 0.25;

/// Draws Screen 2.
///
/// A frame handles the pointer and the keyboard first, then reads the view again if that sent the
/// engine a command, and draws from that: the handle, the outline and the loupe are all drawn
/// where the hand has just put the corner, not where it was a frame ago.
pub fn crop_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(crop) = view.crop.as_ref() else {
        return;
    };
    let area = ui.available_rect_before_wrap();
    // The canvas's own widget goes first, so the handles, made after it, are above it.
    let (rect, background) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
    // The canvas has a name, so the keyboard's focus on it is announced and not silent.
    let name = state.text.t("Photo");
    background.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, name.clone()));
    let display = engine.image(scancraft_core::engine_api::ImageKey::DisplayCopy(crop.photo));
    let Some(display) = display else {
        theme::canvas_background(ui, rect);
        canvas::waiting(ui, rect, &state.text.t("Preparing the photo…"));
        return;
    };
    if state.canvas_view_of != Some(crop.photo) {
        state.canvas_view = crate::state::CanvasView::default();
        state.canvas_view_of = Some(crop.photo);
    }
    let mut zoom_and_pan = state.canvas_view;
    canvas::navigate(ui, rect, &background, &mut zoom_and_pan);
    // The room around the photo for a corner that has left it is there only while one has: until
    // then the photo takes everything the area has.
    let margin = if a_corner_is_off_the_photo(crop) { canvas::VIRTUAL_MARGIN } else { 0.0 };
    let canvas = Canvas::with_view(rect, &display, crop.photo_size, zoom_and_pan, margin);
    state.canvas_view = canvas.view;
    // Pointer and keyboard first; the loupe is held open by whatever handle is pressed now.
    state.loupe.held = false;
    let mut changed = canvas::interact_handles(ui, &canvas, crop, state, engine);
    changed |= keyboard(ui, state, view, crop, engine);
    let refreshed;
    let crop = if changed {
        refreshed = engine.view();
        refreshed.crop.as_ref().unwrap_or(crop)
    } else {
        crop
    };
    canvas::draw_canvas(ui, &canvas, crop, state, engine);
    let painter = ui.painter_at(canvas.area);
    for outline in &crop.outlines {
        canvas::draw_outline(&painter, &canvas, outline);
    }
    canvas::draw_handles(ui, &canvas, crop, state);
    if crop.not_found {
        not_found_hint(ui, state, canvas.area);
    }
    if crop.suggest_split {
        split_button(ui, state, crop, engine, canvas.area);
    }
    // The loupe floats above everything, including the tray (§4.3).
    crate::loupe::draw(ui, &canvas, crop, state, engine);
}

/// Whether any corner of any outline is outside the photo.
pub fn a_corner_is_off_the_photo(crop: &CropView) -> bool {
    let (w, h) = (f64::from(crop.photo_size.0), f64::from(crop.photo_size.1));
    crop.outlines.iter().any(|o| o.corners.0.iter().any(|c| c.x < 0.0 || c.y < 0.0 || c.x > w || c.y > h))
}

/// The Auto-Detect tooltip names the source of the corners (§4.3).
pub fn auto_detect_tooltip(origin: &scancraft_core::recipe::CornerOrigin) -> String {
    match origin {
        scancraft_core::recipe::CornerOrigin::Detected { by, .. } => match by {
            Detector::Apple => "Detected by Apple".to_owned(),
            Detector::Contours => "Detected by contours".to_owned(),
            Detector::Lines => "Detected by lines".to_owned(),
        },
        scancraft_core::recipe::CornerOrigin::NotFound => "Not found".to_owned(),
        scancraft_core::recipe::CornerOrigin::Manual => "Adjusted by hand".to_owned(),
        scancraft_core::recipe::CornerOrigin::FullImage => "The whole photo".to_owned(),
    }
}

/// The Flatten tooltip gives the outcome and its reason (§4.3).
pub fn flatten_tooltip(outline: &scancraft_core::engine_api::OutlineView) -> String {
    let Some(decision) = &outline.flatten_decision else {
        return match outline.flatten {
            FlattenMode::Auto => "Automatic: flattened only if its lines or edges are curved.".to_owned(),
            FlattenMode::On => "Always: the page is flattened.".to_owned(),
            FlattenMode::Off => "Never: the page is straightened by its four corners.".to_owned(),
        };
    };
    match decision.reason {
        FlattenReason::Off => "Left flat: flattening is off".to_owned(),
        FlattenReason::On => "Flattened: the page is flattened".to_owned(),
        FlattenReason::OnButNoFit => "Couldn't flatten the curve: no lines of text or curved edges to follow.".to_owned(),
        FlattenReason::NoEvidence | FlattenReason::NoBend => "Left flat: no bend found".to_owned(),
        FlattenReason::NoGain => "Left flat: flattening gained nothing over the flat page".to_owned(),
        FlattenReason::NotStraighter => "Left flat: the lines were not straighter".to_owned(),
        FlattenReason::Straighter => {
            let before = decision.straightness_corners.unwrap_or_default();
            let after = decision.straightness_flat.unwrap_or_default();
            let gain = if before > 0.0 { (after - before) / before * 100.0 } else { 0.0 };
            format!("Flattened: text lines {} % straighter", gain.round() as i64)
        }
        FlattenReason::Unavailable => "Unavailable: a full-image or PDF page cannot be flattened".to_owned(),
    }
}

/// The hint shown when the detection found nothing (§4.3).
pub fn not_found_hint(ui: &mut Ui, state: &UiState, rect: Rect) {
    let text = state.text.t("Couldn't find the page edges. Drag the corners onto the page.");
    // A label over the canvas, so a screen reader reads it too.
    let width = 460.0;
    let strip = Rect::from_min_size(Pos2::new(rect.center().x - width / 2.0, rect.min.y + 16.0), Vec2::new(width, 40.0));
    ui.put(strip, egui::Label::new(egui::RichText::new(text).color(Color32::WHITE)).halign(egui::Align::Center));
}

/// The "Two pages? Split" button in the canvas's corner (§4.3).
fn split_button(ui: &mut Ui, state: &mut UiState, crop: &CropView, engine: &mut dyn EngineApi, canvas: Rect) {
    let rect = Rect::from_min_size(Pos2::new(canvas.right() - 150.0, canvas.top() + 16.0), Vec2::new(134.0, 30.0));
    let label = state.text.t("Two pages? Split");
    let response = ui.interact(rect, ui.make_persistent_id("scancraft.split"), Sense::click());
    let painter = ui.painter_at(rect);
    // A suggestion, not the way on: neutral, so Correct Perspective stays the one copper button.
    let fill = if response.hovered() { Color32::from_rgb(0x48, 0x48, 0x4C) } else { Color32::from_rgb(0x3A, 0x3A, 0x3D) };
    painter.rect_filled(rect, 6.0, fill);
    painter.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, Color32::from_white_alpha(70)), egui::StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label.clone(), FontId::proportional(14.0), Color32::WHITE);
    if response.clicked() {
        canvas::dispatch(state, engine, Command::CropSplit { photo: crop.photo, on: true });
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
}

/// The items Tab walks through: the four corners, then the one vertex of each bent side, then the
/// spine's two ends (§4.3).
pub fn selectable_items(crop: &CropView) -> Vec<Selection> {
    let mut items = Vec::new();
    let Some(outline) = crop.outlines.first() else { return items };
    for corner in [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft] {
        items.push(Selection::Corner { page: outline.page, corner });
    }
    for side in Side::ALL {
        if !canvas::side_is_curved(outline, side) {
            continue;
        }
        items.push(Selection::Bend { page: outline.page, side, index: canvas::BEND_VERTEX });
    }
    if crop.spine.is_some() {
        items.push(Selection::Spine { photo: crop.photo, end: 0 });
        items.push(Selection::Spine { photo: crop.photo, end: 1 });
    }
    items
}

/// The command an arrow key runs: the selected corner, bend handle or spine end moves by the
/// step (§4.3).
pub fn move_selection(crop: &CropView, selection: &Selection, dx: f64, dy: f64) -> Option<Command> {
    match *selection {
        Selection::Corner { page, corner } => {
            let outline = crop.outlines.iter().find(|o| o.page == page)?;
            let point = crate::state::corner_point(&outline.corners, corner);
            Some(Command::CropSetCorner { page, corner, x: point.x + dx, y: point.y + dy })
        }
        Selection::Bend { page, side, index } => {
            let outline = crop.outlines.iter().find(|o| o.page == page)?;
            let handles = canvas::bend_handles(outline, side);
            let handle = handles.get(index)?;
            let moved = canvas::bend_through(outline, side, Point::new(handle.x + dx, handle.y + dy));
            Some(Command::CropBendSide { page, side, handles: moved })
        }
        Selection::Spine { photo, end } => {
            let spine = crop.spine?;
            let (mut top, mut bottom) = (spine[0], spine[1]);
            if end == 0 {
                top = Point::new(top.x + dx, top.y + dy);
            } else {
                bottom = Point::new(bottom.x + dx, bottom.y + dy);
            }
            Some(Command::CropSetSpine { photo, top, bottom })
        }
    }
}

/// The keyboard model of Crop (§4.3). Returns whether a command went to the engine.
///
/// Tab walks the handles and the arrow keys nudge the selected one. With no handle selected the
/// arrow keys go to the next page that needs a look (the one before, with left and up), or to the
/// next page when none does. Return is Done, and so is Escape once there is nothing selected to let go of.
fn keyboard(ui: &Ui, state: &mut UiState, view: &ViewState, crop: &CropView, engine: &mut dyn EngineApi) -> bool {
    let now = ui.input(|i| i.time);
    let items = selectable_items(crop);
    let (tab, shift, escape, enter) =
        ui.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift, i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
    if escape && state.selection.is_some() {
        state.selection = None;
    } else if (escape || enter) && !state.keys_taken {
        // Done: the page is finished and the finished page shows.
        return canvas::dispatch(state, engine, Command::CropApply { photo: crop.photo });
    }
    if tab && !items.is_empty() {
        let next = match state.selection {
            None => {
                if shift {
                    items.len() - 1
                } else {
                    0
                }
            }
            Some(current) => {
                let index = items.iter().position(|item| *item == current).unwrap_or(0);
                let len = items.len();
                if shift { (index + len - 1) % len } else { (index + 1) % len }
            }
        };
        state.selection = items.get(next).copied();
    }
    let step = ui.input(|i| {
        if i.modifiers.shift {
            STEP_SHIFT
        } else if i.modifiers.alt {
            STEP_OPTION
        } else {
            STEP
        }
    });
    let (left, right, up, down) =
        ui.input(|i| (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
    let (dx, dy) = match (left, right, up, down) {
        (true, _, _, _) => (-step, 0.0),
        (_, true, _, _) => (step, 0.0),
        (_, _, true, _) => (0.0, -step),
        (_, _, _, true) => (0.0, step),
        _ => (0.0, 0.0),
    };
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let Some(selection) = state.selection else {
        // Nothing to nudge: the arrows walk through the pages, the ones that need a look first.
        let forward = right || down;
        let to = crate::sidebar::next_flagged(view, forward).or_else(|| crate::sidebar::neighbour_page(view, forward));
        return to.is_some_and(|page| canvas::dispatch(state, engine, Command::PageOpenCrop { page }));
    };
    let Some(command) = move_selection(crop, &selection, dx, dy) else { return false };
    canvas::dispatch(state, engine, command);
    // The loupe shows the handle that moved, where it has moved to (§4.3). A nudge has no button
    // to release, so the loupe stays for a second.
    if let Some(point) = selection_point(crop, &selection) {
        state.show_loupe(now, Point::new(point.x + dx, point.y + dy));
    }
    true
}

/// Where a selection is in the photo, in photo pixels, for the loupe after a keyboard nudge.
pub fn selection_point(crop: &CropView, selection: &Selection) -> Option<Point> {
    match *selection {
        Selection::Corner { page, corner } => {
            let outline = crop.outlines.iter().find(|o| o.page == page)?;
            Some(crate::state::corner_point(&outline.corners, corner))
        }
        Selection::Bend { page, side, index } => {
            let outline = crop.outlines.iter().find(|o| o.page == page)?;
            canvas::bend_handles(outline, side).get(index).copied()
        }
        Selection::Spine { photo, end } => {
            if crop.photo != photo {
                return None;
            }
            crop.spine?.get(end).copied()
        }
    }
}
