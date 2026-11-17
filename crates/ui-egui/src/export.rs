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

/// A switch with a line under it that says what it does, in words a person who has never opened
/// Settings can follow. Returns whether it was changed.
fn switch(ui: &mut Ui, state: &UiState, value: &mut bool, label: &str, explanation: &str) -> bool {
    let changed = ui.checkbox(value, state.text.t(label)).changed();
    // Under the switch, in the same column as its label, in a smaller and quieter type.
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.add(egui::Label::new(egui::RichText::new(state.text.t(explanation)).small().color(ui.visuals().weak_text_color())).wrap());
    });
    ui.add_space(6.0);
    changed
}

/// A choice between a few named values, with its explanation under it.
fn choice<T: Copy + PartialEq>(ui: &mut Ui, state: &UiState, value: &mut T, label: &str, options: &[(T, String)], explanation: &str) -> bool {
    let mut changed = false;
    ui.label(state.text.t(label));
    ui.horizontal_wrapped(|ui| {
        for (option, name) in options {
            if ui.selectable_label(*value == *option, name.as_str()).clicked() && *value != *option {
                *value = *option;
                changed = true;
            }
        }
    });
    ui.add(egui::Label::new(egui::RichText::new(state.text.t(explanation)).small().color(ui.visuals().weak_text_color())).wrap());
    ui.add_space(6.0);
    changed
}

/// The Settings window (§4.6): the settings in groups, each with a line under it saying what it
/// does, and a button for the folder the scans are kept in.
pub fn settings_window(ctx: &egui::Context, state: &mut UiState) {
    let mut open = state.settings_open;
    let mut changed = false;
    let mut settings = state.settings.clone();
    egui::Window::new(state.text.t("Settings"))
        .open(&mut open)
        .resizable(false)
        .default_width(440.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.viewport_rect().center())
        .show(ctx, |ui| {
            ui.set_width(420.0);
            // Pages.
            crate::theme::section_heading(ui, &state.text.t("Pages"));
            let papers: Vec<(Paper, String)> =
                [Paper::IsoA(4), Paper::UsLetter, Paper::UsLegal, Paper::IsoA(5)].into_iter().map(|p| (p, state.text.t(paper_name(p)))).collect();
            changed |= choice(
                ui,
                state,
                &mut settings.default_paper,
                "Default paper",
                &papers,
                "The size of a page that is not close to any standard size, such as a receipt or a page cut from a larger sheet.",
            );
            changed |= switch(
                ui,
                state,
                &mut settings.snap_to_paper_sizes,
                "Snap to standard paper sizes",
                "A page that is close to a standard size, such as A4 or US Letter, is made exactly that size.",
            );
            // Scanning.
            crate::theme::section_heading(ui, &state.text.t("Scanning"));
            changed |= switch(
                ui,
                state,
                &mut settings.rotate_upright_automatically,
                "Rotate pages upright automatically",
                "Turns a page so that its text reads the right way up. You can turn it back from the page's own note.",
            );
            changed |= switch(
                ui,
                state,
                &mut settings.flatten_curved_pages_automatically,
                "Flatten curved pages automatically",
                "Flattens a bent page, as a page of a book is, when that makes its lines of text straighter.",
            );
            changed |= switch(
                ui,
                state,
                &mut settings.keep_paper_colour,
                "Keep paper colour",
                "Tinted paper stays tinted in the PDF, instead of being made white.",
            );
            // Text.
            crate::theme::section_heading(ui, &state.text.t("Text"));
            changed |= switch(
                ui,
                state,
                &mut settings.language_correction,
                "Language correction in text recognition",
                "Uses the language of the page to correct words that were read wrongly, so a search finds them.",
            );
            // Books.
            crate::theme::section_heading(ui, &state.text.t("Books"));
            let orders = vec![(PageOrder::LeftToRight, state.text.t("Left to right")), (PageOrder::RightToLeft, state.text.t("Right to left"))];
            changed |= choice(
                ui,
                state,
                &mut settings.book_page_order,
                "Book page order",
                &orders,
                "The order the two pages of a spread are read in, and so the order they take in the PDF.",
            );
            // Where the scans are kept.
            crate::theme::section_heading(ui, &state.text.t("Scans folder"));
            ui.horizontal(|ui| {
                if ui.button(state.text.t("Choose…")).clicked() {
                    state.actions.push(crate::state::UiAction::ChooseScansFolder);
                }
                ui.add(egui::Label::new(egui::RichText::new(settings.session_storage.clone()).monospace().small()).truncate());
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        state.text.t("New scans are kept here. A scan you have not exported can be continued from the empty window."),
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                )
                .wrap(),
            );
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
