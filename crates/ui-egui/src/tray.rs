//! The page tray (§4.5): the bar along the bottom of every screen, with the pages in export order.

use egui::{Color32, Key, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, TrayItem, ViewState};

use crate::canvas::dispatch;
use crate::state::UiState;
use crate::theme;

/// The height of the tray's bar (§4.5).
pub const BAR_HEIGHT: f32 = 112.0;
/// The size of a thumbnail (§4.5).
pub const THUMB_SIZE: Vec2 = Vec2::new(64.0, 88.0);
/// The gap between two thumbnails.
pub const GAP: f32 = 8.0;

/// Draws the tray.
pub fn page_tray(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    if view.tray.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        let mut rects = Vec::with_capacity(view.tray.len());
        let mut responses = Vec::with_capacity(view.tray.len());
        for item in &view.tray {
            let (rect, _) = ui.allocate_exact_size(THUMB_SIZE, Sense::hover());
            rects.push(rect);
            draw_thumbnail(ui, state, item, rect, engine);
            let id = ui.make_persistent_id(("scancraft.tray", item.page));
            let response = ui.interact(rect, id, Sense::click_and_drag());
            let label = format!("Page {}", item.number);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
            responses.push(response);
        }
        for (index, item) in view.tray.iter().enumerate() {
            handle_thumbnail(state, item, &responses[index], &rects, index, engine);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            export_button(ui, state, view);
            let count = view.tray.len() as u64;
            let text = state.text.trn(count, "{n} page", "{n} pages");
            ui.label(text);
        });
    });
    delete_key(ui, state, view, engine);
}

/// Draws one thumbnail with its page number, at the page's own shape.
fn draw_thumbnail(ui: &mut Ui, state: &mut UiState, item: &TrayItem, rect: Rect, engine: &mut dyn EngineApi) {
    let texture = state.texture(ui, engine, ImageKey::TrayThumb(item.page), TextureOptions::LINEAR);
    let painter = ui.painter_at(rect.expand(3.0));
    let shown = match texture {
        Some(texture) => crate::thumb::paint_fitted(&painter, rect, &texture),
        None => {
            painter.rect_filled(rect, 2.0, theme::CANVAS_BACKGROUND);
            rect
        }
    };
    if state.tray_selected == Some(item.page) {
        let accent = theme::accent(ui.visuals().dark_mode);
        painter.rect_stroke(shown, 2.0, egui::Stroke::new(2.0, accent), egui::StrokeKind::Middle);
    }
    // The number goes inside the picture, on a dark chip: it was under the thumbnail, outside the
    // area it is allowed to paint in, and so it was never seen.
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(item.number.to_string(), font, Color32::WHITE);
    let chip = Rect::from_min_size(
        Pos2::new(shown.min.x + 3.0, shown.max.y - galley.size().y - 7.0),
        Vec2::new(galley.size().x + 8.0, galley.size().y + 4.0),
    );
    painter.rect_filled(chip, 4.0, Color32::from_black_alpha(170));
    painter.galley(chip.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
    // The page's text is still being read: a small spinner says so.
    if !item.text_ready {
        let spot = Rect::from_center_size(Pos2::new(shown.max.x - 10.0, shown.min.y + 10.0), Vec2::splat(14.0));
        painter.circle_filled(spot.center(), 9.0, Color32::from_black_alpha(170));
        theme::spinner(ui, spot, Color32::WHITE);
    }
}

/// A click reopens the page, a drag reorders it, and the context menu offers the four commands
/// of §4.5.
fn handle_thumbnail(state: &mut UiState, item: &TrayItem, response: &egui::Response, rects: &[Rect], index: usize, engine: &mut dyn EngineApi) {
    if response.drag_started() {
        state.tray_drag = Some((index as u32, index as u32));
    }
    if response.dragged() {
        // The engine moves the page at once, so each step starts from where the page is now: the
        // page under the pointer last frame, not the one the drag began on.
        let from = state.tray_drag.map(|(_, at)| at).unwrap_or(index as u32);
        let to = index_under(rects, response.interact_pointer_pos());
        if to != from {
            dispatch(state, engine, Command::PageMove { from, to });
            state.tray_drag = Some((from, to));
        }
    }
    if response.drag_stopped() {
        state.tray_drag = None;
        state.tray_selected = Some(item.page);
    } else if response.clicked() {
        state.tray_selected = Some(item.page);
        dispatch(state, engine, Command::PageEdit { page: item.page });
    }
    let page = item.page;
    response.context_menu(|ui| {
        if ui.button(state.text.t("Edit Crop")).clicked() {
            dispatch(state, engine, Command::PageOpenCrop { page });
        }
        if ui.button(state.text.t("Rotate Left")).clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: -1 });
        }
        if ui.button(state.text.t("Rotate Right")).clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: 1 });
        }
        if ui.button(state.text.t("Delete")).clicked() {
            delete_page(state, engine, page, item.number);
        }
    });
}

/// The index of the thumbnail under the pointer.
fn index_under(rects: &[Rect], pointer: Option<Pos2>) -> u32 {
    let Some(pointer) = pointer else { return 0 };
    for (index, rect) in rects.iter().enumerate() {
        if rect.contains(pointer) {
            return index as u32;
        }
    }
    // The pointer is between two thumbnails: the nearer one wins.
    let mut best = 0u32;
    let mut best_distance = f32::MAX;
    for (index, rect) in rects.iter().enumerate() {
        let distance = (rect.center() - pointer).length();
        if distance < best_distance {
            best_distance = distance;
            best = index as u32;
        }
    }
    best
}

/// The Delete key removes the selected page (§4.5).
fn delete_key(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let pressed = ui.input(|i| i.key_pressed(Key::Backspace) || i.key_pressed(Key::Delete));
    if !pressed {
        return;
    }
    let Some(page) = state.tray_selected else { return };
    let Some(number) = view.tray.iter().find(|item| item.page == page).map(|item| item.number) else {
        return;
    };
    delete_page(state, engine, page, number);
    state.tray_selected = None;
}

/// Deletes a page and says so, with an Undo: nothing else on the screen says that a page has gone,
/// and the Delete key works from every screen.
fn delete_page(state: &mut UiState, engine: &mut dyn EngineApi, page: scancraft_core::recipe::PageId, number: u32) {
    if dispatch(state, engine, Command::PageDelete { page }) {
        let text = state.text.t("Page {n} deleted").replace("{n}", &number.to_string());
        state.toast_with_undo(text);
    }
}

/// The Export to PDF button and the chevron that opens the export options (§4.5).
fn export_button(ui: &mut Ui, state: &mut UiState, view: &ViewState) {
    let export = state.text.t("Export to PDF");
    if ui.button(export).clicked() {
        state.request_export(view);
    }
    let chevron = state.text.t("Export options");
    let response = theme::arrow_button(ui, &chevron, theme::Arrow::Down).on_hover_text(chevron.clone());
    if response.clicked() {
        state.export_popover = !state.export_popover;
    }
    // The popup clears the flag itself when a click outside closes it. Left set, the flag made the
    // menus take a window to be open for good.
    let mut open = state.export_popover;
    egui::Popup::from_response(&response).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        crate::export::export_options_popover(ui, state);
    });
    state.export_popover = open;
}
