//! Where the user is in the scan, and how to move between the stages.
//!
//! Two bars carry it. The **stage bar** at the top shows the four stages, Import, Crop, Enhance
//! and Export, with the one the user is on lit, the ones behind it ticked and the ones ahead
//! dimmed; each is a button that goes there. The **step bar** above the tray holds the buttons
//! that move on or back, and nothing that changes the picture: those live in the tools panel on
//! the left (see [`crate::tools`]).

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen, ViewState};

use crate::canvas::dispatch;
use crate::state::UiState;
use crate::theme::{self, Direction};

/// The height of the stage bar.
pub const STAGE_BAR_HEIGHT: f32 = 52.0;
/// The height of the step bar.
pub const STEP_BAR_HEIGHT: f32 = 56.0;

/// The four stages of a scan, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Choosing the photos.
    Import,
    /// Marking the page's edges.
    Crop,
    /// Choosing the look.
    Enhance,
    /// Writing the PDF.
    Export,
}

impl Stage {
    /// All four, in order.
    pub const ALL: [Stage; 4] = [Stage::Import, Stage::Crop, Stage::Enhance, Stage::Export];

    /// The stage's name.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Import => "Import",
            Stage::Crop => "Crop",
            Stage::Enhance => "Enhance",
            Stage::Export => "Export",
        }
    }

    /// What the stage is for, in a few words.
    pub fn hint(self) -> &'static str {
        match self {
            Stage::Import => "Choose photos",
            Stage::Crop => "Mark the page's edges",
            Stage::Enhance => "Pick a look",
            Stage::Export => "Save the PDF",
        }
    }

    fn index(self) -> usize {
        match self {
            Stage::Import => 0,
            Stage::Crop => 1,
            Stage::Enhance => 2,
            Stage::Export => 3,
        }
    }
}

/// The stage the screen is showing.
pub fn current_stage(state: &UiState, view: &ViewState) -> Stage {
    if state.show_import {
        return Stage::Import;
    }
    match view.screen {
        Screen::Import => Stage::Import,
        Screen::Crop(_) => Stage::Crop,
        Screen::Enhance(_) => Stage::Enhance,
        Screen::Export => Stage::Export,
    }
}

/// Whether the stage can be opened now.
pub fn stage_enabled(stage: Stage, view: &ViewState) -> bool {
    let has_pages = !view.tray.is_empty();
    match stage {
        Stage::Import => true,
        Stage::Crop => view.crop.is_some() || view.enhance.is_some() || has_pages,
        Stage::Enhance => view.enhance.is_some() || has_pages || view.crop.as_ref().is_some_and(|c| c.outlines.iter().all(|o| o.invalid.is_none())),
        Stage::Export => has_pages || view.enhance.is_some(),
    }
}

/// Goes to a stage.
pub fn go_to(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, stage: Stage) {
    if !stage_enabled(stage, view) {
        return;
    }
    let first_page = view.tray.first().map(|item| item.page);
    match stage {
        Stage::Import => state.show_import = true,
        Stage::Crop => {
            state.show_import = false;
            match view.screen {
                Screen::Crop(_) => {}
                Screen::Enhance(page) => {
                    dispatch(state, engine, Command::PageOpenCrop { page });
                }
                _ => {
                    if let Some(page) = first_page {
                        dispatch(state, engine, Command::PageOpenCrop { page });
                    }
                }
            }
        }
        Stage::Enhance => {
            state.show_import = false;
            match view.screen {
                Screen::Enhance(_) => {}
                Screen::Crop(photo) => {
                    dispatch(state, engine, Command::CropApply { photo });
                }
                _ => {
                    if let Some(page) = first_page {
                        dispatch(state, engine, Command::PageEdit { page });
                    }
                }
            }
        }
        Stage::Export => {
            state.request_export(view);
        }
    }
}

/// Forgets the Import overlay once files were added, so the new photo opens.
pub fn track_work(state: &mut UiState, view: &ViewState) {
    let seen = (view.crop.as_ref().map_or(0, |c| c.photos.len()), view.tray.len());
    if seen.0 > state.work_seen.0 && state.show_import {
        state.show_import = false;
    }
    state.work_seen = seen;
}

/// What the user is working on, for the right end of the stage bar.
fn context(state: &UiState, view: &ViewState) -> String {
    let pages = view.tray.len();
    let added = match pages {
        0 => String::new(),
        n => state.text.trn(n as u64, "{n} page added", "{n} pages added"),
    };
    let here = match (current_stage(state, view), view.crop.as_ref()) {
        (Stage::Crop, Some(crop)) => {
            state.text.t("Photo {a} of {b}").replace("{a}", &crop.position.0.to_string()).replace("{b}", &crop.position.1.to_string())
        }
        _ => String::new(),
    };
    [here, added].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ·  ")
}

/// The stage bar: where the user is, and a way to any other stage.
pub fn stage_bar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let current = current_stage(state, view);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STAGE_BAR_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    let weak = ui.visuals().weak_text_color();
    let strong = theme::ink(ui);
    // Cyan cannot be seen on the light window: the marks of this bar are the accent colour.
    let accent = theme::accent(ui.visuals().dark_mode);
    let slot = 168.0f32;
    let left = rect.min.x + 16.0;
    for (i, stage) in Stage::ALL.into_iter().enumerate() {
        let enabled = stage_enabled(stage, view);
        let x = left + slot * i as f32;
        let cell = Rect::from_min_size(Pos2::new(x, rect.min.y + 4.0), Vec2::new(slot - 8.0, STAGE_BAR_HEIGHT - 8.0));
        let id = ui.make_persistent_id(("scancraft.stage", i));
        let response = ui.interact(cell, id, Sense::click());
        let name = state.text.t(stage.name());
        let label = format!("Step {} of 4: {name}", i + 1);
        let is_current = stage == current;
        let done = stage.index() < current.index() && enabled;
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, is_current, label.clone()));
        let hovered = enabled && response.hovered();
        if is_current {
            painter.rect_filled(cell, 8.0, theme::COPPER.gamma_multiply(0.22));
            painter.rect_filled(Rect::from_min_size(Pos2::new(cell.min.x + 6.0, cell.max.y - 3.0), Vec2::new(cell.width() - 12.0, 3.0)), 1.5, accent);
        } else if hovered {
            painter.rect_filled(cell, 8.0, theme::COPPER.gamma_multiply(0.12));
        }
        // The number, or a tick when the stage is behind.
        let c = Pos2::new(cell.min.x + 20.0, cell.center().y);
        if done {
            painter.circle_filled(c, 11.0, accent);
            let tick = Stroke::new(2.0, Color32::WHITE);
            painter.line_segment([c + Vec2::new(-4.5, 0.5), c + Vec2::new(-1.5, 3.5)], tick);
            painter.line_segment([c + Vec2::new(-1.5, 3.5), c + Vec2::new(4.5, -3.5)], tick);
        } else if is_current {
            painter.circle_filled(c, 11.0, theme::COPPER);
            painter.text(c, Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(13.0), Color32::WHITE);
        } else {
            painter.circle_stroke(c, 11.0, Stroke::new(1.5, weak.gamma_multiply(if enabled { 1.0 } else { 0.5 })));
            painter.text(
                c,
                Align2::CENTER_CENTER,
                (i + 1).to_string(),
                FontId::proportional(13.0),
                weak.gamma_multiply(if enabled { 1.0 } else { 0.5 }),
            );
        }
        let text_colour = if is_current {
            strong
        } else if enabled {
            theme::ink(ui)
        } else {
            weak.gamma_multiply(0.6)
        };
        painter.text(Pos2::new(c.x + 20.0, cell.center().y - 8.0), Align2::LEFT_CENTER, name, FontId::proportional(15.0), text_colour);
        let hint = state.text.t(stage.hint());
        painter.text(Pos2::new(c.x + 20.0, cell.center().y + 9.0), Align2::LEFT_CENTER, hint, FontId::proportional(11.0), weak);
        if response.clicked() && enabled && !is_current {
            go_to(state, view, engine, stage);
        }
        // A short line to the next stage.
        if i < 3 {
            let y = cell.center().y;
            painter.line_segment([Pos2::new(cell.max.x + 1.0, y), Pos2::new(cell.max.x + 7.0, y)], Stroke::new(1.0, weak.gamma_multiply(0.6)));
        }
    }
    let text = context(state, view);
    let mut right = rect.max.x - 16.0;
    if !text.is_empty() {
        let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), weak);
        let width = galley.size().x;
        painter.galley(Pos2::new(right - width, rect.center().y - galley.size().y / 2.0), galley, weak);
        right -= width + 16.0;
    }
    // The engine is preparing pages in the background: say so, with something that moves.
    if view.busy {
        let working = state.text.t("Working…");
        let galley = painter.layout_no_wrap(working, FontId::proportional(13.0), weak);
        let width = galley.size().x;
        painter.galley(Pos2::new(right - width, rect.center().y - galley.size().y / 2.0), galley, weak);
        theme::spinner(ui, Rect::from_center_size(Pos2::new(right - width - 14.0, rect.center().y), Vec2::splat(14.0)), weak);
    }
}

/// The photo before or after the one open, when it has a page to open.
pub fn neighbour_page(view: &ViewState, forward: bool) -> Option<scancraft_core::recipe::PageId> {
    let crop = view.crop.as_ref()?;
    let at = crop.photos.iter().position(|tab| tab.photo == crop.photo)?;
    let to = if forward { at.checked_add(1)? } else { at.checked_sub(1)? };
    crop.photos.get(to)?.page
}

/// The step bar: the buttons that move on or back. It changes nothing in the picture.
pub fn step_bar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let current = current_stage(state, view);
    ui.horizontal_centered(|ui| {
        ui.add_space(16.0);
        match current {
            Stage::Import => {
                let can = view.crop.is_some() || view.enhance.is_some() || !view.tray.is_empty();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(16.0);
                    let label = state.text.t("Continue Scanning");
                    if theme::nav_button(ui, &label, Direction::Forward, can).clicked() && can {
                        state.show_import = false;
                        // With nothing waiting in the engine, the way on is the first added page.
                        if matches!(view.screen, Screen::Import)
                            && let Some(item) = view.tray.first()
                        {
                            dispatch(state, engine, Command::PageEdit { page: item.page });
                        }
                    }
                });
            }
            Stage::Crop => {
                let label = state.text.t("Import more");
                if theme::nav_button(ui, &label, Direction::Back, true).clicked() {
                    state.show_import = true;
                }
                if let Some(crop) = view.crop.as_ref() {
                    ui.add_space(24.0);
                    let previous = neighbour_page(view, false);
                    let next = neighbour_page(view, true);
                    let l = state.text.t("Previous Photo");
                    if theme::arrow_button(ui, &l, theme::Arrow::Left).on_hover_text(l.clone()).clicked()
                        && let Some(page) = previous
                    {
                        dispatch(state, engine, Command::PageOpenCrop { page });
                    }
                    let position =
                        state.text.t("Photo {a} of {b}").replace("{a}", &crop.position.0.to_string()).replace("{b}", &crop.position.1.to_string());
                    ui.label(position);
                    let l = state.text.t("Next Photo");
                    if theme::arrow_button(ui, &l, theme::Arrow::Right).on_hover_text(l.clone()).clicked()
                        && let Some(page) = next
                    {
                        dispatch(state, engine, Command::PageOpenCrop { page });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        let valid = crop.outlines.iter().all(|o| o.invalid.is_none());
                        let apply = state.text.t("Correct Perspective");
                        let mut response = theme::nav_button(ui, &apply, Direction::Forward, valid);
                        if let Some(reason) = crop.outlines.iter().find_map(|o| o.invalid.clone()) {
                            response = response.on_hover_text(reason);
                        }
                        if response.clicked() && valid {
                            dispatch(state, engine, Command::CropApply { photo: crop.photo });
                        }
                    });
                }
            }
            Stage::Enhance => {
                if let Some(enhance) = view.enhance.as_ref() {
                    let page = enhance.page;
                    let back = state.text.t("Back to Crop");
                    if theme::nav_button(ui, &back, Direction::Back, true).clicked() {
                        dispatch(state, engine, Command::PageOpenCrop { page });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        let add = state.text.t(if enhance.more_to_come { "Add Page & Next" } else { "Add Page" });
                        if theme::nav_button(ui, &add, Direction::Forward, true).clicked() {
                            dispatch(state, engine, Command::PageAdd { page });
                        }
                    });
                }
            }
            Stage::Export => {}
        }
    });
}
