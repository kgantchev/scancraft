//! The finished page (§4.4): the page canvas, the filters' names, and the notes the engine gives
//! about the page. The controls that change the page are in the inspector and the toolbar.

use egui::{Color32, Key, Rect, Sense, TextureOptions, Ui};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, ViewState};
use scancraft_core::recipe::Look;

use crate::canvas;
use crate::state::UiState;
use crate::theme;

/// The filters of the list, in the order of §4.4.
pub const LOOKS: [Look; 4] = [Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument];

/// The name of a filter (§4.4).
pub fn look_name(look: Look) -> &'static str {
    match look {
        Look::Original => "Original",
        Look::BlackWhite => "Black & White",
        Look::Grayscale => "Grayscale",
        Look::ColorDocument => "Color Document",
    }
}

/// Draws the finished page: the canvas, and the keyboard of the view.
pub fn enhance_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let area = ui.available_rect_before_wrap();
    let (rect, response) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
    // The canvas has a name, so the keyboard's focus on it is announced and not silent.
    let name = state.text.t("Page");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, name.clone()));
    if state.enhance_canvas_of != Some(enhance.page) {
        state.enhance_canvas_view = crate::state::CanvasView::default();
        state.enhance_canvas_of = Some(enhance.page);
    }
    let mut zoom_and_pan = state.enhance_canvas_view;
    canvas::navigate(ui, rect, &response, &mut zoom_and_pan);
    state.enhance_canvas_view = zoom_and_pan;
    // Show Original is held by its button, which is drawn before the canvas, or by the \ key.
    let original = state.show_original || ui.input(|i| i.key_down(Key::Backslash));
    page_canvas(ui, state, view, rect, original, engine);
    keyboard(ui, state, view, engine);
}

/// The page canvas: the flattened page in its filter, or with no filter while Show Original is
/// held. It zooms and pans like the crop canvas does.
fn page_canvas(ui: &mut Ui, state: &mut UiState, view: &ViewState, rect: Rect, original: bool, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    let wanted = if original { ImageKey::OriginalPreview(enhance.page) } else { ImageKey::LookPreview(enhance.page) };
    // While the engine renders the page in a new filter, or the unfiltered page for the first
    // time, the canvas keeps the picture it had, so a click on a filter changes the page at once
    // and never blanks it.
    let (texture, image) = match engine.image(wanted) {
        Some(image) => {
            let Some(texture) = state.texture_with(ui, wanted, image.clone(), TextureOptions::LINEAR) else {
                return;
            };
            (texture, image)
        }
        None => {
            let fallback = ImageKey::LookPreview(enhance.page);
            let previous = state.textures.get(&wanted).or_else(|| state.textures.get(&fallback));
            match previous {
                Some(previous) => (previous.handle.clone(), previous.image.clone()),
                None => {
                    // Nothing to show yet: say so, instead of a blank canvas.
                    theme::canvas_background(ui, rect);
                    canvas::waiting(ui, rect, &state.text.t("Preparing the page…"));
                    return;
                }
            }
        }
    };
    theme::canvas_background(ui, rect);
    let (place, applied) = canvas::fit_view(rect, (image.width() as f32, image.height() as f32), state.enhance_canvas_view);
    state.enhance_canvas_view = applied;
    ui.painter_at(rect).image(texture.id(), place, Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
    if original {
        // Say what is on show, so a held button is never mistaken for the finished page.
        let label = state.text.t("Original");
        let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE);
        let chip = Rect::from_min_size(rect.min + egui::vec2(12.0, 12.0), galley.size() + egui::vec2(16.0, 8.0));
        ui.painter().rect_filled(chip, 6.0, Color32::from_black_alpha(190));
        ui.painter().galley(chip.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
    }
}

/// The notes the engine gives about the page (§4.4).
pub fn notes(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else {
        return;
    };
    if enhance.turned_automatically {
        let text = state.text.t("Turned upright automatically");
        ui.horizontal_wrapped(|ui| {
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
        ui.horizontal_wrapped(|ui| {
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

/// Applies the filter of the list's index (§4.4): to the pages selected together when there are
/// several, which start at the filter's own strength, else to the open page.
pub fn apply_look(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, index: usize) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let Some(look) = LOOKS.get(index).copied() else { return };
    let pages = crate::sidebar::targets(state, view);
    if pages.len() >= 2 {
        let strength = look.default_strength().unwrap_or(0);
        canvas::dispatch(state, engine, Command::PageSetLookMany { pages, look, strength });
    } else {
        canvas::dispatch(state, engine, Command::PageSetLook { page: enhance.page, look });
    }
}

/// The keyboard of the finished page: up and down move through the filters, the number keys pick
/// one, and left and right go to the page before and after.
fn keyboard(ui: &Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    // The strength slider, while it has the keyboard's focus, takes the arrow keys itself. (Any
    // widget having the focus is not the test: a click on a filter or on the page gives that
    // widget the focus, and the arrow keys would then stop.)
    if state.keys_taken {
        return;
    }
    let Some(enhance) = view.enhance.as_ref() else { return };
    let current = LOOKS.iter().position(|look| *look == enhance.look).unwrap_or(0);
    let (up, down, left, right) =
        ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight)));
    if up || down {
        let len = LOOKS.len();
        let index = if down { (current + 1) % len } else { (current + len - 1) % len };
        apply_look(state, view, engine, index);
    }
    for (index, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4].into_iter().enumerate() {
        if ui.input(|i| i.key_pressed(key)) {
            apply_look(state, view, engine, index);
        }
    }
    if left || right {
        let page = crate::sidebar::neighbour_page(view, right);
        if let Some(page) = page {
            canvas::dispatch(state, engine, Command::PageEdit { page });
        }
    }
}
