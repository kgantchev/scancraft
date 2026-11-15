//! ScanCraft engine (specification §6.2–§6.6, L3).
//!
//! Owners (build plan §4): WP6 owns everything except `flatten` (A29), which is WP11's.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cache;
pub mod crop;
pub mod curve;
pub mod error;
pub mod flatten;
pub mod jobs;
pub mod session;
pub mod settings;
pub mod undo;

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, LoupeCrop, Screen, ViewState};
use scancraft_core::geom::Point;
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::PhotoId;
use scancraft_core::traits::{PageDetector, Recognizer, UprightVoter};
use std::path::PathBuf;
use std::sync::Arc;

/// The platform services the apps plug in (specification §6.1): Vision on macOS, ocrs in the
/// command-line tool, none in most tests.
#[derive(Clone, Default)]
pub struct Services {
    pub detector: Option<Arc<dyn PageDetector>>,
    pub recognizer: Option<Arc<dyn Recognizer>>,
    pub upright: Option<Arc<dyn UprightVoter>>,
}

/// One session (§6.6).
pub struct Engine {
    session_dir: PathBuf,
    services: Services,
}

impl Engine {
    pub fn new(session_dir: PathBuf, services: Services) -> Engine {
        Engine { session_dir, services }
    }

    pub fn session_dir(&self) -> &PathBuf {
        &self.session_dir
    }

    pub fn services(&self) -> &Services {
        &self.services
    }
}

impl EngineApi for Engine {
    fn view(&self) -> ViewState {
        ViewState {
            screen: Screen::Import,
            crop: None,
            enhance: None,
            tray: Vec::new(),
            book_spreads: false,
            can_undo: false,
            can_redo: false,
            busy: false,
            notices: Vec::new(),
            restore_offer: None,
        }
    }

    fn dispatch(&mut self, command: Command) -> Result<(), String> {
        Err(format!("the engine doesn't run commands yet (WP6): {command:?}"))
    }

    fn poll(&mut self) -> bool {
        false
    }

    fn image(&mut self, key: ImageKey) -> Option<Arc<Rgb8>> {
        let _ = key;
        None
    }

    fn loupe(&mut self, photo: PhotoId, centre: Point, side: f64) -> Option<LoupeCrop> {
        let _ = (photo, centre, side);
        None
    }
}
