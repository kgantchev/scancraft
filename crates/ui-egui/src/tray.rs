//! The page tray (§4.5): the bar along the bottom of every screen, with the pages in export order.

use egui::{Key, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};

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
            export_button(ui, state);
            let count = view.tray.len() as u64;
            let text = state.text.trn(count, "{n} page", "{n} pages");
            ui.label(text);
        });
    });
    delete_key(ui, state, view, engine);
}

/// Draws one thumbnail with its page number.
fn draw_thumbnail(ui: &mut Ui, state: &mut UiState, item: &TrayItem, rect: Rect, engine: &mut dyn EngineApi) {
    let texture = state.texture(ui, engine, ImageKey::TrayThumb(item.page), TextureOptions::LINEAR);
    let painter = ui.painter_at(rect);
    if let Some(texture) = texture {
        painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), egui::Color32::WHITE);
    } else {
        painter.rect_filled(rect, 2.0, theme::CANVAS_BACKGROUND);
    }
    if state.tray_selected == Some(item.page) {
        painter.rect_stroke(rect, 2.0, egui::Stroke::new(2.0, theme::SCAN_CYAN), egui::StrokeKind::Middle);
    }
    let number = item.number.to_string();
    painter.text(
        Pos2::new(rect.center().x, rect.max.y + 10.0),
        egui::Align2::CENTER_CENTER,
        number,
        egui::FontId::proportional(12.0),
        ui.visuals().text_color(),
    );
}

/// A click reopens the page, a drag reorders it, and the context menu offers the four commands
/// of §4.5.
fn handle_thumbnail(state: &mut UiState, item: &TrayItem, response: &egui::Response, rects: &[Rect], index: usize, engine: &mut dyn EngineApi) {
    if response.drag_started() {
        state.tray_drag = Some((index as u32, index as u32));
    }
    if response.clicked() {
        if state.tray_drag.is_some() {
            // The pointer was dragged: the drag was the action, not a click.
            state.tray_drag = None;
        } else {
            state.tray_selected = Some(item.page);
            dispatch(state, engine, Command::PageEdit { page: item.page });
        }
    }
    if response.dragged() {
        let from = state.tray_drag.map(|(from, _)| from).unwrap_or(index as u32);
        let to = index_under(rects, response.interact_pointer_pos());
        state.tray_drag = Some((from, to));
        if to != from {
            dispatch(state, engine, Command::PageMove { from, to });
        }
    }
    let page = item.page;
    response.context_menu(|ui| {
        if ui.button("Edit Crop").clicked() {
            dispatch(state, engine, Command::PageOpenCrop { page });
        }
        if ui.button("Rotate Left").clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: -1 });
        }
        if ui.button("Rotate Right").clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: 1 });
        }
        if ui.button("Delete").clicked() {
            dispatch(state, engine, Command::PageDelete { page });
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
    if !view.tray.iter().any(|item| item.page == page) {
        return;
    }
    dispatch(state, engine, Command::PageDelete { page });
    state.tray_selected = None;
}

/// The Export to PDF button and the chevron that opens the export options (§4.5).
fn export_button(ui: &mut Ui, state: &mut UiState) {
    let export = state.text.t("Export to PDF");
    if ui.button(export).clicked() {
        let options = state.export_options.clone();
        state.actions.push(crate::state::UiAction::ExportPdf { options });
    }
    let chevron = state.text.t("Export options");
    let response = theme::arrow_button(ui, &chevron, theme::Arrow::Down).on_hover_text(chevron.clone());
    if response.clicked() {
        state.export_popover = !state.export_popover;
    }
    if state.export_popover {
        egui::Popup::from_response(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            crate::export::export_options_popover(ui, state);
        });
    }
}
