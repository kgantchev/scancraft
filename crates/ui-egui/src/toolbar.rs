//! The toolbar (§4.1), along the top of the window: Add; a switch between the finished page
//! (Filter) and Crop; Rotate Left, Rotate Right and Delete; Undo and Redo; and, at the far end,
//! Export PDF, the one copper button of the window, with the export options behind a chevron.
//!
//! The groups keep their widths; when the window is too narrow for all of them, the ones that do
//! not fit move to a menu at the end of the row (`craft_ui::layout::fit_toolbar` decides
//! which), and nothing is hidden for good.

use craft_ui::buttons::IconButton;
use egui::{Rect, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ViewState};

use crate::canvas::dispatch;
use crate::icons;
use crate::sidebar::{self, Mode};
use crate::state::{UiAction, UiState};
use crate::theme;

/// The height of the toolbar.
pub const HEIGHT: f32 = 48.0;
/// The size of an icon button.
const BUTTON: f32 = 32.0;
/// The space between two buttons, and between two groups.
const GAP: f32 = 4.0;
const GROUP_GAP: f32 = 14.0;
/// What Export PDF and its chevron take at the far end, set aside before the groups are fitted.
const EXPORT_WIDTH: f32 = 160.0;

/// An icon button's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Add,
    RotateLeft,
    RotateRight,
    Delete,
    Undo,
    Redo,
}

impl Action {
    /// The name it has on screen and for a screen reader.
    pub fn label(self) -> &'static str {
        match self {
            Action::Add => "Add",
            Action::RotateLeft => "Rotate Left",
            Action::RotateRight => "Rotate Right",
            Action::Delete => "Delete",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
        }
    }

    /// Whether it can act on what the window shows.
    pub fn enabled(self, view: &ViewState) -> bool {
        match self {
            Action::Add => true,
            Action::RotateLeft | Action::RotateRight | Action::Delete => sidebar::current_page(view).is_some(),
            Action::Undo => view.can_undo,
            Action::Redo => view.can_redo,
        }
    }

    fn paint(self, painter: &egui::Painter, rect: Rect, colour: egui::Color32) {
        match self {
            Action::Add => icons::plus(painter, rect, colour),
            Action::RotateLeft => icons::rotate_left(painter, rect, colour),
            Action::RotateRight => icons::rotate_right(painter, rect, colour),
            Action::Delete => icons::trash(painter, rect, colour),
            Action::Undo => icons::undo(painter, rect, colour),
            Action::Redo => icons::redo(painter, rect, colour),
        }
    }

    /// Does it: to the pages selected together when there are several, else to the open page.
    pub fn run(self, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
        let pages = sidebar::targets(state, view);
        match self {
            Action::Add => state.actions.push(UiAction::ImportFiles),
            Action::RotateLeft | Action::RotateRight => {
                let quarter_turns = if self == Action::RotateLeft { -1 } else { 1 };
                sidebar::rotate_pages(state, engine, &pages, quarter_turns);
            }
            Action::Delete => sidebar::delete_pages(state, view, engine, &pages),
            Action::Undo => {
                dispatch(state, engine, Command::EditUndo);
            }
            Action::Redo => {
                dispatch(state, engine, Command::EditRedo);
            }
        }
    }
}

/// The groups of icon buttons, in order, with the switch between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Add,
    Mode,
    Page,
    History,
}

impl Group {
    const ALL: [Group; 4] = [Group::Add, Group::Mode, Group::Page, Group::History];

    fn actions(self) -> &'static [Action] {
        match self {
            Group::Add => &[Action::Add],
            Group::Mode => &[],
            Group::Page => &[Action::RotateLeft, Action::RotateRight, Action::Delete],
            Group::History => &[Action::Undo, Action::Redo],
        }
    }
}

/// Switches to the finished page or to Crop.
pub fn switch_mode(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, to: Mode) {
    if sidebar::mode_of(view) == to {
        return;
    }
    match to {
        // Leaving Crop is Done: the page is finished, its flattening decided, its text read again.
        Mode::Filter => {
            if let Some(crop) = view.crop.as_ref() {
                dispatch(state, engine, Command::CropApply { photo: crop.photo });
            }
        }
        Mode::Crop => {
            if let Some(page) = sidebar::current_page(view) {
                dispatch(state, engine, Command::PageOpenCrop { page });
            }
        }
    }
}

/// Draws the toolbar.
pub fn toolbar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let dark = ui.visuals().dark_mode;
    let style = theme::icon_button_style(dark);
    let mode = sidebar::mode_of(view);
    let filter = state.text.t("Filter");
    let crop = state.text.t("Crop");
    let mode_width = {
        let font = egui::FontId::proportional(14.0);
        [&filter, &crop].iter().map(|l| ui.painter().layout_no_wrap((*l).clone(), font.clone(), egui::Color32::WHITE).size().x + 24.0).sum::<f32>()
    };
    let natural: Vec<f32> = Group::ALL
        .iter()
        .map(|g| match g {
            Group::Mode => mode_width,
            other => other.actions().len() as f32 * BUTTON + other.actions().len().saturating_sub(1) as f32 * GAP,
        })
        .collect();
    let available = (ui.available_width() - EXPORT_WIDTH - 24.0).max(0.0);
    let fit = craft_ui::layout::fit_toolbar(&natural, available, GROUP_GAP, BUTTON).unwrap_or_default();

    craft_ui::direction::horizontal(ui, |ui| {
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing = Vec2::new(GAP, 0.0);
        let mut shown = 0;
        for (index, _) in &fit.shown {
            let Some(group) = Group::ALL.get(*index) else { continue };
            if shown > 0 {
                ui.add_space(GROUP_GAP - GAP);
            }
            shown += 1;
            match group {
                Group::Mode => {
                    let has_page = sidebar::current_page(view).is_some();
                    let on = usize::from(mode == Mode::Crop);
                    if let Some(chosen) = theme::segmented(ui, [&filter, &crop], on, [has_page, has_page]) {
                        switch_mode(state, view, engine, if chosen == 0 { Mode::Filter } else { Mode::Crop });
                    }
                }
                group => {
                    for action in group.actions() {
                        icon_button(ui, state, view, engine, &style, *action);
                    }
                }
            }
        }
        if !fit.overflow.is_empty() {
            ui.add_space(GROUP_GAP - GAP);
            overflow_menu(ui, state, view, engine, &style, &fit.overflow, [&filter, &crop]);
        }
        // The far end: Export PDF, with the options behind a chevron.
        ui.with_layout(craft_ui::direction::trailing(ui), |ui| {
            ui.add_space(12.0);
            export_button(ui, state, view, &style);
        });
    });
}

/// One icon button, with its name for a tooltip and a screen reader.
fn icon_button(
    ui: &mut Ui,
    state: &mut UiState,
    view: &ViewState,
    engine: &mut dyn EngineApi,
    style: &craft_ui::buttons::IconButtonStyle,
    action: Action,
) {
    let label = state.text.t(action.label());
    let enabled = action.enabled(view);
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            IconButton::new(&label, Vec2::splat(BUTTON), style).show(ui, |ui, rect, colour| action.paint(ui.painter(), rect, colour))
        })
        .inner
        .on_hover_text(label.clone());
    if response.clicked() && enabled {
        action.run(state, view, engine);
    }
}

/// The menu at the end of the row, with what did not fit.
fn overflow_menu(
    ui: &mut Ui,
    state: &mut UiState,
    view: &ViewState,
    engine: &mut dyn EngineApi,
    style: &craft_ui::buttons::IconButtonStyle,
    overflow: &[usize],
    mode_labels: [&String; 2],
) {
    let label = state.text.t("More");
    let response = IconButton::new(&label, Vec2::splat(BUTTON), style)
        .show(ui, |ui, rect, colour| icons::more(ui.painter(), rect, colour))
        .on_hover_text(label.clone());
    if response.clicked() {
        state.toolbar_more = !state.toolbar_more;
    }
    let mut open = state.toolbar_more;
    egui::Popup::from_response(&response).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::CloseOnClick).show(|ui| {
        ui.set_min_width(150.0);
        for index in overflow {
            let Some(group) = Group::ALL.get(*index) else { continue };
            match group {
                Group::Mode => {
                    let mode = sidebar::mode_of(view);
                    for (to, name) in [(Mode::Filter, mode_labels[0]), (Mode::Crop, mode_labels[1])] {
                        if ui.selectable_label(mode == to, name.as_str()).clicked() {
                            switch_mode(state, view, engine, to);
                        }
                    }
                }
                group => {
                    for action in group.actions() {
                        let name = state.text.t(action.label());
                        if ui.add_enabled(action.enabled(view), egui::Button::new(name)).clicked() {
                            action.run(state, view, engine);
                        }
                    }
                }
            }
            ui.separator();
        }
    });
    state.toolbar_more = open;
}

/// Export PDF and the chevron that opens the export options (§4.6). Laid out from the far end,
/// so the chevron is drawn first.
fn export_button(ui: &mut Ui, state: &mut UiState, view: &ViewState, style: &craft_ui::buttons::IconButtonStyle) {
    let ready = !view.tray.is_empty() && view.importing.is_none() && view.exporting.is_none();
    let options = state.text.t("Export options");
    let chevron = ui
        .add_enabled_ui(!view.tray.is_empty(), |ui| {
            IconButton::new(&options, Vec2::new(24.0, BUTTON), style).show(ui, |ui, rect, colour| icons::chevron_down(ui.painter(), rect, colour))
        })
        .inner
        .on_hover_text(options.clone());
    if chevron.clicked() {
        state.export_popover = !state.export_popover;
    }
    // The popup clears the flag itself when a click outside closes it. Left set, the flag made the
    // menus take a window to be open for good.
    let mut open = state.export_popover;
    egui::Popup::from_response(&chevron).open_bool(&mut open).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        crate::export::export_options_popover(ui, state);
    });
    state.export_popover = open;
    ui.add_space(2.0);
    let label = state.text.t("Export PDF");
    if theme::primary_button(ui, &label, ready, 30.0).clicked() && ready {
        state.start_export();
    }
}
