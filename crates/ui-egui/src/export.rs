//! The export options and the Settings window (§4.6).

use egui::{Ui, Vec2};

use scancraft_core::recipe::{ExportOptions, PageOrder, PageSizeOption, Paper, Quality};

use crate::state::UiState;

/// The name of a page size (§4.6).
pub fn page_size_name(size: PageSizeOption) -> &'static str {
    match size {
        PageSizeOption::Auto => "Auto",
        PageSizeOption::A4 => "A4",
        PageSizeOption::UsLetter => "US Letter",
        PageSizeOption::UsLegal => "US Legal",
        PageSizeOption::A5 => "A5",
    }
}

/// The name of a quality (§4.6).
pub fn quality_name(quality: Quality) -> &'static str {
    match quality {
        Quality::Small => "Small",
        Quality::Balanced => "Balanced",
        Quality::Best => "Best",
    }
}

/// The name of a paper (§4.6).
pub fn paper_name(paper: Paper) -> &'static str {
    match paper {
        Paper::IsoA(number) => match number {
            4 => "A4",
            5 => "A5",
            _ => "A",
        },
        Paper::UsLetter => "US Letter",
        Paper::UsLegal => "US Legal",
        Paper::Id1 => "ID1",
        Paper::BusinessCard => "Business Card",
    }
}

/// The export options, in a popover from the Export chevron (§4.6).
pub fn export_options_popover(ui: &mut Ui, state: &mut UiState) {
    let mut options = state.export_options.clone();
    ui.label("Export options");
    ui.add_space(4.0);
    // The page size.
    ui.horizontal(|ui| {
        ui.label("Page size");
        for size in [PageSizeOption::Auto, PageSizeOption::A4, PageSizeOption::UsLetter, PageSizeOption::UsLegal, PageSizeOption::A5] {
            let name = state.text.t(page_size_name(size));
            if ui.selectable_label(options.page_size == size, name).clicked() {
                options.page_size = size;
            }
        }
    });
    // The quality.
    ui.horizontal(|ui| {
        ui.label("Quality");
        for quality in [Quality::Small, Quality::Balanced, Quality::Best] {
            let name = state.text.t(quality_name(quality));
            if ui.selectable_label(options.quality == quality, name).clicked() {
                options.quality = quality;
            }
        }
    });
    // The searchable text.
    ui.horizontal(|ui| {
        let mut searchable = options.searchable;
        if ui.checkbox(&mut searchable, "Searchable text").changed() {
            options.searchable = searchable;
        }
    });
    // The text language. The contract has no way to ask the recogniser for the languages it
    // supports, so only Automatic is offered.
    ui.horizontal(|ui| {
        ui.label("Text language");
        ui.label("Automatic");
    });
    ui.add_space(4.0);
    if options != state.export_options {
        state.export_options = options;
    }
}

/// The Settings window (§4.6).
pub fn settings_window(ctx: &egui::Context, state: &mut UiState) {
    let mut open = state.settings_open;
    let mut changed = false;
    let mut settings = state.settings.clone();
    egui::Window::new("Settings").open(&mut open).resizable(false).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Default paper");
            for paper in [Paper::IsoA(4), Paper::UsLetter, Paper::UsLegal, Paper::IsoA(5)] {
                let name = state.text.t(paper_name(paper));
                if ui.selectable_label(settings.default_paper == paper, name).clicked() {
                    settings.default_paper = paper;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            let mut snap = settings.snap_to_paper_sizes;
            if ui.checkbox(&mut snap, "Snap to standard paper sizes").changed() {
                settings.snap_to_paper_sizes = snap;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut rotate = settings.rotate_upright_automatically;
            if ui.checkbox(&mut rotate, "Rotate pages upright automatically").changed() {
                settings.rotate_upright_automatically = rotate;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut flatten = settings.flatten_curved_pages_automatically;
            if ui.checkbox(&mut flatten, "Flatten curved pages automatically").changed() {
                settings.flatten_curved_pages_automatically = flatten;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Book page order");
            for (order, name) in [(PageOrder::LeftToRight, "Left to right"), (PageOrder::RightToLeft, "Right to left")] {
                let name = state.text.t(name);
                if ui.selectable_label(settings.book_page_order == order, name).clicked() {
                    settings.book_page_order = order;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            let mut correction = settings.language_correction;
            if ui.checkbox(&mut correction, "Language correction in text recognition").changed() {
                settings.language_correction = correction;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut colour = settings.keep_paper_colour;
            if ui.checkbox(&mut colour, "Keep paper colour").changed() {
                settings.keep_paper_colour = colour;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Session storage");
            ui.label(settings.session_storage.clone());
        });
    });
    state.settings_open = open;
    if changed {
        state.settings = settings.clone();
        state.actions.push(crate::state::UiAction::SettingsChanged(settings));
    }
}

/// The name the Save panel opens with: "Scan 2026-11-14 at 21.04.07.pdf" (§4.6).
pub fn default_export_file_name(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("Scan %Y-%m-%d at %H.%M.%S.pdf").to_string()
}

/// The default export options (§4.6).
pub fn default_export_options() -> ExportOptions {
    ExportOptions::default()
}

/// The space the settings window needs.
pub fn settings_size() -> Vec2 {
    Vec2::new(420.0, 320.0)
}
