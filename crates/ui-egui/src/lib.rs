//! ScanCraft's interface (specification §4, L4).
//!
//! The whole interface: the empty window and the workspace (the toolbar, the pages sidebar, the
//! canvas, the inspector and the status bar), the loupe, the handles, the menus and the settings.
//! It draws from the engine's [`ViewState`] and changes nothing but by dispatching a
//! [`scancraft_core::command::Command`], so the wiring to the real engine stays one seam.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod a11y;
pub mod canvas;
pub mod crop;
pub mod enhance;
pub mod export;
pub mod fake_engine;
pub mod icons;
pub mod import;
pub mod inspector;
pub mod loupe;
pub mod menu;
pub mod sidebar;
pub mod state;
pub mod statusbar;
pub mod text;
pub mod theme;
pub mod thumb;
pub mod toolbar;

use scancraft_core::engine_api::{EngineApi, Notice, ViewState};

pub use fake_engine::FakeEngine;
pub use menu::{build_menu_bar, menu_state};
pub use state::{Accessibility, Settings, UiAction, UiState};

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
        theme::set_accessibility(ui.ctx(), self.state.accessibility);
        let view = engine.view();
        // The export options hang from the Export button, which is there only while there are pages.
        if view.tray.is_empty() {
            self.state.export_popover = false;
        }
        sidebar::prune_selection(&mut self.state, &view);
        self.take_engine_notices(&view);
        self.track_export(&view);
        // Until there are pages, and while nothing is being read, the window is the empty one.
        let workspace = !view.tray.is_empty() || view.importing.is_some();
        if workspace {
            self.workspace(ui, &view, engine);
        } else {
            self.state.keys_taken = false;
            self.state.show_original = false;
            egui::CentralPanel::default().show(ui, |ui| import::import_screen(ui, &mut self.state, &view, engine));
        }
        // Files can be dropped on the window at any time, whatever it is showing.
        self.handle_drop(ui, engine);
        self.drop_overlay(ui, !workspace);
        if !self.is_modal() {
            sidebar::delete_key(ui, &mut self.state, &view, engine);
            self.selection_keys(ui, &view);
        }
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
        export::export_sheet(ui.ctx(), &mut self.state, &view, engine);
        // The messages float over everything, above the status bar.
        let bottom = if workspace { statusbar::HEIGHT } else { 0.0 } + 12.0;
        self.toasts(ui, engine, bottom);
    }

    /// The workspace: the toolbar along the top, the pages on the left, the inspector on the
    /// right, the status bar along the bottom, and the page in the middle.
    ///
    /// The regions are laid out by rectangle, not as panels, because of the order the widgets are
    /// made in: it is the order the Tab key walks them, and for a keyboard or a screen reader the
    /// order is toolbar, pages, canvas, inspector, status bar. Panels would make the canvas last.
    fn workspace(&mut self, ui: &mut egui::Ui, view: &ViewState, engine: &mut dyn EngineApi) {
        let full = ui.max_rect();
        let rtl = craft_ui::direction::is_rtl(&*ui);
        let toolbar_rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), toolbar::HEIGHT));
        let status_rect = egui::Rect::from_min_max(egui::pos2(full.min.x, full.max.y - statusbar::HEIGHT), full.max);
        let body = egui::Rect::from_min_max(egui::pos2(full.min.x, toolbar_rect.max.y), egui::pos2(full.max.x, status_rect.min.y));
        let sidebar_width = if self.state.sidebar_hidden { 0.0 } else { sidebar::WIDTH };
        let inspector_width = if self.state.inspector_hidden || (view.crop.is_none() && view.enhance.is_none()) { 0.0 } else { inspector::WIDTH };
        // The pages are at the reading start of the window and the inspector at its end.
        let (left_width, right_width) = if rtl { (inspector_width, sidebar_width) } else { (sidebar_width, inspector_width) };
        let left = egui::Rect::from_min_max(body.min, egui::pos2(body.min.x + left_width, body.max.y));
        let right = egui::Rect::from_min_max(egui::pos2(body.max.x - right_width, body.min.y), body.max);
        let middle = egui::Rect::from_min_max(egui::pos2(left.max.x, body.min.y), egui::pos2(right.min.x, body.max.y));
        let (sidebar_rect, inspector_rect) = if rtl { (right, left) } else { (left, right) };
        // The lines between the regions.
        let line = ui.visuals().widgets.noninteractive.bg_stroke;
        let painter = ui.painter();
        painter.hline(full.x_range(), toolbar_rect.max.y, line);
        painter.hline(full.x_range(), status_rect.min.y, line);
        if left_width > 0.0 {
            painter.vline(left.max.x, body.y_range(), line);
        }
        if right_width > 0.0 {
            painter.vline(right.min.x, body.y_range(), line);
        }
        // The widgets, in the order of the keyboard's walk.
        region(ui, toolbar_rect, |ui| toolbar::toolbar(ui, &mut self.state, view, engine));
        if sidebar_width > 0.0 {
            region(ui, sidebar_rect, |ui| sidebar::sidebar(ui, &mut self.state, view, engine));
        }
        // The canvas takes everything the regions leave, edge to edge, dark in both appearances.
        region(ui, middle, |ui| {
            if view.crop.is_some() {
                crop::crop_screen(ui, &mut self.state, view, engine);
            } else if view.enhance.is_some() {
                enhance::enhance_screen(ui, &mut self.state, view, engine);
            } else {
                let rect = ui.available_rect_before_wrap();
                ui.allocate_rect(rect, egui::Sense::hover());
                theme::canvas_background(ui, rect);
                canvas::waiting(ui, rect, &self.state.text.t("Reading your photos…"));
            }
        });
        if inspector_width > 0.0 {
            let clip = inspector_rect;
            region(ui, inspector_rect.shrink(12.0), |ui| {
                ui.set_clip_rect(clip.intersect(ui.clip_rect()));
                inspector::inspector(ui, &mut self.state, view, engine)
            });
        }
        region(ui, status_rect, |ui| statusbar::status_bar(ui, &mut self.state, view, engine));
        // The two flags the filter inspector sets for the canvas (read a frame later, since the
        // canvas is made before it) mean nothing without it.
        if inspector_width == 0.0 || view.enhance.is_none() {
            self.state.keys_taken = false;
            self.state.show_original = false;
        }
    }

    /// ⌘A selects every page, and Escape lets go of a selection (in Crop, Escape is Done, and the
    /// selection stays for when the user comes back).
    fn selection_keys(&mut self, ui: &egui::Ui, view: &ViewState) {
        let (select_all, escape) = ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Escape)));
        if select_all && view.tray.len() >= 2 {
            sidebar::select_all(&mut self.state, view);
        } else if escape && view.crop.is_none() {
            sidebar::clear_selection(&mut self.state);
        }
    }

    /// Follows the export through the sheet over the window: while it runs, and once it has been
    /// written. It is told from the engine's view, which holds the progress and the summary.
    fn track_export(&mut self, view: &ViewState) {
        if view.exporting.is_some() {
            self.state.export_sheet = state::ExportSheet::Running;
        } else if let Some(summary) = view.last_export.as_ref().filter(|s| self.state.summary_seen.as_ref() != Some(*s)) {
            self.state.summary_seen = Some(summary.clone());
            self.state.export_sheet = state::ExportSheet::Finished(summary.clone());
        } else if self.state.export_sheet == state::ExportSheet::Running {
            // It stopped without writing a file: the engine's message says why.
            self.state.export_sheet = state::ExportSheet::Closed;
        }
        if view.last_export.is_none() {
            self.state.summary_seen = None;
        }
    }

    /// Runs a menu item: the interface's own items, and the commands of the rest.
    pub fn run_menu_command(&mut self, command: &craft_menu::CommandRef, engine: &mut dyn EngineApi) {
        let view = engine.view();
        menu::handle_menu_command(command, &mut self.state, &view, engine);
    }

    /// The hash of everything the menus show: the view's, and which of the panels are showing.
    pub fn menu_state(&self, view: &ViewState) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        menu::menu_state(view).hash(&mut hasher);
        (self.state.sidebar_hidden, self.state.inspector_hidden).hash(&mut hasher);
        hasher.finish()
    }

    /// Whether a menu item that is a switch is on: the pages and the inspector are showing.
    pub fn menu_item_checked(&self, id: &str) -> Option<bool> {
        match id {
            menu::TOGGLE_PAGES => Some(!self.state.sidebar_hidden),
            menu::TOGGLE_INSPECTOR => Some(!self.state.inspector_hidden),
            _ => None,
        }
    }

    /// What the app shell has to do this frame: open a panel, show a window.
    pub fn take_actions(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.state.actions)
    }

    /// Tells the interface what the system's Reduce Motion and Increase Contrast are (§4.9).
    pub fn set_accessibility(&mut self, accessibility: Accessibility) {
        self.state.accessibility = accessibility;
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
        self.state.settings_open
            || self.state.export_popover
            || self.state.about_open
            || self.state.label_open
            || self.state.export_sheet != state::ExportSheet::Closed
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
        // With Reduce Motion a toast does not fade, so the only frame it needs is the one on which it
        // goes; otherwise the fade's start, and then every frame of it.
        let still = self.state.accessibility.reduce_motion;
        let lead = if still { 0.0 } else { state::TOAST_FADE };
        let next = self
            .state
            .notices
            .iter()
            .filter(|toast| !toast.error)
            .filter_map(|toast| toast.shown_at.map(|at| at + state::TOAST_SECONDS - lead - now))
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
                            ui.set_opacity(toast.opacity(now, still));
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    theme::toast_icon(ui, toast.error);
                                    let message = ui.label(&toast.text);
                                    // A screen reader says the message when it appears, without the
                                    // user going to find it: politely for news, at once for an error.
                                    let live = if toast.error { egui::accesskit::Live::Assertive } else { egui::accesskit::Live::Polite };
                                    ui.ctx().accesskit_node_builder(message.id, |node| node.set_live(live));
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

    /// Shows a message from the app shell, such as a refused import (§4.8).
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.state.notice(text, error);
    }

    /// The About panel.
    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.state.about_open;
        let title = self.state.text.t("About ScanCraft");
        let summary = self.state.text.t("ScanCraft turns photos of paper documents into clean, searchable PDFs.");
        let version = self.state.text.t("Version {v}").replace("{v}", env!("CARGO_PKG_VERSION"));
        egui::Window::new(title).open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label(summary);
            ui.label(version);
        });
        self.state.about_open = open;
    }
}

/// Lays out `add` in `rect`, clipped to it, top down from its corner.
fn region(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        add(ui);
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
