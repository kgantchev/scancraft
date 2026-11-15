//! The ScanCraft Mac app. WP13 wires the real engine and Vision services; WP14 packages it.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use scancraft_engine::{Engine, Services};
use scancraft_ui_egui::ScanCraftUi;
use std::process::ExitCode;

struct App {
    ui: ScanCraftUi,
    engine: Engine,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _ = frame;
        if scancraft_core::engine_api::EngineApi::poll(&mut self.engine) {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _ = frame;
        self.ui.ui(ui, &mut self.engine);
    }
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
    let result = eframe::run_native("ScanCraft", options, Box::new(move |_cc| Ok(Box::new(App { ui: ScanCraftUi::new(), engine }))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ScanCraft couldn't start: {e}");
            ExitCode::FAILURE
        }
    }
}
