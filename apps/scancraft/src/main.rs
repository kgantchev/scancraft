//! The ScanCraft Mac app. WP13 wires the real engine and Vision services; WP14 packages it.
//!
//! The shell owns the window: the native menu bar, the native panels and the quit handling. The
//! interface itself lives in `scancraft-ui-egui` and changes the engine only by dispatching
//! commands.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_dialogs::{Answer, Dialogs, FileDialog};
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ViewState};
use scancraft_core::recipe::{ExportOptions, FlattenMode, PageOrder, Paper};
use scancraft_engine::{Engine, Services};
use scancraft_ui_egui::{ScanCraftUi, Settings, UiAction};
use std::process::ExitCode;
use std::sync::Arc;

/// What the app does when a panel answers.
#[derive(Debug, Clone)]
enum PanelAction {
    /// The Open panel answered with files to import.
    Import,
    /// The Save panel answered with the PDF's path.
    Export { options: ExportOptions },
    /// The folder panel answered with the folder of the reference set the labels go to.
    LabelFolder,
}

struct App {
    ui: ScanCraftUi,
    engine: Engine,
    menu: Option<craft_menu::NativeMenu>,
    panels: Dialogs<PanelAction>,
    /// The "Cancel Export or Keep Exporting" question is showing (§4.8).
    quit_prompt: bool,
    /// The user chose to quit although an export is running.
    quit_confirmed: bool,
}

impl App {
    /// The state of a menu item for this frame.
    fn item_state(command: &craft_menu::CommandRef, view: &ViewState, ui: &ScanCraftUi) -> craft_menu::ItemState {
        // An item is live only when it can act on what the window shows.
        let mut state = craft_menu::ItemState { enabled: scancraft_ui_egui::menu::item_enabled(command.id.as_str(), view), checked: None };
        // The two switches of the View menu are on while their panel shows.
        state.checked = ui.menu_item_checked(command.id.as_str());
        match command.id.as_str() {
            "crop.set_flatten" => {
                let mode = command.params.get("mode").and_then(|m| m.as_str()).unwrap_or("Auto");
                let current = view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.flatten));
                state.checked = Some(matches!(
                    (mode, current),
                    ("Auto", Some(FlattenMode::Auto)) | ("On", Some(FlattenMode::On)) | ("Off", Some(FlattenMode::Off))
                ));
            }
            "page.set_look" => {
                let look = command.params.get("look").and_then(|l| l.as_str()).unwrap_or("Original");
                let current = view.enhance.as_ref().map(|enhance| enhance.look);
                state.checked = Some(matches!(
                    (look, current),
                    ("Original", Some(scancraft_core::recipe::Look::Original))
                        | ("BlackWhite", Some(scancraft_core::recipe::Look::BlackWhite))
                        | ("Grayscale", Some(scancraft_core::recipe::Look::Grayscale))
                        | ("ColorDocument", Some(scancraft_core::recipe::Look::ColorDocument))
                ));
            }
            _ => {}
        }
        state
    }

    /// Runs a menu item: the app's own Quit, and everything else through the interface.
    fn run_command(&mut self, command: &craft_menu::CommandRef, ctx: &egui::Context) {
        if command.id == craft_quit::QUIT_MENU_ID {
            if self.engine.is_exporting() && !self.quit_confirmed {
                self.quit_prompt = true;
                return;
            }
            craft_quit::on_menu_event(ctx, &command.id);
            return;
        }
        self.ui.run_menu_command(command, &mut self.engine);
    }

    /// Opens the panels the interface asked for, and runs the answers.
    fn panels(&mut self, ui: &egui::Ui, frame: &eframe::Frame) {
        for action in self.ui.take_actions() {
            match action {
                UiAction::ImportFiles => {
                    let dialog = FileDialog::open_many()
                        .title("Import photos and PDFs")
                        .filter("Photos and PDFs", &["jpg", "jpeg", "png", "heic", "heif", "tif", "tiff", "webp", "pdf"]);
                    self.panels.ask(dialog, PanelAction::Import);
                }
                UiAction::ExportPdf { options } => {
                    let name = scancraft_ui_egui::export::default_export_file_name(chrono::Local::now());
                    let dialog = FileDialog::save().title("Export to PDF").file_name(name).filter("PDF", &["pdf"]);
                    self.panels.ask(dialog, PanelAction::Export { options });
                }
                UiAction::ChooseLabelFolder => {
                    let dialog = FileDialog::folder().title("Choose the reference set's folder");
                    self.panels.ask(dialog, PanelAction::LabelFolder);
                }
                UiAction::OpenSettings | UiAction::ShowAbout => {
                    // The interface opens its own windows.
                }
                UiAction::RevealInFinder { path } => {
                    if let Err(e) = reveal_in_finder(&path) {
                        self.ui.notice(e, true);
                    }
                }
                UiAction::OpenFile { path } => {
                    if let Err(e) = open_file(&path) {
                        self.ui.notice(e, true);
                    }
                }
                UiAction::SettingsChanged(settings) => {
                    let engine = &mut self.engine;
                    engine.set_settings(|current| {
                        current.default_paper = settings.default_paper;
                        current.snap_to_paper = settings.snap_to_paper_sizes;
                        current.rotate_upright = settings.rotate_upright_automatically;
                        current.flatten_automatically = settings.flatten_curved_pages_automatically;
                        current.book_page_order = settings.book_page_order;
                        current.language_correction = settings.language_correction;
                        current.keep_paper_colour = settings.keep_paper_colour;
                        current.sessions_dir = Some(settings.session_storage.clone());
                    });
                }
            }
        }
        for (action, answer) in self.panels.take_answers() {
            match (action, answer) {
                (PanelAction::Import, Answer::Paths(paths)) => {
                    let paths: Vec<String> = paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
                    if let Err(e) = self.engine.dispatch(Command::ImportAdd { paths }) {
                        self.ui.notice(e, true);
                    }
                }
                (PanelAction::Export { options }, Answer::Paths(paths)) => {
                    if let Some(path) = paths.first()
                        && let Err(e) = self.engine.dispatch(Command::ExportPdf { path: path.to_string_lossy().into_owned(), options })
                    {
                        self.ui.notice(e, true);
                    }
                }
                (PanelAction::LabelFolder, Answer::Paths(paths)) => {
                    if let Some(folder) = paths.first() {
                        self.ui.set_label_folder(folder.to_string_lossy().into_owned());
                    }
                }
                _ => {}
            }
        }
        self.panels.show_pending(Some(frame));
        let _ = ui;
    }
}

impl App {
    /// "A PDF is being written. Cancel the export, or keep exporting?" (§4.8).
    fn quit_question(&mut self, ctx: &egui::Context) {
        if !self.quit_prompt {
            return;
        }
        if !self.engine.is_exporting() {
            self.quit_prompt = false;
            return;
        }
        egui::Window::new("Quit during an export").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
            ctx,
            |ui| {
                ui.label("A PDF is still being written.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Keep Exporting").clicked() {
                        self.quit_prompt = false;
                    }
                    if ui.button("Cancel Export").clicked() {
                        self.quit_prompt = false;
                        self.quit_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            },
        );
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(menu) = self.menu.as_mut() {
            menu.raw_input(raw_input);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _ = frame;
        if scancraft_core::engine_api::EngineApi::poll(&mut self.engine) {
            ctx.request_repaint();
        }
        // The menu bar, rebuilt from the view and laid out for the platform.
        let view = self.engine.view();
        let mut bar = scancraft_ui_egui::build_menu_bar(&view);
        craft_menu::apply_state(&mut bar, |command| App::item_state(command, &view, &self.ui));
        let state = self.ui.menu_state(&view);
        let options = craft_menu::LayoutOptions::new("ScanCraft");
        let gate = craft_menu::Gate { modal: self.ui.is_modal(), recording_shortcut: false, text_editing: false };
        let clicks = match self.menu.as_mut() {
            Some(menu) => {
                menu.set_gate(gate);
                menu.take_clicks(ctx)
            }
            None => Vec::new(),
        };
        for command in clicks {
            self.run_command(&command, ctx);
        }
        if let Some(menu) = self.menu.as_mut() {
            menu.sync_with(state, || craft_menu::layout(&bar, craft_menu::Target::current(), &options));
        }
        // Quitting during an export asks one question; the session is already saved (§4.8).
        if ctx.input(|i| i.viewport().close_requested()) && self.engine.is_exporting() && !self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_prompt = true;
        }
        craft_quit::on_close_request(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui.set_settings(ui_settings(&self.engine));
        self.ui.ui(ui, &mut self.engine);
        self.panels(ui, frame);
        self.quit_question(ui.ctx());
    }
}

/// Shows a file in the Finder. The path is passed as an argument, never through a shell, and
/// only when it is absolute, so it can never be read as one of `open`'s own options.
fn reveal_in_finder(path: &str) -> Result<(), String> {
    require_absolute(path)?;
    run_open(&["-R", path])
}

/// Opens a file with the program the system has for it.
fn open_file(path: &str) -> Result<(), String> {
    require_absolute(path)?;
    run_open(&[path])
}

fn require_absolute(path: &str) -> Result<(), String> {
    if std::path::Path::new(path).is_absolute() { Ok(()) } else { Err("That file's place is not known.".to_owned()) }
}

/// Runs the system's `open` with `args`; only the Mac has one.
fn run_open(args: &[&str]) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("This can only be done on a Mac.".to_owned());
    }
    match std::process::Command::new("open").args(args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("The file could not be opened ({status}).")),
        Err(e) => Err(format!("The file could not be opened: {e}.")),
    }
}

/// The engine's settings as the interface's own values.
fn ui_settings(engine: &Engine) -> Settings {
    let settings = engine.settings();
    Settings {
        default_paper: settings.default_paper,
        snap_to_paper_sizes: settings.snap_to_paper,
        rotate_upright_automatically: settings.rotate_upright,
        flatten_curved_pages_automatically: settings.flatten_automatically,
        book_page_order: settings.book_page_order,
        language_correction: settings.language_correction,
        keep_paper_colour: settings.keep_paper_colour,
        session_storage: settings.sessions_dir.clone().unwrap_or_else(scancraft_ui_egui::state::default_session_storage),
    }
}

/// The paper of the system's region, for the settings window's default.
#[allow(dead_code)]
fn default_paper() -> Paper {
    scancraft_ui_egui::state::default_paper()
}

/// The page order of the system's language, for the settings window's default.
#[allow(dead_code)]
fn default_page_order() -> PageOrder {
    scancraft_ui_egui::state::default_page_order()
}

/// Starts the engine in a new session folder under `root`, so the last session is still there to
/// offer ("Continue your last scan?", §4.2). Sessions with nothing in them are swept first: they
/// are never worth offering, and the cleanup keeps every session that was not exported.
fn start_engine(root: &std::path::Path, services: Services) -> Engine {
    for info in scancraft_engine::session::list_sessions(root) {
        if !info.exported && info.pages == 0 && info.dir.parent() == Some(root) {
            let _ = std::fs::remove_dir_all(&info.dir);
        }
    }
    let stamp = chrono::Local::now().format("Scan %Y-%m-%d at %H.%M.%S").to_string();
    let mut dir = root.join(&stamp);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{stamp} {n}"));
        n += 1;
    }
    Engine::new(dir, services)
}

fn services() -> Services {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Arc;
        Services {
            detector: Some(Arc::new(scancraft_apple::VisionPageDetector)),
            recognizer: Some(Arc::new(scancraft_apple::VisionRecognizer)),
            upright: Some(Arc::new(scancraft_apple::VisionUprightVoter)),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Services::default()
    }
}

/// The app icon at 256 px, with a transparent margin outside its rounded corners. It sets the Dock
/// and window icon when the app runs unbundled; the packaged app takes `assets/ScanCraft.icns`.
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/scancraft-256.png");

fn main() -> ExitCode {
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]).with_min_inner_size([860.0, 600.0]);
    match eframe::icon_data::from_png_bytes(APP_ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        // A missing icon is not worth refusing to start for.
        Err(e) => eprintln!("ScanCraft couldn't read its icon: {e}"),
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let root = scancraft_engine::settings::path_from_setting(&scancraft_ui_egui::state::default_session_storage());
    let engine = start_engine(&root, services());
    let result = eframe::run_native(
        "ScanCraft",
        options,
        Box::new(move |cc| {
            // The menu bar is installed once, on the main thread, in the creator closure.
            let bar = scancraft_ui_egui::build_menu_bar(&scancraft_ui_egui::fake_engine::empty_view());
            let menu = craft_menu::mac::install(&cc.egui_ctx, &bar);
            let wake: craft_dialogs::Wake = {
                let ctx = cc.egui_ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            Ok(Box::new(App { ui: ScanCraftUi::new(), engine, menu, panels: Dialogs::new(wake), quit_prompt: false, quit_confirmed: false }))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ScanCraft couldn't start: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_launch_starts_a_new_session_folder_under_the_root_and_sweeps_empty_ones() {
        let root = std::env::temp_dir().join(format!("scancraft-app-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let first = start_engine(&root, Services::default());
        let first_dir = first.session_dir().clone();
        assert_eq!(first_dir.parent(), Some(root.as_path()), "the session is in the sessions folder, not the temporary one");
        drop(first);
        let earlier = root.join("Scan earlier");
        std::fs::rename(&first_dir, &earlier).unwrap();
        let second = start_engine(&root, Services::default());
        assert_eq!(second.session_dir().parent(), Some(root.as_path()));
        assert!(!earlier.exists(), "a session with nothing in it is swept");
        drop(second);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_app_icon_decodes_and_keeps_its_transparent_corners() {
        let icon = eframe::icon_data::from_png_bytes(APP_ICON_PNG).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
        let alpha = |x: u32, y: u32| icon.rgba[((y * icon.width + x) * 4 + 3) as usize];
        assert_eq!(alpha(1, 1), 0, "the corner outside the rounded square is clear");
        assert_eq!(alpha(128, 128), 255, "the middle is solid");
    }
}
