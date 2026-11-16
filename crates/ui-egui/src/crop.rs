//! Screen 2, Boundary Detection and Cropping (§4.3): the toolbar, the canvas, the handles and the
//! keyboard model.

use egui::{Align2, Color32, FontId, Key, Pos2, Rect, Sense, Ui, Vec2};

use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EngineApi, ImageKey, ViewState};
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
    photo_strip(ui, state, crop, engine);
    let area = ui.available_rect_before_wrap();
    // The canvas's own widget goes first, so the handles, made after it, are above it.
    let (rect, background) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
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
    let canvas = Canvas::with_view(rect, &display, crop.photo_size, zoom_and_pan);
    state.canvas_view = canvas.view;
    // Pointer and keyboard first; the loupe is held open by whatever handle is pressed now.
    state.loupe.held = false;
    let mut changed = canvas::interact_handles(ui, &canvas, crop, state, engine);
    changed |= keyboard(ui, state, crop, &canvas, engine);
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

/// The size of a photo's thumbnail in the strip.
const STRIP_THUMB: Vec2 = Vec2::new(48.0, 64.0);

/// The strip of every photo of the batch, so any one can be opened and corrected, in any order. A
/// photo whose pages are all in the tray has a tick. It is not shown for a single photo.
pub fn photo_strip(ui: &mut Ui, state: &mut UiState, crop: &CropView, engine: &mut dyn EngineApi) {
    if crop.photos.len() < 2 {
        return;
    }
    ui.add_space(4.0);
    egui::ScrollArea::horizontal().id_salt("scancraft.photo_strip").auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (index, tab) in crop.photos.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(STRIP_THUMB, Sense::hover());
                let painter = ui.painter_at(rect.expand(3.0));
                match state.texture(ui, engine, ImageKey::PhotoThumb(tab.photo), egui::TextureOptions::LINEAR) {
                    Some(texture) => {
                        painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(rect, 2.0, theme::CANVAS_BACKGROUND);
                    }
                }
                if tab.photo == crop.photo {
                    painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0, theme::SCAN_CYAN), egui::StrokeKind::Middle);
                }
                if tab.added {
                    // A tick, drawn: the interface's font has no glyph for one.
                    let c = rect.right_top() + Vec2::new(-9.0, 9.0);
                    painter.circle_filled(c, 7.0, theme::HANDLE);
                    let stroke = egui::Stroke::new(2.0, theme::SCAN_CYAN);
                    painter.line_segment([c + Vec2::new(-3.5, 0.0), c + Vec2::new(-1.0, 3.0)], stroke);
                    painter.line_segment([c + Vec2::new(-1.0, 3.0), c + Vec2::new(4.0, -3.5)], stroke);
                }
                let id = ui.make_persistent_id(("scancraft.photo_tab", tab.photo));
                let response = ui.interact(rect, id, Sense::click());
                let label = format!("Photo {} of {}: {}", index + 1, crop.photos.len(), tab.name);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, tab.page.is_some(), label.clone()));
                let response = response.on_hover_text(tab.name.clone());
                if response.clicked()
                    && tab.photo != crop.photo
                    && let Some(page) = tab.page
                {
                    canvas::dispatch(state, engine, Command::PageOpenCrop { page });
                }
            }
        });
    });
    ui.add_space(4.0);
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
            FlattenMode::Auto => "Auto: the page is flattened when the checks pass".to_owned(),
            FlattenMode::On => "On: the page is always flattened".to_owned(),
            FlattenMode::Off => "Off: the page is flattened by its four corners".to_owned(),
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
    painter.rect_filled(rect, 6.0, theme::COPPER);
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

/// The keyboard model of Screen 2 (§4.3). Returns whether a command went to the engine.
fn keyboard(ui: &Ui, state: &mut UiState, crop: &CropView, _canvas: &Canvas, engine: &mut dyn EngineApi) -> bool {
    let now = ui.input(|i| i.time);
    let items = selectable_items(crop);
    let (tab, shift, escape) = ui.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift, i.key_pressed(Key::Escape)));
    if escape {
        state.selection = None;
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
    let Some(selection) = state.selection else { return false };
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
