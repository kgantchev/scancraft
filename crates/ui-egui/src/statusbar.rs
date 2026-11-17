//! The status bar, along the bottom of the window: how many pages there are, what is going on in
//! the background and how far along it is, and how many pages need a look, with a button that
//! opens the first.

use egui::{Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Progress, ViewState};

use crate::canvas::dispatch;
use crate::sidebar::flagged_pages;
use crate::state::UiState;
use crate::theme;

/// The height of the status bar.
pub const HEIGHT: f32 = 24.0;

/// "Reading text 3 of 4": the work's name and how far along it is.
fn progress_text(state: &UiState, name: &str, progress: Progress) -> String {
    let done = (progress.done + 1).min(progress.total);
    state.text.t(name).replace("{done}", &done.to_string()).replace("{total}", &progress.total.to_string())
}

/// What the bar says, left to right, as plain text: the page count, the work being done and the
/// pages that need a look. (The button and the spinners are drawn with it.)
pub fn summary(state: &UiState, view: &ViewState) -> Vec<String> {
    let mut parts = vec![state.text.trn(view.tray.len() as u64, "{n} page", "{n} pages")];
    if let Some(progress) = view.importing {
        parts.push(progress_text(state, "Reading photo {done} of {total}", progress));
    }
    if let Some(progress) = view.reading_text {
        parts.push(progress_text(state, "Reading text {done} of {total}", progress));
    }
    let flagged = flagged_pages(view).len() as u64;
    if flagged > 0 {
        parts.push(state.text.trn(flagged, "{n} page needs a look", "{n} pages need a look"));
    }
    parts
}

/// Draws the status bar. Every part is a real label, so a screen reader reads what is going on.
pub fn status_bar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let weak = ui.visuals().weak_text_color();
    let small = |text: String| egui::RichText::new(text).size(12.0);
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.label(small(state.text.trn(view.tray.len() as u64, "{n} page", "{n} pages")));
        // Several pages selected together: what Rotate, Delete and the filters will act on.
        let selected = crate::sidebar::targets(state, view).len();
        if selected >= 2 {
            ui.label(small(state.text.trn(selected as u64, "{n} page selected", "{n} pages selected")).color(theme::accent(ui.visuals().dark_mode)));
        }
        // Background work, with something that moves.
        let mut work: Vec<(&str, Progress)> = Vec::new();
        if let Some(p) = view.importing {
            work.push(("Reading photo {done} of {total}", p));
        }
        if let Some(p) = view.reading_text {
            work.push(("Reading text {done} of {total}", p));
        }
        for (name, progress) in work {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                theme::spinner(ui, rect, weak);
                ui.label(small(progress_text(state, name, progress)).color(weak));
            });
        }
        // Pages that need a look, and the way to the first.
        let flagged = flagged_pages(view);
        if let Some(first) = flagged.first() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, theme::NEEDS_LOOK);
                ui.label(small(state.text.trn(flagged.len() as u64, "{n} page needs a look", "{n} pages need a look")));
                let show = state.text.t("Show");
                if ui.add(egui::Button::new(small(show)).small()).clicked() {
                    dispatch(state, engine, Command::PageOpenCrop { page: *first });
                }
            });
        }
    });
}
