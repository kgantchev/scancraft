//! The export options and the Settings window (§4.6).

use egui::{Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ViewState};
use scancraft_core::recipe::{ExportOptions, PageId, PageOrder, PageSizeOption, Paper, Quality, REFERENCE_CATEGORIES};

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
    ui.label(state.text.t("Export options"));
    ui.add_space(4.0);
    // The page size.
    ui.horizontal(|ui| {
        ui.label(state.text.t("Page size"));
        for size in [PageSizeOption::Auto, PageSizeOption::A4, PageSizeOption::UsLetter, PageSizeOption::UsLegal, PageSizeOption::A5] {
            let name = state.text.t(page_size_name(size));
            if ui.selectable_label(options.page_size == size, name).clicked() {
                options.page_size = size;
            }
        }
    });
    // The quality.
    ui.horizontal(|ui| {
        ui.label(state.text.t("Quality"));
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
        if ui.checkbox(&mut searchable, state.text.t("Searchable text")).changed() {
            options.searchable = searchable;
        }
    });
    // The text language. The contract has no way to ask the recogniser for the languages it
    // supports, so only Automatic is offered.
    ui.horizontal(|ui| {
        ui.label(state.text.t("Text language"));
        ui.label(state.text.t("Automatic"));
    });
    ui.add_space(4.0);
    if options != state.export_options {
        state.export_options = options;
    }
}

/// A file's size for people: "812 KB", "3.4 MB".
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// The last part of a path, for the sheet that names the file.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_owned()
}

/// The sheet over the window for the export: while the PDF is written, how far along it is by page
/// and a Cancel; once it is written, its name, its pages and its size, and what to do with it.
pub fn export_sheet(ctx: &egui::Context, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    match state.export_sheet.clone() {
        crate::state::ExportSheet::Closed => {}
        crate::state::ExportSheet::Running => {
            let title = state.text.t("Exporting PDF");
            egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_width(300.0);
                let (done, total) = view.exporting.map_or((0, 1), |p| (p.done, p.total.max(1)));
                let text = state.text.t("{done} of {total} pages").replace("{done}", &done.to_string()).replace("{total}", &total.to_string());
                // The words are a label, so a screen reader has them; the bar is the picture of them.
                ui.label(text);
                ui.add(egui::ProgressBar::new(done as f32 / total as f32));
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    if ui.button(state.text.t("Cancel")).clicked() {
                        crate::canvas::dispatch(state, engine, scancraft_core::command::Command::ExportCancel);
                    }
                });
            });
        }
        crate::state::ExportSheet::Finished(summary) => {
            let title = state.text.t("PDF exported");
            egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(egui::RichText::new(file_name(&summary.path)).strong());
                let pages = state.text.trn(u64::from(summary.pages), "{n} page", "{n} pages");
                ui.label(format!("{pages} · {}", human_size(summary.bytes)));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(state.text.t("Show in Finder")).clicked() {
                        state.actions.push(crate::state::UiAction::RevealInFinder { path: summary.path.clone() });
                    }
                    if ui.button(state.text.t("Open")).clicked() {
                        state.actions.push(crate::state::UiAction::OpenFile { path: summary.path.clone() });
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(state.text.t("New Scan")).clicked() {
                        crate::canvas::dispatch(state, engine, scancraft_core::command::Command::SessionNew);
                        state.export_sheet = crate::state::ExportSheet::Closed;
                    }
                    if ui.button(state.text.t("Done")).clicked() {
                        state.export_sheet = crate::state::ExportSheet::Closed;
                    }
                });
            });
        }
    }
}

/// The Settings window (§4.6).
pub fn settings_window(ctx: &egui::Context, state: &mut UiState) {
    let mut open = state.settings_open;
    let mut changed = false;
    let mut settings = state.settings.clone();
    egui::Window::new(state.text.t("Settings")).open(&mut open).resizable(false).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(state.text.t("Default paper"));
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
            if ui.checkbox(&mut snap, state.text.t("Snap to standard paper sizes")).changed() {
                settings.snap_to_paper_sizes = snap;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut rotate = settings.rotate_upright_automatically;
            if ui.checkbox(&mut rotate, state.text.t("Rotate pages upright automatically")).changed() {
                settings.rotate_upright_automatically = rotate;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut flatten = settings.flatten_curved_pages_automatically;
            if ui.checkbox(&mut flatten, state.text.t("Flatten curved pages automatically")).changed() {
                settings.flatten_curved_pages_automatically = flatten;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(state.text.t("Book page order"));
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
            if ui.checkbox(&mut correction, state.text.t("Language correction in text recognition")).changed() {
                settings.language_correction = correction;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            let mut colour = settings.keep_paper_colour;
            if ui.checkbox(&mut colour, state.text.t("Keep paper colour")).changed() {
                settings.keep_paper_colour = colour;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(state.text.t("Session storage"));
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

/// The page a label is written for: the one on Screen 2, or else the one on Screen 3.
fn labelled_page(view: &ViewState) -> Option<PageId> {
    view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page)).or_else(|| view.enhance.as_ref().map(|e| e.page))
}

/// The window that writes a photo's corners into a reference set (specification §10.2).
///
/// The person adjusts the corners on Screen 2, as in any scanner, then picks the photo's category
/// and the reference set's folder here. The corners written are the page's own, so a label is
/// exactly what the person saw and dragged.
pub fn label_window(ctx: &egui::Context, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let mut open = state.label_open;
    let page = labelled_page(view);
    egui::Window::new(state.text.t("Label for Reference Set")).open(&mut open).resizable(false).show(ctx, |ui| {
        if page.is_none() {
            ui.label(state.text.t("Open a photo and adjust its corners first."));
            return;
        }
        // The combo box's own label names it for a screen reader, which a separate label does not.
        let current = REFERENCE_CATEGORIES.get(state.label_category).copied().unwrap_or(REFERENCE_CATEGORIES[0]);
        egui::ComboBox::from_label(state.text.t("Category")).selected_text(current).show_ui(ui, |ui| {
            for (index, name) in REFERENCE_CATEGORIES.iter().enumerate() {
                ui.selectable_value(&mut state.label_category, index, *name);
            }
        });
        ui.horizontal(|ui| {
            ui.label(state.text.t("Folder"));
            match &state.label_folder {
                Some(folder) => ui.label(folder.as_str()),
                None => ui.label(state.text.t("No folder chosen")),
            };
            if ui.button(state.text.t("Choose folder…")).clicked() {
                state.actions.push(crate::state::UiAction::ChooseLabelFolder);
            }
        });
        let ready = state.label_folder.is_some();
        if ui.add_enabled(ready, egui::Button::new(state.text.t("Save label"))).clicked()
            && let (Some(page), Some(folder)) = (page, state.label_folder.clone())
        {
            let category = REFERENCE_CATEGORIES.get(state.label_category).copied().unwrap_or(REFERENCE_CATEGORIES[0]).to_owned();
            crate::canvas::dispatch(state, engine, Command::ReferenceLabel { page, folder, category });
        }
    });
    state.label_open = open;
}
