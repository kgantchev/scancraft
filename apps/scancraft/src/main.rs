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
}

impl App {
    /// The state of a menu item for this frame.
    fn item_state(command: &craft_menu::CommandRef, view: &ViewState) -> craft_menu::ItemState {
        let mut state = craft_menu::ItemState { enabled: true, checked: None };
        match command.id.as_str() {
            "edit.undo" => state.enabled = view.can_undo,
            "edit.redo" => state.enabled = view.can_redo,
            "crop.auto" => state.checked = view.crop.as_ref().map(|crop| crop.auto_detect),
            "crop.book" => state.checked = Some(view.book_spreads),
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
                    let _ = self.engine.dispatch(Command::ImportAdd { paths });
                }
                (PanelAction::Export { options }, Answer::Paths(paths)) => {
                    if let Some(path) = paths.first() {
                        let _ = self.engine.dispatch(Command::ExportPdf { path: path.to_string_lossy().into_owned(), options });
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
        craft_menu::apply_state(&mut bar, |command| App::item_state(command, &view));
        let state = scancraft_ui_egui::menu_state(&view);
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
        craft_quit::on_close_request(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ui.set_settings(ui_settings(&self.engine));
        self.ui.ui(ui, &mut self.engine);
        self.panels(ui, frame);
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

fn main() -> ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]).with_min_inner_size([860.0, 600.0]),
        ..Default::default()
    };
    let engine = Engine::new(std::env::temp_dir().join("scancraft-session"), services());
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
            Ok(Box::new(App { ui: ScanCraftUi::new(), engine, menu, panels: Dialogs::new(wake) }))
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
