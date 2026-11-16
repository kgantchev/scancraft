//! The tools panel: every control that changes the picture or the page, grouped by what it does.
//!
//! The buttons that move to another step are not here; they are in the step bar
//! ([`crate::stepper`]). A tool is a plain grey button, a switch or a slider; only a step button
//! is copper.

use egui::{Sense, TextureOptions, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, ViewState};
use scancraft_core::recipe::FlattenMode;

use crate::canvas::dispatch;
use crate::crop::{auto_detect_tooltip, flatten_tooltip};
use crate::enhance::{LOOKS, look_name};
use crate::state::UiState;
use crate::theme;

/// The width of the panel.
pub const PANEL_WIDTH: f32 = 264.0;
/// The size of a look's thumbnail in the panel.
pub const LOOK_THUMB: Vec2 = Vec2::new(112.0, 150.0);

/// The tools of Screen 2: the page's edges, curved pages, books and the photo.
pub fn crop_tools(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(crop) = view.crop.as_ref() else { return };
    let Some(outline) = crop.outlines.first().cloned() else { return };
    let photo = crop.photo;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let title = state.text.t("Adjust the page");
        ui.heading(title);
        let hint =
            state.text.t("Drag a corner onto each corner of the page. Drag the pill on a side to move that side; double-click it to bend the side.");
        ui.label(egui::RichText::new(hint).small().color(ui.visuals().weak_text_color()));

        let heading = state.text.t("Page edges");
        theme::section_heading(ui, &heading);
        // Detection is something to ask for again, not a state to switch on and off: the control
        // is a button, off while the edges are the detected ones.
        let reset = state.text.t("Reset to Detected Edges");
        let source = state.text.t(&auto_detect_tooltip(&outline.corners_from));
        let response = theme::tool_button(ui, &reset, !crop.auto_detect).on_hover_text(source.clone());
        if response.clicked() && !crop.auto_detect {
            dispatch(state, engine, Command::CropAuto { page: outline.page });
        }
        ui.label(egui::RichText::new(source).small().color(ui.visuals().weak_text_color()));
        let full = state.text.t("Use Full Image");
        if theme::tool_button(ui, &full, true).clicked() {
            dispatch(state, engine, Command::CropFullImage { page: outline.page });
        }

        let heading = state.text.t("Curved pages");
        theme::section_heading(ui, &heading);
        let flatten_label = state.text.t("Flatten");
        ui.label(flatten_label).on_hover_text(flatten_tooltip(&outline));
        ui.horizontal(|ui| {
            let mut mode = outline.flatten;
            for (option, name) in [(FlattenMode::Auto, "Auto"), (FlattenMode::On, "On"), (FlattenMode::Off, "Off")] {
                let name = state.text.t(name);
                if ui.radio_value(&mut mode, option, egui::RichText::new(name).color(theme::ink(ui))).changed() && mode != outline.flatten {
                    dispatch(state, engine, Command::CropSetFlatten { page: outline.page, mode });
                }
            }
        });
        ui.label(egui::RichText::new(flatten_tooltip(&outline)).small().color(ui.visuals().weak_text_color()));

        let heading = state.text.t("Books");
        theme::section_heading(ui, &heading);
        let mut spreads = view.book_spreads;
        let label = state.text.t("Book Spreads");
        if ui.checkbox(&mut spreads, egui::RichText::new(label).color(theme::ink(ui))).changed() {
            dispatch(state, engine, Command::CropBook { on: spreads });
        }
        let split = outline.half.is_some() || crop.outlines.len() > 1;
        if split {
            let join = state.text.t("Join Pages");
            if theme::tool_button(ui, &join, true).clicked() {
                dispatch(state, engine, Command::CropSplit { photo, on: false });
            }
        } else {
            let label = state.text.t("Split Into Two Pages");
            if theme::tool_button(ui, &label, true).clicked() {
                dispatch(state, engine, Command::CropSplit { photo, on: true });
            }
        }

        let heading = state.text.t("This photo");
        theme::section_heading(ui, &heading);
        let discard = state.text.t("Discard Photo");
        if theme::tool_button(ui, &discard, true).clicked() {
            dispatch(state, engine, Command::ImportDiscard { item: photo });
        }
    });
}

/// The tools of Screen 3: the look, its strength and the page's turn.
pub fn enhance_tools(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let page = enhance.page;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let title = state.text.t("Adjust the look");
        ui.heading(title);
        let heading = state.text.t("Look");
        theme::section_heading(ui, &heading);
        look_grid(ui, state, view, engine);

        if enhance.look.default_strength().is_some() {
            let heading = state.text.t("Strength");
            ui.add_space(10.0);
            let mut strength = f32::from(enhance.strength);
            let response = ui.add(egui::Slider::new(&mut strength, 0.0..=100.0).text(heading));
            if response.changed() {
                let value = strength.clamp(0.0, 100.0).round() as u8;
                dispatch(state, engine, Command::PageSetStrength { page, value });
            }
        }

        let heading = state.text.t("Turn");
        theme::section_heading(ui, &heading);
        ui.columns(2, |columns| {
            let left = state.text.t("Rotate Left");
            if theme::tool_button(&mut columns[0], &left, true).clicked() {
                dispatch(state, engine, Command::PageRotate { page, quarter_turns: -1 });
            }
            let right = state.text.t("Rotate Right");
            if theme::tool_button(&mut columns[1], &right, true).clicked() {
                dispatch(state, engine, Command::PageRotate { page, quarter_turns: 1 });
            }
        });
        crate::enhance::notes(ui, state, view, engine);
    });
}

/// The four looks as a grid of thumbnails, two to a row.
fn look_grid(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    egui::Grid::new("scancraft.looks").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
        for (index, look) in LOOKS.iter().enumerate() {
            let name = state.text.t(look_name(*look));
            let selected = state.filter_index == index;
            let response = ui
                .vertical(|ui| {
                    let (rect, response) = ui.allocate_exact_size(LOOK_THUMB, Sense::click());
                    let texture = state.texture(ui, engine, ImageKey::FilterThumb(enhance.page, *look), TextureOptions::LINEAR);
                    // The thumbnail keeps the page's own shape inside its box.
                    let shown = match texture {
                        Some(texture) => crate::thumb::paint_fitted(&ui.painter_at(rect), rect, &texture),
                        None => {
                            ui.painter_at(rect).rect_filled(rect, 4.0, theme::CANVAS_BACKGROUND);
                            rect
                        }
                    };
                    if selected {
                        let accent = theme::accent(ui.visuals().dark_mode);
                        ui.painter_at(rect.expand(2.0)).rect_stroke(shown, 4.0, egui::Stroke::new(3.0, accent), egui::StrokeKind::Outside);
                    }
                    ui.label(name.clone());
                    response
                })
                .inner;
            if response.clicked() {
                crate::enhance::apply_look(state, view, engine, index);
            }
            let label = format!("{name} filter");
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label.clone()));
            if index % 2 == 1 {
                ui.end_row();
            }
        }
    });
}
