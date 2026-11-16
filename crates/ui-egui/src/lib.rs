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
        egui::Panel::top("scancraft.stages")
            .exact_size(stepper::STAGE_BAR_HEIGHT)
            .resizable(false)
            .show_separator_line(true)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
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
            egui::Panel::left("scancraft.tools").exact_size(tools::PANEL_WIDTH).resizable(false).show_separator_line(true).frame(frame).show(
                ui,
                |ui| {
                    if stage == stepper::Stage::Crop {
                        tools::crop_tools(ui, &mut self.state, &view, engine);
                    } else {
                        tools::enhance_tools(ui, &mut self.state, &view, engine);
                    }
                },
            );
        }
        self.take_engine_notices(&view);
        egui::CentralPanel::default().show(ui, |ui| match stage {
            stepper::Stage::Import => import::import_screen(ui, &mut self.state, &view, engine),
            stepper::Stage::Crop => crop::crop_screen(ui, &mut self.state, &view, engine),
            stepper::Stage::Enhance => enhance::enhance_screen(ui, &mut self.state, &view, engine),
            stepper::Stage::Export => self.export_screen(ui),
        });
        // Files can be dropped on the window at any time, whatever it is showing.
        self.handle_drop(ui, engine);
        self.drop_overlay(ui, stage == stepper::Stage::Import);
        // The windows that float above the screens.
        if self.state.settings_open {
            export::settings_window(ui.ctx(), &mut self.state);
        }
        if self.state.about_open {
            self.about_window(ui.ctx());
        }
        if let Some((waiting, pages)) = self.state.confirm_export {
            self.confirm_export_window(ui.ctx(), waiting, pages);
        }
        if self.state.label_open {
            export::label_window(ui.ctx(), &mut self.state, &view, engine);
        }
        // The messages float over everything, above the bars at the bottom.
        let bottom = if show_tray { tray::BAR_HEIGHT } else { 0.0 } + stepper::STEP_BAR_HEIGHT + 12.0;
        self.toasts(ui, engine, bottom);
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
        self.state.settings_open || self.state.export_popover || self.state.about_open || self.state.label_open || self.state.confirm_export.is_some()
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

    /// Whether a handle is pressed and so holds the loupe open (§4.3).
    pub fn loupe_held(&self) -> bool {
        self.state.loupe.held
    }

    /// When a keyboard nudge's loupe stops showing, in the app's clock (§4.3).
    pub fn loupe_until(&self) -> Option<f64> {
        self.state.loupe.until
    }

    /// When a keyboard nudge last showed the loupe, in the app's clock (§4.3).
    pub fn loupe_since(&self) -> Option<f64> {
        self.state.loupe.since
    }

    /// The photo position the loupe magnifies, while it has one (§4.3).
    pub fn loupe_focus(&self) -> Option<scancraft_core::geom::Point> {
        self.state.loupe.focus
    }

    /// The messages the interface shows now (§4.8).
    pub fn notices(&self) -> Vec<Notice> {
        self.state.notices.iter().map(|toast| Notice { text: toast.text.clone(), error: toast.error }).collect()
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

    /// Takes the engine's messages into the interface's own list.
    ///
    /// The engine holds a message for one poll, which is a frame or two: far too short to read. So
    /// the interface takes each message of the engine once, and shows it itself.
    fn take_engine_notices(&mut self, view: &ViewState) {
        let shown: Vec<String> = view.notices.iter().map(|n| n.text.clone()).collect();
        for notice in &view.notices {
            if !self.state.engine_notices_seen.contains(&notice.text) {
                self.state.notice(notice.text.clone(), notice.error);
            }
        }
        self.state.engine_notices_seen = shown;
    }

    /// Draws the messages as toasts over the window, `bottom` points above its bottom edge.
    ///
    /// A toast takes no room in the layout, so nothing on the screen moves when one comes or
    /// goes. A short one fades after five seconds; an error stays until it is closed. One that
    /// has an Undo runs the general undo, which is right because the last thing done is what the
    /// message is about.
    fn toasts(&mut self, ui: &mut egui::Ui, engine: &mut dyn EngineApi, bottom: f32) {
        let now = ui.input(|i| i.time);
        for toast in &mut self.state.notices {
            if toast.shown_at.is_none() {
                toast.shown_at = Some(now);
            }
        }
        self.state.notices.retain(|toast| !toast.expired(now));
        if self.state.notices.is_empty() {
            return;
        }
        // The next frame that has to change something: the fade's start, then every frame of it.
        let next = self
            .state
            .notices
            .iter()
            .filter(|toast| !toast.error)
            .filter_map(|toast| toast.shown_at.map(|at| at + state::TOAST_SECONDS - state::TOAST_FADE - now))
            .fold(f64::INFINITY, f64::min);
        if next <= 0.0 {
            ui.ctx().request_repaint();
        } else if next.is_finite() {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(next));
        }
        let mut undo = false;
        let mut closed: Option<usize> = None;
        let rows: Vec<(usize, state::Toast)> = self.state.notices.iter().cloned().enumerate().collect();
        let undo_label = self.state.text.t("Undo");
        let dismiss_label = self.state.text.t("Dismiss");
        egui::Area::new(ui.make_persistent_id("scancraft.toasts"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -bottom])
            .show(ui.ctx(), |ui| {
                ui.vertical_centered(|ui| {
                    for (index, toast) in &rows {
                        ui.scope(|ui| {
                            ui.set_opacity(toast.opacity(now));
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    theme::toast_icon(ui, toast.error);
                                    ui.label(&toast.text);
                                    if toast.undo && ui.button(&undo_label).clicked() {
                                        undo = true;
                                        closed = Some(*index);
                                    }
                                    if theme::close_button(ui, &dismiss_label).clicked() {
                                        closed = Some(*index);
                                    }
                                });
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
            });
        if undo {
            canvas::dispatch(&mut self.state, engine, scancraft_core::command::Command::EditUndo);
        }
        if let Some(index) = closed
            && index < self.state.notices.len()
        {
            self.state.notices.remove(index);
        }
    }

    /// Files dropped on the window, whatever it is showing, are added to the scan (§4.2).
    fn handle_drop(&mut self, ui: &egui::Ui, engine: &mut dyn EngineApi) {
        let dropped: Vec<String> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_string_lossy().into_owned()).collect());
        if dropped.is_empty() {
            return;
        }
        canvas::dispatch(&mut self.state, engine, scancraft_core::command::Command::ImportAdd { paths: dropped });
    }

    /// Tints the whole window, and says what a drop would do, while files are held over it. The
    /// empty window draws its own, bigger one.
    fn drop_overlay(&mut self, ui: &egui::Ui, has_own: bool) {
        let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering || has_own {
            return;
        }
        let rect = ui.ctx().viewport_rect();
        let accent = theme::accent(ui.visuals().dark_mode);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.make_persistent_id("scancraft.drop-overlay")));
        painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.1));
        painter.rect_stroke(rect.shrink(6.0), 10.0, egui::Stroke::new(3.0, accent), egui::StrokeKind::Inside);
        let label = self.state.text.t("Drop to add pages");
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(22.0), ui.visuals().text_color());
    }

    /// The Export screen: the export runs in the app shell, so the interface says so (§4.4).
    fn export_screen(&mut self, ui: &mut egui::Ui) {
        let text = self.state.text.t("Exporting the PDF…");
        ui.centered_and_justified(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(text);
            });
        });
    }

    /// "3 photos haven't been added. Export the 5 added pages?" (§4.8).
    fn confirm_export_window(&mut self, ctx: &egui::Context, waiting: usize, pages: usize) {
        let photos = self.state.text.trn(waiting as u64, "{n} photo hasn't been added.", "{n} photos haven't been added.");
        let question = self.state.text.trn(pages as u64, "Export the {n} added page?", "Export the {n} added pages?");
        egui::Window::new("Some photos haven't been added").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ctx,
            |ui| {
                ui.label(format!("{photos} {question}"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(self.state.text.t("Export")).clicked() {
                        self.state.confirm_export = None;
                        self.state.start_export();
                    }
                    if ui.button(self.state.text.t("Cancel")).clicked() {
                        self.state.confirm_export = None;
                    }
                });
            },
        );
    }

    /// Shows a message from the app shell, such as a refused import (§4.8).
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.state.notice(text, error);
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
