//! ScanCraft's interface (specification §4, L4).
//!
//! The whole interface: Screens 1–3, the canvas, the loupe, the handles, the tray, the menus and
//! the settings. It draws from the engine's [`ViewState`] and changes nothing but by dispatching
//! a [`scancraft_core::command::Command`], so the wiring to the real engine stays one seam.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod a11y;
pub mod canvas;
pub mod crop;
pub mod enhance;
pub mod export;
pub mod fake_engine;
pub mod import;
pub mod loupe;
pub mod menu;
pub mod state;
pub mod stepper;
pub mod text;
pub mod theme;
pub mod tools;
pub mod tray;

use scancraft_core::engine_api::{EngineApi, Notice, ViewState};

pub use fake_engine::FakeEngine;
pub use menu::{build_menu_bar, menu_state};
pub use state::{Settings, UiAction, UiState};

/// The whole interface. The app calls [`ScanCraftUi::ui`] once per frame.
pub struct ScanCraftUi {
    state: UiState,
}

impl ScanCraftUi {
    /// The interface of a fresh session.
    pub fn new() -> ScanCraftUi {
        ScanCraftUi { state: UiState::new() }
    }

    /// Draws one frame into the window's root `ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &mut dyn EngineApi) {
        self.apply_theme(ui.ctx());
        let view = engine.view();
        stepper::track_work(&mut self.state, &view);
        let stage = stepper::current_stage(&self.state, &view);
        let show_tray = !view.tray.is_empty() && !self.state.tray_hidden;
        // The stage bar, along the top: where the user is in the scan, and a way to any stage.
        egui::Panel::top("scancraft.stages").exact_size(stepper::STAGE_BAR_HEIGHT).resizable(false).show_separator_line(true).frame(egui::Frame::NONE).show(ui, |ui| {
            stepper::stage_bar(ui, &mut self.state, &view, engine);
        });
        // The tray's bar, along the bottom of the window (§4.5).
        if show_tray {
            egui::Panel::bottom("scancraft.tray").exact_size(tray::BAR_HEIGHT).resizable(false).frame(egui::Frame::NONE).show(ui, |ui| {
                tray::page_tray(ui, &mut self.state, &view, engine);
            });
        }
        // The step bar, above the tray: the buttons that move on or back.
        egui::Panel::bottom("scancraft.steps").exact_size(stepper::STEP_BAR_HEIGHT).resizable(false).frame(egui::Frame::NONE).show(ui, |ui| {
            stepper::step_bar(ui, &mut self.state, &view, engine);
        });
        // The tools, on the left: everything that changes the picture.
        if matches!(stage, stepper::Stage::Crop | stepper::Stage::Enhance) {
            let frame = egui::Frame::NONE.inner_margin(egui::Margin::same(12));
            egui::Panel::left("scancraft.tools").exact_size(tools::PANEL_WIDTH).resizable(false).show_separator_line(true).frame(frame).show(ui, |ui| {
                if stage == stepper::Stage::Crop {
                    tools::crop_tools(ui, &mut self.state, &view, engine);
                } else {
                    tools::enhance_tools(ui, &mut self.state, &view, engine);
                }
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.notice_strip(ui, &view);
            match stage {
                stepper::Stage::Import => import::import_screen(ui, &mut self.state, &view, engine),
                stepper::Stage::Crop => crop::crop_screen(ui, &mut self.state, &view, engine),
                stepper::Stage::Enhance => enhance::enhance_screen(ui, &mut self.state, &view, engine),
                stepper::Stage::Export => self.export_screen(ui),
            }
        });
        // The windows that float above the screens.
        if self.state.settings_open {
            export::settings_window(ui.ctx(), &mut self.state);
        }
        if self.state.about_open {
            self.about_window(ui.ctx());
        }
        if self.state.label_open {
            export::label_window(ui.ctx(), &mut self.state, &view, engine);
        }
    }

    /// Runs a menu item: the interface's own items, and the commands of the rest.
    pub fn run_menu_command(&mut self, command: &craft_menu::CommandRef, engine: &mut dyn EngineApi) {
        let view = engine.view();
        menu::handle_menu_command(command, &mut self.state, &view, engine);
    }

    /// What the app shell has to do this frame: open a panel, show a window.
    pub fn take_actions(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.state.actions)
    }

    /// The Settings window's values (§4.6).
    pub fn settings(&self) -> &Settings {
        &self.state.settings
    }

    /// Sets the Settings window's values (§4.6).
    pub fn set_settings(&mut self, settings: Settings) {
        self.state.settings = settings;
    }

    /// Whether a window of the interface is open, so the menu's items are greyed out.
    pub fn is_modal(&self) -> bool {
        self.state.settings_open || self.state.export_popover || self.state.about_open || self.state.label_open
    }

    /// Sets the folder of the reference set the labels are written to, after the folder panel
    /// answered.
    pub fn set_label_folder(&mut self, folder: String) {
        self.state.label_folder = Some(folder);
    }

    /// The keyboard's selection on Screen 2 (§4.3).
    pub fn selection(&self) -> Option<crate::state::Selection> {
        self.state.selection
    }

    /// Whether the loupe is showing now (§4.3).
    pub fn loupe_visible(&self, now: f64) -> bool {
        self.state.loupe_visible(now)
    }

    /// When the loupe stops showing, in the app's clock (§4.3).
    pub fn loupe_until(&self) -> Option<f64> {
        self.state.loupe_until
    }

    /// When the loupe was last shown, in the app's clock (§4.3).
    pub fn loupe_since(&self) -> Option<f64> {
        self.state.loupe_since
    }

    /// The messages the interface itself shows (§4.8).
    pub fn notices(&self) -> &[Notice] {
        &self.state.notices
    }

    /// Pushes ScanCraft's palette into the style, when the appearance changes.
    fn apply_theme(&mut self, ctx: &egui::Context) {
        let theme = ctx.theme();
        if self.state.theme == Some(theme) {
            return;
        }
        self.state.theme = Some(theme);
        theme::apply(ctx);
    }

    /// The messages of the engine and of the interface, in a strip at the top of the screen.
    ///
    /// The engine holds a message for one poll, which is a frame or two: far too short to read,
    /// and the strip made the whole screen jump down and back up as it came and went. So the
    /// interface takes each message of the engine into its own list, where it stays until the
    /// user dismisses it.
    fn notice_strip(&mut self, ui: &mut egui::Ui, view: &ViewState) {
        let shown: Vec<String> = view.notices.iter().map(|n| n.text.clone()).collect();
        for notice in &view.notices {
            if !self.state.engine_notices_seen.contains(&notice.text) {
                self.state.notice(notice.text.clone(), notice.error);
            }
        }
        self.state.engine_notices_seen = shown;
        let rows: Vec<(String, bool)> = self.state.notices.iter().map(|n| (n.text.clone(), n.error)).collect();
        if rows.is_empty() {
            return;
        }
        let dismiss = self.state.text.t("Dismiss");
        ui.vertical(|ui| {
            for (text, error) in &rows {
                notice_row(ui, text, *error);
            }
            if ui.button(dismiss).clicked() {
                self.state.notices.clear();
            }
        });
        ui.add_space(8.0);
    }

    /// The Export screen: the export runs in the app shell, so the interface says so (§4.4).
    fn export_screen(&mut self, ui: &mut egui::Ui) {
        let text = self.state.text.t("Exporting the PDF…");
        ui.centered_and_justified(|ui| {
            ui.label(text);
        });
    }

    /// The About panel.
    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.state.about_open;
        egui::Window::new("About ScanCraft").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("ScanCraft turns photos of paper documents into clean, searchable PDFs.");
            ui.label("Version 0.1.0");
        });
        self.state.about_open = open;
    }
}

/// One message in the strip at the top of the screen.
fn notice_row(ui: &mut egui::Ui, text: &str, error: bool) {
    let colour = if error { theme::INVALID } else { ui.visuals().text_color() };
    ui.horizontal(|ui| {
        ui.colored_label(colour, text);
    });
}

impl Default for ScanCraftUi {
    fn default() -> ScanCraftUi {
        ScanCraftUi::new()
    }
}

impl std::fmt::Debug for ScanCraftUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanCraftUi").field("state", &self.state).finish()
    }
}
