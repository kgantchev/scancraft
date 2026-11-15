//! What the interface sees of the engine (build plan WP6, WP7).
//!
//! The interface draws from a [`ViewState`] snapshot and changes things only by dispatching
//! [`Command`]s. Pixels arrive as shared images by [`ImageKey`]. WP7 builds against a fake that
//! implements [`EngineApi`]; WP13 swaps in the real engine.

use crate::command::Command;
use crate::geom::{Point, Quad};
use crate::image::Rgb8;
use crate::recipe::{CornerOrigin, FlattenDecision, FlattenMode, Half, Look, PageId, PhotoId, SideShape};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Import,
    Crop(PhotoId),
    Enhance(PageId),
    Export,
}

/// Screen 2's outline of one page cut from the current photo.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineView {
    pub page: PageId,
    pub corners: Quad,
    pub corners_from: CornerOrigin,
    pub sides: [SideShape; 4],
    /// The fitted outline when the page is flattened (A28 step 6), as one closed polyline.
    pub fitted: Option<Vec<Point>>,
    pub half: Option<Half>,
    /// `Some(reason)` when the shape is invalid (§4.3 "Invalid shape").
    pub invalid: Option<String>,
    pub flatten: FlattenMode,
    pub flatten_decision: Option<FlattenDecision>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropView {
    pub photo: PhotoId,
    /// "Photo 2 of 7".
    pub position: (u32, u32),
    pub photo_size: (u32, u32),
    pub outlines: Vec<OutlineView>,
    pub spine: Option<[Point; 2]>,
    /// A30 found a likely spread while Book Spreads is off.
    pub suggest_split: bool,
    pub auto_detect: bool,
    pub not_found: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnhanceView {
    pub page: PageId,
    pub look: Look,
    pub strength: u8,
    pub quarter_turns: u8,
    pub turned_automatically: bool,
    pub flattened_automatically: bool,
    pub half: Option<Half>,
    pub low_resolution: bool,
    /// The label "Add Page & Next" when photos or pages remain.
    pub more_to_come: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrayItem {
    pub page: PageId,
    pub number: u32,
    pub text_ready: bool,
}

/// A message for the user (§4.8).
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewState {
    pub screen: Screen,
    pub crop: Option<CropView>,
    pub enhance: Option<EnhanceView>,
    pub tray: Vec<TrayItem>,
    pub book_spreads: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Pages still being prepared, for progress indicators.
    pub busy: bool,
    pub notices: Vec<Notice>,
    /// Set when the last session was never exported (§4.2's restore banner), with its page count.
    pub restore_offer: Option<u32>,
}

/// An image the interface can ask for (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageKey {
    /// Screen 2's display copy, long side up to 3072 px.
    DisplayCopy(PhotoId),
    /// Screen 3's live preview, long side 1280 px.
    LookPreview(PageId),
    /// A filter-strip thumbnail, 512 px.
    FilterThumb(PageId, Look),
    /// A tray thumbnail, 256 px.
    TrayThumb(PageId),
}

/// A square cut from the full-resolution photo for the loupe (§6.5).
#[derive(Debug, Clone, PartialEq)]
pub struct LoupeCrop {
    pub image: Arc<Rgb8>,
    /// The photo position of the crop's top-left pixel and its scale (photo px per crop px).
    pub origin: Point,
    pub scale: f64,
}

/// The engine as the interface sees it.
pub trait EngineApi {
    fn view(&self) -> ViewState;
    /// Runs one command. Errors are returned for the interface to show, never panicked.
    fn dispatch(&mut self, command: Command) -> Result<(), String>;
    /// Collects finished background work; true when the view changed and needs a repaint.
    fn poll(&mut self) -> bool;
    /// The image for `key` if it is ready; the engine starts making it otherwise.
    fn image(&mut self, key: ImageKey) -> Option<Arc<Rgb8>>;
    /// A loupe crop centred on `centre`, about `side` photo pixels square.
    fn loupe(&mut self, photo: PhotoId, centre: Point, side: f64) -> Option<LoupeCrop>;
}
