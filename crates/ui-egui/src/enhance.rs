//! Screen 3, Filtering and Export (§4.4): the page canvas, the filter strip, the strength slider
//! and the notes the engine gives about the page.

use egui::{Color32, Key, Rect, Sense, TextureOptions, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, ViewState};
use scancraft_core::recipe::Look;

use crate::canvas;
use crate::state::UiState;
use crate::theme;

/// The looks of the filter strip, in the order of §4.4.
pub const LOOKS: [Look; 4] = [Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument];
/// The size of a filter-strip thumbnail (§4.4).
pub const THUMB_SIZE: Vec2 = Vec2::new(112.0, 150.0);
/// The room the filter strip and the strength slider take below the page.
pub const STRIP_HEIGHT: f32 = 210.0;

/// The name of a look (§4.4).
pub fn look_name(look: Look) -> &'static str {
    match look {
        Look::Original => "Original",
        Look::BlackWhite => "Black & White",
        Look::Grayscale => "Grayscale",
        Look::ColorDocument => "Color Document",
    }
}

/// Draws Screen 3.
pub fn enhance_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    if view.enhance.is_none() {
        return;
    }
    enhance_toolbar(ui, state, view, engine);
    let area = ui.available_rect_before_wrap();
    // The filter strip and the slider have their room below the page.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(area.width(), (area.height() - STRIP_HEIGHT).max(120.0)), Sense::click());
    page_canvas(ui, state, view, rect, response.is_pointer_button_down_on(), engine);
    notes(ui, state, view, engine);
    filter_strip(ui, state, view, engine);
    strength_slider(ui, state, view, engine);
    keyboard(ui, state, view, engine);
}

/// The toolbar above the canvas (§4.4).
fn enhance_toolbar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let page = enhance.page;
    ui.horizontal_wrapped(|ui| {
        let rotate_left = state.text.t("Rotate Left");
        if ui.button(rotate_left).clicked() {
            canvas::dispatch(state, engine, Command::PageRotate { page, quarter_turns: -1 });
        }
        let rotate_right = state.text.t("Rotate Right");
        if ui.button(rotate_right).clicked() {
            canvas::dispatch(state, engine, Command::PageRotate { page, quarter_turns: 1 });
        }
        let back = state.text.t("Back to Crop");
        if ui.button(back).clicked() {
            canvas::dispatch(state, engine, Command::PageOpenCrop { page });
        }
        let add = state.text.t(if enhance.more_to_come { "Add Page & Next" } else { "Add Page" });
        if ui.button(add).clicked() {
            canvas::dispatch(state, engine, Command::PageAdd { page });
        }
        let export = state.text.t("Export to PDF");
        if ui.button(export).clicked() {
            let options = state.export_options.clone();
            state.actions.push(crate::state::UiAction::ExportPdf { options });
        }
    });
}

/// The page canvas: the flattened page, and the Original look while the page is held down
/// (§4.4).
fn page_canvas(ui: &mut Ui, state: &mut UiState, view: &ViewState, rect: Rect, pressed: bool, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let key = if pressed {
        // The contract has no preview of the Original look, so the 512 px thumbnail is used.
        ImageKey::FilterThumb(enhance.page, Look::Original)
    } else {
        ImageKey::LookPreview(enhance.page)
    };
    // While the engine renders the page in a new look, the canvas keeps the picture it had, so a
    // click on a look changes the page at once and never blanks it.
    let (texture, image) = match engine.image(key) {
        Some(image) => {
            let Some(texture) = state.texture_with(ui, key, image.clone(), TextureOptions::LINEAR) else {
                return;
            };
            (texture, image)
        }
        None => match state.textures.get(&key) {
            Some(previous) => (previous.handle.clone(), previous.image.clone()),
            None => return,
        },
    };
    theme::canvas_background(ui, rect);
    let size = (image.width() as f32, image.height() as f32);
    let inner = rect.shrink(canvas::PADDING);
    let scale = if size.0 > 0.0 && size.1 > 0.0 { (inner.width() / size.0).min(inner.height() / size.1) } else { 0.0 };
    let shown = Vec2::new(size.0 * scale, size.1 * scale);
    let place = Rect::from_min_size(inner.min + (inner.size() - shown) / 2.0, shown);
    ui.painter_at(rect).image(texture.id(), place, Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
}

/// The filter strip: four thumbnails with their names (§4.4).
pub fn filter_strip(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    ui.horizontal(|ui| {
        for (index, look) in LOOKS.iter().enumerate() {
            let name = state.text.t(look_name(*look));
            let selected = state.filter_index == index;
            let response = ui.vertical(|ui| {
                let (rect, response) = ui.allocate_exact_size(THUMB_SIZE, Sense::click());
                let texture = state.texture(ui, engine, ImageKey::FilterThumb(enhance.page, *look), TextureOptions::LINEAR);
                if let Some(texture) = texture {
                    ui.painter_at(rect).image(texture.id(), rect, Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                if selected {
                    ui.painter_at(rect).rect_stroke(rect, 4.0, egui::Stroke::new(2.0, theme::SCAN_CYAN), egui::StrokeKind::Middle);
                }
                // The look's name is under its thumbnail (§4.4).
                ui.label(name.clone());
                response
            });
            if response.inner.clicked() {
                apply_look(state, view, engine, index);
            }
            let label_text = format!("{name} filter");
            response.inner.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label_text.clone()));
        }
    });
}

/// The strength slider, hidden for the Original look (§4.4).
pub fn strength_slider(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let Some(_default) = enhance.look.default_strength() else {
        return;
    };
    let mut strength = f32::from(enhance.strength);
    let label = state.text.t("Strength");
    let response = ui.add(egui::Slider::new(&mut strength, 0.0..=100.0).text(label));
    if response.changed() {
        let value = strength.clamp(0.0, 100.0).round() as u8;
        canvas::dispatch(state, engine, Command::PageSetStrength { page: enhance.page, value });
    }
}

/// The notes the engine gives about the page (§4.4).
fn notes(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    if enhance.turned_automatically {
        let text = state.text.t("Turned upright automatically");
        ui.horizontal(|ui| {
            ui.label(text);
            let undo = state.text.t("Undo");
            if ui.button(undo).clicked() {
                canvas::dispatch(state, engine, Command::EditUndo);
            }
        });
    }
    if enhance.flattened_automatically {
        let text = state.text.t("Flattened curved page");
        ui.horizontal(|ui| {
            ui.label(text);
            let undo = state.text.t("Undo");
            if ui.button(undo).clicked() {
                canvas::dispatch(state, engine, Command::EditUndo);
            }
        });
    }
    if let Some(half) = enhance.half {
        let text = match half {
            scancraft_core::recipe::Half::Left => state.text.t("Left page of a spread"),
            scancraft_core::recipe::Half::Right => state.text.t("Right page of a spread"),
        };
        ui.label(text);
    }
    if enhance.low_resolution {
        let text = state.text.t("This page is under 150 DPI; small text may be hard to read");
        ui.label(text);
    }
}

/// Applies the look of the filter strip's selection (§4.4).
fn apply_look(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, index: usize) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let Some(look) = LOOKS.get(index).copied() else { return };
    state.filter_index = index;
    canvas::dispatch(state, engine, Command::PageSetLook { page: enhance.page, look });
}

/// The keyboard of Screen 3: the arrow keys move the filter strip's selection, and the number keys
/// pick a look (§4.4).
fn keyboard(ui: &Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let left = ui.input(|i| i.key_pressed(Key::ArrowLeft));
    let right = ui.input(|i| i.key_pressed(Key::ArrowRight));
    if left || right {
        let len = LOOKS.len();
        let index = if right { (state.filter_index + 1) % len } else { (state.filter_index + len - 1) % len };
        apply_look(state, view, engine, index);
    }
    for (index, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4].into_iter().enumerate() {
        if ui.input(|i| i.key_pressed(key)) {
            apply_look(state, view, engine, index);
        }
    }
    // The strip's selection follows the page's look.
    if let Some(index) = LOOKS.iter().position(|look| *look == enhance.look) {
        state.filter_index = index;
    }
}
