//! The inspector, on the right: the controls of the view the window is in. For the finished page
//! they are the filters, the strength and Show Original; for Crop they are the page's edges, how a
//! curved page is flattened, splitting a photo into two pages, and removing the photo.
//!
//! Every control changes the page as it is touched, and every change can be undone. The one
//! thing the inspector never has is a button that moves on: switching view and Done are in the
//! toolbar and on the keyboard.

use egui::{Pos2, Rect, Sense, TextureOptions, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, ViewState};
use scancraft_core::recipe::FlattenMode;

use crate::canvas::dispatch;
use crate::crop::{auto_detect_tooltip, flatten_tooltip};
use crate::enhance::{LOOKS, look_name};
use crate::state::UiState;
use crate::theme;

/// The width of the inspector.
pub const WIDTH: f32 = 260.0;
/// The box a filter's thumbnail is fitted into.
pub const FILTER_THUMB: Vec2 = Vec2::new(64.0, 84.0);

/// Draws the inspector for the view the window is in.
pub fn inspector(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("scancraft.inspector").show(ui, |ui| {
        if view.crop.is_some() {
            crop_inspector(ui, state, view, engine);
        } else if view.enhance.is_some() {
            filter_inspector(ui, state, view, engine);
        }
    });
}

/// The filter controls: the four filters, their strength, Apply to All Pages and Show Original.
fn filter_inspector(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    let page = enhance.page;
    let title = state.text.t("Filter");
    ui.heading(title);
    ui.add_space(6.0);
    filter_list(ui, state, view, engine);

    if let Some(default) = enhance.look.default_strength() {
        ui.add_space(10.0);
        let heading = state.text.t("Strength");
        let mut strength = f32::from(enhance.strength);
        let response = ui.add(egui::Slider::new(&mut strength, 0.0..=100.0).text(heading));
        // The slider moves with the arrow keys while it has the focus: they are not for the filters.
        state.keys_taken |= response.has_focus();
        if response.changed() {
            let value = strength.clamp(0.0, 100.0).round() as u8;
            dispatch(state, engine, Command::PageSetStrength { page, value });
        }
        // A double-click puts the strength back to where the filter starts.
        if response.double_clicked() {
            dispatch(state, engine, Command::PageSetStrength { page, value: default });
        }
    }

    ui.add_space(8.0);
    let several = view.tray.len() > 1;
    let all = state.text.t("Apply to All Pages");
    if theme::tool_button(ui, &all, several).clicked() {
        dispatch(state, engine, Command::PageSetLookAll { look: enhance.look, strength: enhance.strength });
    }

    // Show Original: held, the page is shown with no filter on it, to see what the filter did.
    let original = state.text.t("Show Original");
    let response = theme::tool_button(ui, &original, true)
        .on_hover_text(state.text.t("Hold to compare with the page without its filter. The \\ key does the same."));
    if response.is_pointer_button_down_on() {
        state.show_original = true;
    }

    ui.add_space(6.0);
    crate::enhance::notes(ui, state, view, engine);
}

/// The filters as a list: a thumbnail at the page's own shape, and the filter's name.
fn filter_list(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(enhance) = view.enhance.as_ref() else { return };
    for (index, look) in LOOKS.iter().enumerate() {
        let name = state.text.t(look_name(*look));
        let selected = enhance.look == *look;
        let (row, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), FILTER_THUMB.y + 8.0), Sense::click());
        let dark = ui.visuals().dark_mode;
        if selected {
            ui.painter().rect_filled(row, 6.0, theme::accent(dark).gamma_multiply(if dark { 0.22 } else { 0.12 }));
        } else if response.hovered() {
            ui.painter().rect_filled(row, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let thumb_box = Rect::from_min_size(row.min + Vec2::new(6.0, 4.0), FILTER_THUMB);
        let texture = state.texture(ui, engine, ImageKey::FilterThumb(enhance.page, *look), TextureOptions::LINEAR);
        match texture {
            Some(texture) => {
                crate::thumb::paint_fitted(&ui.painter_at(row), thumb_box, &texture);
            }
            None => {
                ui.painter_at(row).rect_filled(crate::thumb::fit_inside(thumb_box, [3, 4]), 3.0, theme::CANVAS_BACKGROUND);
            }
        }
        let colour = ui.visuals().text_color();
        ui.painter().text(
            Pos2::new(thumb_box.max.x + 12.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(14.0),
            colour,
        );
        if response.clicked() {
            crate::enhance::apply_look(state, view, engine, index);
        }
        let label = state.text.t("{name} filter").replace("{name}", &name);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label.clone()));
        let _ = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
}

/// The crop controls: the page's edges, flattening, splitting, and removing the photo.
fn crop_inspector(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let Some(crop) = view.crop.as_ref() else { return };
    let Some(outline) = crop.outlines.first().cloned() else { return };
    let photo = crop.photo;
    let title = state.text.t("Crop");
    ui.heading(title);
    let hint =
        state.text.t("Drag a corner onto each corner of the page. Drag the pill on a side to move that side; double-click it to bend the side.");
    ui.label(egui::RichText::new(hint).small().color(ui.visuals().weak_text_color()));

    // The edges. Detection is something to ask for again, not a state to switch on and off.
    let heading = state.text.t("Page edges");
    theme::section_heading(ui, &heading);
    let reset = state.text.t("Reset to Detected Edges");
    let source = state.text.t(&auto_detect_tooltip(&outline.corners_from));
    let response = theme::tool_button(ui, &reset, !crop.auto_detect).on_hover_text(source.clone());
    if response.clicked() && !crop.auto_detect {
        dispatch(state, engine, Command::CropAuto { page: outline.page });
    }
    let whole = state.text.t("Use Whole Photo");
    if theme::tool_button(ui, &whole, true).clicked() {
        dispatch(state, engine, Command::CropFullImage { page: outline.page });
    }
    ui.label(egui::RichText::new(source).small().color(ui.visuals().weak_text_color()));

    // Flattening, with what it did in plain words.
    let heading = state.text.t("Flatten curved pages");
    theme::section_heading(ui, &heading);
    ui.horizontal(|ui| {
        let mut mode = outline.flatten;
        for (option, name) in [(FlattenMode::Auto, "Automatic"), (FlattenMode::On, "Always"), (FlattenMode::Off, "Never")] {
            let name = state.text.t(name);
            if ui.radio_value(&mut mode, option, egui::RichText::new(name).color(theme::ink(ui))).changed() && mode != outline.flatten {
                dispatch(state, engine, Command::CropSetFlatten { page: outline.page, mode });
            }
        }
    });
    let outcome = state.text.t(&flatten_tooltip(&outline));
    ui.label(egui::RichText::new(outcome).small().color(ui.visuals().weak_text_color()));

    // Splitting a photo of a book into its two pages.
    let heading = state.text.t("Two pages");
    theme::section_heading(ui, &heading);
    let split = outline.half.is_some() || crop.outlines.len() > 1;
    let mut on = split;
    let label = state.text.t("Split into two pages");
    if ui.checkbox(&mut on, egui::RichText::new(label).color(theme::ink(ui))).changed() {
        dispatch(state, engine, Command::CropSplit { photo, on });
    }
    if split {
        // Once on, the same cut can be asked for every photo of the scan.
        let mut every = view.book_spreads;
        let label = state.text.t("Do this for every photo in this scan");
        if ui.checkbox(&mut every, egui::RichText::new(label).color(theme::ink(ui)).small()).changed() {
            dispatch(state, engine, Command::CropBook { on: every });
        }
    }

    let heading = state.text.t("This photo");
    theme::section_heading(ui, &heading);
    let remove = state.text.t("Remove Photo");
    if theme::tool_button(ui, &remove, true).clicked() {
        dispatch(state, engine, Command::ImportDiscard { item: photo });
    }
    ui.add_space(12.0);
    let done = state.text.t("Done");
    if theme::tool_button(ui, &done, true).clicked() {
        dispatch(state, engine, Command::CropApply { photo });
    }
}
