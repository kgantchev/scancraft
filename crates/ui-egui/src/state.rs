//! What the interface remembers between frames: the keyboard's selection, the loupe, the tray's
//! drag, the export options, the settings and the uploaded images.
//!
//! None of it is the engine's business: the engine's state comes from `ViewState` and changes
//! only through commands.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Pos2, TextureHandle};
use scancraft_core::command::Corner;
use scancraft_core::engine_api::{EngineApi, ImageKey, Notice};
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
}

/// Everything the interface remembers between frames.
pub struct UiState {
    /// The strings, in the app's language.
    pub text: Text,
    /// The keyboard's selection on Screen 2.
    pub selection: Option<Selection>,
    /// The loupe is shown until this time, in seconds since the app started.
    pub loupe_until: Option<f64>,
    /// When the loupe was last shown, in the same clock.
    pub loupe_since: Option<f64>,
    /// Where the loupe is centred, in window points.
    pub loupe_centre: Option<Pos2>,
    /// The filter strip's selection: 0 is Original, 3 is Color Document.
    pub filter_index: usize,
    /// The page tray is hidden (⌥⌘T).
    pub tray_hidden: bool,
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
    /// The messages the interface itself shows (§4.8).
    pub notices: Vec<Notice>,
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
    /// The Import stage is shown above the stage the engine is at, to add more files.
    pub show_import: bool,
    /// How much work the last frame had (photos, tray pages), to tell when files were added.
    pub work_seen: (usize, usize),
}

/// How many messages the strip keeps; the oldest goes when there are more.
pub const MAX_NOTICES: usize = 8;

impl UiState {
    /// The state of a fresh session.
    pub fn new() -> UiState {
        UiState {
            text: Text::new(),
            selection: None,
            loupe_until: None,
            loupe_since: None,
            loupe_centre: None,
            filter_index: 3,
            tray_hidden: false,
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
            show_import: false,
            work_seen: (0, 0),
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
        let handle = ui.ctx().load_texture("scancraft-loupe", color_image(&crop.image), egui::TextureOptions::NEAREST);
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

    /// Shows the loupe for a second, centred at `centre` (§4.3).
    pub fn show_loupe(&mut self, now: f64, centre: Pos2) {
        self.loupe_until = Some(now + 1.0);
        self.loupe_since = Some(now);
        self.loupe_centre = Some(centre);
    }

    /// Whether the loupe is showing now.
    pub fn loupe_visible(&self, now: f64) -> bool {
        self.loupe_until.is_some_and(|until| now < until)
    }

    /// Adds a message for the user (§4.8).
    pub fn notice(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if self.notices.iter().any(|n| n.text == text) {
            return;
        }
        if self.notices.len() >= MAX_NOTICES {
            self.notices.remove(0);
        }
        self.notices.push(Notice { text, error });
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
            .field("tray_hidden", &self.tray_hidden)
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
