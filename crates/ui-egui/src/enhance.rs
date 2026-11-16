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

/// Draws Screen 3: the page. Its tools are in the tools panel and its buttons in the step bar.
pub fn enhance_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let area = ui.available_rect_before_wrap();
    let (rect, response) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
    if state.enhance_canvas_of != Some(enhance.page) {
        state.enhance_canvas_view = crate::state::CanvasView::default();
        state.enhance_canvas_of = Some(enhance.page);
    }
    let mut zoom_and_pan = state.enhance_canvas_view;
    canvas::navigate(ui, rect, &response, &mut zoom_and_pan);
    state.enhance_canvas_view = zoom_and_pan;
    // A press shows the Original look, until the page is zoomed in: then a drag pans it.
    let pressed = response.is_pointer_button_down_on() && zoom_and_pan.zoom <= canvas::MIN_ZOOM;
    page_canvas(ui, state, view, rect, pressed, engine);
    keyboard(ui, state, view, engine);
}

/// The page canvas: the flattened page, and the Original look while the page is held down
/// (§4.4). It zooms and pans like the crop canvas does.
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
            None => {
                // Nothing to show yet: say so, instead of a blank canvas.
                theme::canvas_background(ui, rect);
                canvas::waiting(ui, rect, &state.text.t("Preparing the page…"));
                return;
            }
        },
    };
    theme::canvas_background(ui, rect);
    let (place, applied) = canvas::fit_view(rect, (image.width() as f32, image.height() as f32), state.enhance_canvas_view);
    state.enhance_canvas_view = applied;
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
pub fn notes(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    if enhance.turned_automatically {
        let text = state.text.t("Turned upright automatically");
        ui.horizontal(|ui| {
            ui.label(text);
            let undo = state.text.t("Undo");
            // The one thing this note is about: turn the page back. The general undo would revert
            // whatever the user did last, which may be something else.
            if ui.button(undo).clicked() {
                let turned = i8::try_from(enhance.quarter_turns % 4).unwrap_or(0);
                canvas::dispatch(state, engine, Command::PageRotate { page: enhance.page, quarter_turns: -turned });
            }
        });
    }
    if enhance.flattened_automatically {
        let text = state.text.t("Flattened curved page");
        ui.horizontal(|ui| {
            ui.label(text);
            let undo = state.text.t("Undo");
            // Leave this page flat: the one thing this note is about.
            if ui.button(undo).clicked() {
                canvas::dispatch(state, engine, Command::CropSetFlatten { page: enhance.page, mode: scancraft_core::recipe::FlattenMode::Off });
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
pub fn apply_look(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, index: usize) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let Some(look) = LOOKS.get(index).copied() else { return };
    state.filter_index = index;
    canvas::dispatch(state, engine, Command::PageSetLook { page: enhance.page, look });
}

/// The keyboard of Screen 3: the arrow keys move the filter strip's selection, and the number keys
/// pick a look (§4.4).
fn keyboard(ui: &Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    // A widget that has the keyboard's focus, the strength slider say, takes the arrow keys and the
    // digits itself; they change the look only when nothing does.
    let taken = ui.memory(|m| m.focused().is_some());
    if !taken {
        arrow_and_number_keys(ui, state, view, engine);
    }
    // The strip's selection follows the page's look.
    if let Some(index) = LOOKS.iter().position(|look| *look == enhance.look) {
        state.filter_index = index;
    }
}

/// The arrow keys move the filter strip's selection, and the number keys pick a look (§4.4).
fn arrow_and_number_keys(ui: &Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
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
}
