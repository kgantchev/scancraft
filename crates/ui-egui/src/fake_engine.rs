//! A fake engine: the whole world the interface's tests draw against.
//!
//! WP13 swaps it for the real engine, so the wiring stays a single seam: the interface only ever
//! sees an [`EngineApi`].

use std::sync::Arc;

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, LoupeCrop, Screen, ViewState};
use scancraft_core::geom::Point;
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::PhotoId;

/// The view of a session with nothing in it: the Import screen and no pages.
pub fn empty_view() -> ViewState {
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

/// An engine that answers with what the test gave it and remembers what it was told.
#[derive(Debug)]
pub struct FakeEngine {
    view: ViewState,
    dispatched: Vec<Command>,
    images: Vec<(ImageKey, Arc<Rgb8>)>,
    loupe: Option<LoupeCrop>,
    loupe_requests: Vec<(PhotoId, Point, f64)>,
    poll_result: bool,
    /// The error every command gets, when the test wants one.
    error: Option<String>,
}

impl Default for FakeEngine {
    fn default() -> FakeEngine {
        FakeEngine {
            view: empty_view(),
            dispatched: Vec::new(),
            images: Vec::new(),
            loupe: None,
            loupe_requests: Vec::new(),
            poll_result: false,
            error: None,
        }
    }
}

impl FakeEngine {
    /// An engine on the Import screen with nothing in it.
    pub fn new() -> FakeEngine {
        FakeEngine::default()
    }

    /// An engine that shows `view`.
    pub fn with_view(view: ViewState) -> FakeEngine {
        FakeEngine { view, ..FakeEngine::default() }
    }

    /// An engine that has an image for `key`.
    pub fn with_image(mut self, key: ImageKey, image: Arc<Rgb8>) -> FakeEngine {
        self.images.push((key, image));
        self
    }

    /// Every command the interface sent, in order.
    pub fn dispatched(&self) -> &[Command] {
        &self.dispatched
    }

    /// The last command the interface sent.
    pub fn last(&self) -> Option<&Command> {
        self.dispatched.last()
    }

    /// Forgets the commands sent so far.
    pub fn clear(&mut self) {
        self.dispatched.clear();
        self.loupe_requests.clear();
    }

    /// Shows another view.
    pub fn set_view(&mut self, view: ViewState) {
        self.view = view;
    }

    /// The loupe crop the engine hands out.
    pub fn set_loupe(&mut self, crop: Option<LoupeCrop>) {
        self.loupe = crop;
    }

    /// What the interface asked the engine for a loupe crop: the photo, the centre and the side.
    pub fn loupe_requests(&self) -> &[(PhotoId, Point, f64)] {
        &self.loupe_requests
    }

    /// Makes every command fail with `error`.
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }
}

impl EngineApi for FakeEngine {
    fn view(&self) -> ViewState {
        self.view.clone()
    }

    fn dispatch(&mut self, command: Command) -> Result<(), String> {
        self.dispatched.push(command);
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn poll(&mut self) -> bool {
        self.poll_result
    }

    fn image(&mut self, key: ImageKey) -> Option<Arc<Rgb8>> {
        self.images.iter().find(|(k, _)| *k == key).map(|(_, image)| image.clone())
    }

    fn loupe(&mut self, photo: PhotoId, centre: Point, side: f64) -> Option<LoupeCrop> {
        self.loupe_requests.push((photo, centre, side));
        self.loupe.clone()
    }
}
