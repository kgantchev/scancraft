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

/// One photo of the batch, for the strip along Screen 2 that switches between them.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoTab {
    pub photo: PhotoId,
    /// A page of the photo to open with `PageOpenCrop`; `None` while the photo is still being read.
    pub page: Option<PageId>,
    pub name: String,
    /// Every page of the photo is in the tray.
    pub added: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropView {
    /// Every photo of the batch, in import order.
    pub photos: Vec<PhotoTab>,
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

/// Why a page is flagged for a second look: what the interface tells the user before it opens
/// Crop on that page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewReason {
    /// Detection found no page edges, so the outline is a guess.
    NotFound,
    /// Detection found edges but scored them low.
    LowScore,
    /// The outline is not a usable shape (§4.3 "Invalid shape").
    InvalidOutline,
    /// The photo looks like a two-page spread.
    LooksLikeSpread,
}

/// Where a page is in its life from import to a finished picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageState {
    /// The photo is still being read, cut, flattened and filtered: the page has no picture yet.
    Preparing,
    /// The page is made and nothing about it needs the user.
    Ready,
    /// The page is made, but its edges need checking, for the given reason, until the user has
    /// looked at it (`page.mark_reviewed`) or changed its crop.
    NeedsLook(ReviewReason),
    /// The page could not be made. The text says why, in words for the user.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrayItem {
    pub page: PageId,
    /// The photo the page was cut from. A photo that holds two pages (a spread) gives two items
    /// with the same photo, which is how the interface keeps a spread's halves together.
    pub photo: PhotoId,
    pub number: u32,
    pub text_ready: bool,
    pub state: PageState,
}

/// How far a piece of background work has got: `done` of `total` items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// The PDF written by the last export of the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    /// Where the file is.
    pub path: String,
    pub pages: u32,
    /// The file's size in bytes.
    pub bytes: u64,
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
    /// Photos being read and cut into pages after an import: how many are done of how many were
    /// added. `None` when no import is running.
    pub importing: Option<Progress>,
    /// Pages whose text is being recognised: how many are done of how many are in the document.
    /// `None` when every page has its text.
    pub reading_text: Option<Progress>,
    /// The PDF being written: how many pages are done of how many. `None` when no export is
    /// running. `export.cancel` stops it.
    pub exporting: Option<Progress>,
    /// The PDF the last export wrote, until the next scan starts.
    pub last_export: Option<ExportSummary>,
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
    /// The page's cropped, flattened and upright picture with no filter on it, long side 1280 px:
    /// what "Show Original" compares the filtered page against.
    OriginalPreview(PageId),
    /// A photo's thumbnail for Screen 2's strip, long side 256 px.
    PhotoThumb(PhotoId),
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
