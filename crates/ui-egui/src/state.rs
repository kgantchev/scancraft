//! What the interface remembers between frames: the keyboard's selection, the loupe, the tray's
//! drag, the export options, the settings and the uploaded images.
//!
//! None of it is the engine's business: the engine's state comes from `ViewState` and changes
//! only through commands.

use std::collections::HashMap;
use std::sync::Arc;

use egui::TextureHandle;
use scancraft_core::command::Corner;
use scancraft_core::engine_api::{EngineApi, ImageKey};
use scancraft_core::geom::{Point, Side};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{ExportOptions, PageId, PageOrder, Paper, PhotoId};

use crate::text::Text;

/// What the keyboard has selected on Screen 2, and what the loupe's crosshair marks (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// One of the four corners of a page.
    Corner { page: PageId, corner: Corner },
    /// One of the three bend handles of a side: 0, 1 or 2.
    Bend { page: PageId, side: Side, index: usize },
    /// One of the spine's two ends: 0 is the top, 1 the bottom.
    Spine { photo: PhotoId, end: usize },
}

/// The Settings window's values (§4.6).
///
/// The engine keeps its own copy of these; the interface shows them and hands the changed ones
/// back. The contract has no command for them yet (see the package's report), so the two copies
/// are bridged by the app shell.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The paper of pages that don't snap to a standard size.
    pub default_paper: Paper,
    /// Snap to standard paper sizes.
    pub snap_to_paper_sizes: bool,
    /// Rotate pages upright automatically (A15).
    pub rotate_upright_automatically: bool,
    /// New pages start with Flatten on Auto when on, and on Off when off.
    pub flatten_curved_pages_automatically: bool,
    /// Which way the pages of a book are read.
    pub book_page_order: PageOrder,
    /// Language correction in text recognition.
    pub language_correction: bool,
    /// Keep tinted paper tinted instead of making it white.
    pub keep_paper_colour: bool,
    /// Where the sessions live.
    pub session_storage: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            default_paper: default_paper(),
            snap_to_paper_sizes: true,
            rotate_upright_automatically: true,
            flatten_curved_pages_automatically: true,
            book_page_order: default_page_order(),
            language_correction: true,
            keep_paper_colour: false,
            session_storage: default_session_storage(),
        }
    }
}

/// The paper of the system's region: US Letter in the regions the Unicode CLDR data lists for
/// Letter, A4 everywhere else (§4.6).
pub fn default_paper() -> Paper {
    match craft_i18n::Region::system().paper {
        craft_i18n::Paper::A4 => Paper::IsoA(4),
        craft_i18n::Paper::Letter => Paper::UsLetter,
    }
}

/// The book's page order: right to left when the system's first preferred language is written
/// right to left (§4.6).
pub fn default_page_order() -> PageOrder {
    let (i18n, _problems) = craft_i18n::I18n::new(&[], &[]);
    let lang = i18n.system_lang();
    if i18n.rtl(lang) { PageOrder::RightToLeft } else { PageOrder::LeftToRight }
}

/// Where the sessions live (§4.6).
pub fn default_session_storage() -> String {
    "~/Library/Application Support/ScanCraft/Sessions".to_owned()
}

/// An uploaded image: the pixels it came from and the texture they are in.
pub struct Texture {
    /// The image the texture was made of. The engine hands out the same `Arc` while the image is
    /// the same, so the pointer tells the interface whether the texture is still current.
    pub image: Arc<Rgb8>,
    /// The texture on the GPU.
    pub handle: TextureHandle,
}

/// The loupe's texture: the crop it was made of, and the texture itself.
pub struct LoupeTexture {
    /// The crop's top-left pixel, in photo pixels.
    pub origin: Point,
    /// Photo pixels per crop pixel.
    pub scale: f64,
    /// The crop the texture was made of.
    pub image: Arc<Rgb8>,
    /// The texture on the GPU.
    pub handle: TextureHandle,
}

/// What the magnifier is showing and for how long (§4.3).
///
/// It shows while a handle is pressed and goes the moment the button is released. A keyboard
/// nudge has no button to release, so it shows the loupe for a second instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoupeState {
    /// The photo position it magnifies: the handle's, in full-resolution photo pixels.
    pub focus: Option<Point>,
    /// A handle is pressed this frame.
    pub held: bool,
    /// After a keyboard nudge the loupe shows until this time, in seconds since the app started.
    pub until: Option<f64>,
    /// When the last nudge showed it, in the same clock.
    pub since: Option<f64>,
}

/// The photo's view on the canvas: how far it is zoomed in, and where the pan has put it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasView {
    /// 1 is the whole photo in the canvas; larger shows less of it, larger.
    pub zoom: f32,
    /// How far the photo's centre has moved from the canvas's centre, in points.
    pub pan: egui::Vec2,
}

impl Default for CanvasView {
    fn default() -> CanvasView {
        CanvasView { zoom: 1.0, pan: egui::Vec2::ZERO }
    }
}

/// The edge handle being dragged: where the side was when the drag began, and how far the
/// pointer has come along the side's outward normal since the last frame.
#[derive(Debug, Clone, Copy)]
pub struct EdgeDrag {
    /// The page whose side is being moved.
    pub page: PageId,
    /// The side being moved.
    pub side: Side,
    /// The side's middle when the drag began, in photo pixels.
    pub middle: Point,
    /// The side's outward normal when the drag began.
    pub normal: Point,
    /// The pointer's last projection on that normal, in photo pixels.
    pub last: f64,
}

/// Something the app shell has to do: open a native panel or a window (§4.6).
///
/// The interface itself cannot open a panel: `craft-dialogs` needs the window, and the window is
/// the shell's.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// Open the Open panel for photos and PDFs (§4.2).
    ImportFiles,
    /// Open the Save panel for the PDF (§4.6).
    ExportPdf { options: ExportOptions },
    /// Open the Settings window (§4.6).
    OpenSettings,
    /// Show the About panel.
    ShowAbout,
    /// The Settings window's values changed (§4.6).
    SettingsChanged(Settings),
    /// Open the folder panel for the reference set the labels are written to (§10.2).
    ChooseLabelFolder,
    /// Show a file in the Finder.
    RevealInFinder { path: String },
    /// Open a file with the program the system has for it.
    OpenFile { path: String },
}

/// The sheet over the window while a PDF is being written, and after it has been.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ExportSheet {
    /// No sheet.
    #[default]
    Closed,
    /// The PDF is being written: the progress, and Cancel.
    Running,
    /// The PDF was written: where it is, how big, and what to do with it.
    Finished(scancraft_core::engine_api::ExportSummary),
}

/// Everything the interface remembers between frames.
pub struct UiState {
    /// The strings, in the app's language.
    pub text: Text,
    /// The keyboard's selection on Screen 2.
    pub selection: Option<Selection>,
    /// The magnifier: where it looks, and whether a handle holds it open.
    pub loupe: LoupeState,
    /// The handle the pointer is pressing now, so it can be drawn as a ring.
    pub pressed_handle: Option<egui::Id>,
    /// How far off a handle's centre the press began, so a grab does not move the handle.
    pub grab: egui::Vec2,
    /// The photo's zoom and pan on the canvas, and the photo they belong to.
    pub canvas_view: CanvasView,
    /// The photo `canvas_view` is for: opening another photo starts again at the whole photo.
    pub canvas_view_of: Option<PhotoId>,
    /// A widget that uses the arrow keys itself (the strength slider) has the keyboard's focus this
    /// frame, so the arrow and number keys must not also change the look. It is set while the
    /// tools are drawn and read by the page canvas, which comes after them.
    pub keys_taken: bool,
    /// The finished page's zoom and pan on Screen 3, and the page they belong to.
    pub enhance_canvas_view: CanvasView,
    /// The page `enhance_canvas_view` is for.
    pub enhance_canvas_of: Option<PageId>,
    /// The filter strip's selection: 0 is Original, 3 is Color Document.
    pub filter_index: usize,
    /// The pages sidebar is hidden (⌥⌘T).
    pub sidebar_hidden: bool,
    /// The inspector is hidden (⌥⌘I).
    pub inspector_hidden: bool,
    /// Show Original is held, by the button or the \ key: the page shows with no filter on it.
    pub show_original: bool,
    /// The toolbar's overflow menu is open.
    pub toolbar_more: bool,
    /// The sheet over the window for the export.
    pub export_sheet: ExportSheet,
    /// The last export's summary the sheet has shown, so a new one is told from an old one.
    pub summary_seen: Option<scancraft_core::engine_api::ExportSummary>,
    /// The export options popover is open.
    pub export_popover: bool,
    /// The Settings window is open.
    pub settings_open: bool,
    /// The About panel is open.
    pub about_open: bool,
    /// The window that labels a photo for a reference set is open.
    pub label_open: bool,
    /// The category chosen in that window: an index into `REFERENCE_CATEGORIES`.
    pub label_category: usize,
    /// The folder of the reference set the labels are written to, once one has been chosen.
    pub label_folder: Option<String>,
    /// The export options (§4.6).
    pub export_options: ExportOptions,
    /// The Settings window's values (§4.6).
    pub settings: Settings,
    /// The messages floating over the window (§4.8).
    pub notices: Vec<Toast>,
    /// The texts of the engine's notices the last frame showed, so a notice is taken into
    /// [`UiState::notices`] once, however many frames the engine keeps it.
    pub engine_notices_seen: Vec<String>,
    /// The uploaded images, by key.
    pub textures: HashMap<ImageKey, Texture>,
    /// The tray's drag: the index it started at and the index it is over.
    pub tray_drag: Option<(u32, u32)>,
    /// The tray's selected page.
    pub tray_selected: Option<PageId>,
    /// The edge handle being dragged, when there is one.
    pub edge_drag: Option<EdgeDrag>,
    /// The loupe's texture: the crop it was made of, and the texture itself.
    pub loupe_texture: Option<LoupeTexture>,
    /// What the app shell has to do this frame.
    pub actions: Vec<UiAction>,
    /// The theme the palette was last applied for.
    pub theme: Option<egui::Theme>,
}

/// How many messages are kept at once; the oldest goes when there are more.
pub const MAX_NOTICES: usize = 8;
/// How long a message that is not an error stays on the screen, in seconds.
pub const TOAST_SECONDS: f64 = 5.0;
/// How long the end of that time is spent fading, in seconds.
pub const TOAST_FADE: f64 = 0.6;

/// A message that floats over the window: a short event, which fades after [`TOAST_SECONDS`], or
/// an error, which stays until it is closed (§4.8). It takes no room in the layout, so the screen
/// under it does not move when it comes or goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    /// What it says.
    pub text: String,
    /// An error: it stays until the user closes it.
    pub error: bool,
    /// It offers an Undo beside the text, which runs the general undo: the last thing done is the
    /// thing the message is about.
    pub undo: bool,
    /// When it first showed, in seconds on the app's clock. It is set on the first frame that
    /// draws it, because the code that makes a message has no clock.
    pub shown_at: Option<f64>,
}

impl Toast {
    /// Whether it has been up long enough to go: errors never do.
    pub fn expired(&self, now: f64) -> bool {
        !self.error && self.shown_at.is_some_and(|at| now - at >= TOAST_SECONDS)
    }

    /// How opaque it is at `now`: 1 until the last moments, then fading to 0.
    pub fn opacity(&self, now: f64) -> f32 {
        if self.error {
            return 1.0;
        }
        match self.shown_at {
            Some(at) => (((TOAST_SECONDS - (now - at)) / TOAST_FADE).clamp(0.0, 1.0)) as f32,
            None => 1.0,
        }
    }
}

impl UiState {
    /// The state of a fresh session.
    pub fn new() -> UiState {
        UiState {
            text: Text::new(),
            selection: None,
            loupe: LoupeState::default(),
            pressed_handle: None,
            grab: egui::Vec2::ZERO,
            canvas_view: CanvasView::default(),
            canvas_view_of: None,
            keys_taken: false,
            enhance_canvas_view: CanvasView::default(),
            enhance_canvas_of: None,
            filter_index: 3,
            sidebar_hidden: false,
            inspector_hidden: false,
            show_original: false,
            toolbar_more: false,
            export_sheet: ExportSheet::Closed,
            summary_seen: None,
            export_popover: false,
            settings_open: false,
            about_open: false,
            label_open: false,
            label_category: 0,
            label_folder: None,
            export_options: ExportOptions::default(),
            settings: Settings::default(),
            notices: Vec::new(),
            engine_notices_seen: Vec::new(),
            textures: HashMap::new(),
            tray_drag: None,
            tray_selected: None,
            edge_drag: None,
            loupe_texture: None,
            actions: Vec::new(),
            theme: None,
        }
    }

    /// The loupe's texture, uploaded when the crop changed.
    ///
    /// The crop moves with the pointer, so the texture is made again whenever its origin or its
    /// scale changes (§6.5).
    pub fn loupe_texture(&mut self, ui: &egui::Ui, crop: &scancraft_core::engine_api::LoupeCrop) -> Option<egui::TextureHandle> {
        if let Some(cached) = &self.loupe_texture
            && cached.origin == crop.origin
            && cached.scale == crop.scale
            && Arc::ptr_eq(&cached.image, &crop.image)
        {
            return Some(cached.handle.clone());
        }
        // Smoothed: nearest-neighbour sampling shimmers as the lens moves over text.
        let handle = ui.ctx().load_texture("scancraft-loupe", color_image(&crop.image), egui::TextureOptions::LINEAR);
        self.loupe_texture = Some(LoupeTexture { origin: crop.origin, scale: crop.scale, image: crop.image.clone(), handle: handle.clone() });
        Some(handle)
    }

    /// The texture for `key`, uploaded when the engine's image changed.
    ///
    /// The engine hands out the same `Arc` while its image is the same, so the pointer tells the
    /// interface whether the texture on the GPU is still current.
    pub fn texture(
        &mut self,
        ui: &egui::Ui,
        engine: &mut dyn EngineApi,
        key: ImageKey,
        options: egui::TextureOptions,
    ) -> Option<egui::TextureHandle> {
        let image = engine.image(key)?;
        self.texture_with(ui, key, image, options)
    }

    /// The texture for `key`, made from an image the caller already has.
    pub fn texture_with(&mut self, ui: &egui::Ui, key: ImageKey, image: Arc<Rgb8>, options: egui::TextureOptions) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.textures.get(&key)
            && Arc::ptr_eq(&cached.image, &image)
        {
            return Some(cached.handle.clone());
        }
        let handle = ui.ctx().load_texture(format!("scancraft-{key:?}"), color_image(&image), options);
        self.textures.insert(key, Texture { image, handle: handle.clone() });
        Some(handle)
    }

    /// Asks the shell for the Save panel.
    pub fn start_export(&mut self) {
        let options = self.export_options.clone();
        self.actions.push(UiAction::ExportPdf { options });
    }

    /// Shows the loupe for a second on `focus`, a photo position: what a keyboard nudge does,
    /// which has no button to release (§4.3).
    pub fn show_loupe(&mut self, now: f64, focus: Point) {
        self.loupe.until = Some(now + 1.0);
        self.loupe.since = Some(now);
        self.loupe.focus = Some(focus);
    }

    /// Keeps the loupe open on `focus`, a photo position, while a handle is pressed (§4.3).
    pub fn hold_loupe(&mut self, focus: Point) {
        self.loupe.held = true;
        self.loupe.focus = Some(focus);
    }

    /// Whether the loupe is showing now.
    pub fn loupe_visible(&self, now: f64) -> bool {
        self.loupe.held || self.loupe.until.is_some_and(|until| now < until)
    }

    /// Adds a message for the user (§4.8). Saying the same thing again shows it again.
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.push_toast(Toast { text: text.into(), error, undo: false, shown_at: None });
    }

    /// Adds a short message with an Undo beside it: "Page 3 deleted" (§4.8).
    pub fn toast_with_undo(&mut self, text: impl Into<String>) {
        self.push_toast(Toast { text: text.into(), error: false, undo: true, shown_at: None });
    }

    fn push_toast(&mut self, toast: Toast) {
        // The same words again are one message, shown from the start again.
        self.notices.retain(|n| n.text != toast.text);
        if self.notices.len() >= MAX_NOTICES {
            self.notices.remove(0);
        }
        self.notices.push(toast);
    }
}

impl Default for UiState {
    fn default() -> UiState {
        UiState::new()
    }
}

impl std::fmt::Debug for UiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiState")
            .field("selection", &self.selection)
            .field("filter_index", &self.filter_index)
            .field("sidebar_hidden", &self.sidebar_hidden)
            .field("export_options", &self.export_options)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// The photo position of a corner, for the loupe's crosshair and the accessible name.
pub fn corner_point(corners: &scancraft_core::geom::Quad, corner: Corner) -> Point {
    match corner {
        Corner::TopLeft => corners.tl(),
        Corner::TopRight => corners.tr(),
        Corner::BottomRight => corners.br(),
        Corner::BottomLeft => corners.bl(),
    }
}

/// The image as egui's own pixels.
pub fn color_image(image: &Rgb8) -> egui::ColorImage {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for pixel in image.data() {
        rgb.push(pixel[0]);
        rgb.push(pixel[1]);
        rgb.push(pixel[2]);
    }
    egui::ColorImage::from_rgb([width, height], &rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(error: bool) -> Toast {
        Toast { text: "Page 3 deleted".to_owned(), error, undo: true, shown_at: Some(100.0) }
    }

    #[test]
    fn a_short_message_goes_after_five_seconds() {
        let t = toast(false);
        assert!(!t.expired(104.9));
        assert!(t.expired(105.0));
    }

    #[test]
    fn an_error_stays_until_it_is_closed() {
        let t = toast(true);
        assert!(!t.expired(100.0 + 3600.0));
        assert_eq!(t.opacity(100.0 + 3600.0), 1.0);
    }

    #[test]
    fn a_short_message_fades_over_its_last_moments() {
        let t = toast(false);
        assert_eq!(t.opacity(101.0), 1.0);
        let fading = t.opacity(104.7);
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        assert_eq!(t.opacity(105.0), 0.0);
    }

    #[test]
    fn a_message_that_has_not_been_drawn_has_no_clock_yet() {
        let t = Toast { shown_at: None, ..toast(false) };
        assert!(!t.expired(1e9));
    }

    #[test]
    fn saying_the_same_thing_again_shows_it_again_from_the_start() {
        let mut state = UiState::new();
        state.toast_with_undo("Page 3 deleted");
        state.notices[0].shown_at = Some(1.0);
        state.toast_with_undo("Page 3 deleted");
        assert_eq!(state.notices.len(), 1);
        assert_eq!(state.notices[0].shown_at, None);
    }
}
