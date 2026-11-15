//! ScanCraft interface (specification §4, L4). Owner: WP7, built against a fake `EngineApi`
//! until WP13 connects the real engine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use scancraft_core::engine_api::{EngineApi, Screen};

/// The whole interface. The app calls [`ScanCraftUi::ui`] once per frame.
#[derive(Debug, Default)]
pub struct ScanCraftUi {}

impl ScanCraftUi {
    pub fn new() -> ScanCraftUi {
        ScanCraftUi {}
    }

    /// Draws one frame into the window's root `ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui, engine: &mut dyn EngineApi) {
        let view = engine.view();
        let screen = match view.screen {
            Screen::Import => "Import",
            Screen::Crop(_) => "Crop",
            Screen::Enhance(_) => "Enhance",
            Screen::Export => "Export",
        };
        ui.label(format!("ScanCraft: the {screen} screen is not built yet (WP7)."));
    }
}
