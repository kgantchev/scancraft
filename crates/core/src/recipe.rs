//! The page recipe (specification §6.2) and the session file that stores it (§6.6).
//!
//! Rendering is a pure function of a [`Page`], its source pixels and `ALGO_VERSION`.

use crate::camera::FocalSource;
use crate::geom::{Point, Quad};
use serde::{Deserialize, Serialize};

/// The session file's format version. Raise it when a stored type changes shape, and keep
/// reading the older versions.
pub const SESSION_FORMAT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PageId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PhotoId(pub u64);

/// A source file in the session's `sources/` folder, named by the hex SHA-256 of its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceId(pub String);

/// What a photo's EXIF says about its focal length (A1 step 5), for A11.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct FocalInput {
    /// `FocalLengthIn35mmFilm`, in millimetres.
    pub f35_mm: Option<f64>,
    /// `DigitalZoomRatio`.
    pub digital_zoom: Option<f64>,
}

/// Where an imported item came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImportOrigin {
    /// A photo file.
    File,
    /// Page `index` (from 0) of an imported PDF, rendered by A24.
    PdfPage { index: u32 },
    /// Page `index` of a multi-page TIFF.
    TiffPage { index: u32 },
}

/// One imported item waiting in the batch, before and after it becomes pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhotoItem {
    pub id: PhotoId,
    pub source: SourceId,
    /// The file name shown in messages; never written to logs (§1.2).
    pub name: String,
    pub origin: ImportOrigin,
    /// Oriented, full-resolution size in pixels.
    pub size: (u32, u32),
    pub focal: FocalInput,
}

/// Which detector found the corners (Screen 2's Auto-Detect tooltip).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Detector {
    Apple,
    Contours,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CornerOrigin {
    Detected {
        by: Detector,
        score: f32,
    },
    /// Detection found nothing; the corners sit 3% inside the photo's edges (A9 step 6.5).
    NotFound,
    Manual,
    FullImage,
}

/// One point A25 traced on a side, in photo pixels, with its step strength as weight.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SideSample {
    pub point: Point,
    pub weight: f32,
}

/// The shape of one side of the outline (A25, Screen 2's bend handles).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum SideShape {
    #[default]
    Straight,
    /// Traced by A25. `curved` is A25 step 6's verdict; `bow` is a fraction of the side's length.
    Traced { samples: Vec<SideSample>, bow: f32, curved: bool },
    /// Bent by the user: the bend handles' photo positions at a quarter, half and three quarters
    /// of the side.
    Bent { handles: [Point; 3] },
    /// The spine of a spread (A30): not a paper edge; never traced or scored.
    Spine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Half {
    Left,
    Right,
}

/// A page cut from a book spread (A30).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpreadHalf {
    pub half: Half,
    /// The spine's top and bottom ends, in photo pixels.
    pub spine: [Point; 2],
    /// The whole spread's outline, which both halves share.
    pub spread_corners: Quad,
    /// A30 step 3's confidence; 0 for a cut at the centre line.
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FlattenMode {
    #[default]
    Auto,
    On,
    Off,
}

/// A27's two bend directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BendMode {
    /// Rulings run down the page, as in a book.
    Across,
    /// Rulings run across the page, as in a letter folded in three.
    Down,
}

/// A fitted sheet (A27): everything A28 needs to unroll it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub mode: BendMode,
    /// The rotation R, row-major.
    pub rotation: [f64; 9],
    pub translation: [f64; 3],
    /// The profile's chord length, in units of the ruling length.
    pub chord: f64,
    /// The 10 free knot values of the profile η.
    pub profile: [f64; 10],
    /// The focal length the fit used, in units of the long side.
    pub focal: f64,
    pub focal_source: FocalSource,
    /// The fit's final cost F.
    pub cost: f64,
}

/// Why A29 flattened a page or left it flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlattenReason {
    /// Flatten is Off.
    Off,
    /// Flatten is On and the fit was valid.
    On,
    /// Flatten is On but the fit was invalid or had nothing to fit (§4.8's note).
    OnButNoFit,
    /// Auto, check 1: too few lines and no pair of curved sides.
    NoEvidence,
    /// Auto, check 2: D95 under 0.2%.
    NoBend,
    /// Auto, check 3: not enough gain over the plane.
    NoGain,
    /// Auto, check 4: lines not straighter.
    NotStraighter,
    /// Auto: every check passed.
    Straighter,
    /// Use Full Image or a PDF page: flattening unavailable.
    Unavailable,
}

/// A29's record (A29 "Record").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlattenDecision {
    pub flattened: bool,
    pub reason: FlattenReason,
    /// D95 as a fraction of the page's diagonal in the photo.
    pub d95: f64,
    pub plane_cost: f64,
    pub curved_cost: f64,
    /// Straightness in analysis pixels, corners only and flattened; `None` when not measured.
    pub straightness_corners: Option<f64>,
    pub straightness_flat: Option<f64>,
    pub lines_corners: u32,
    pub lines_flat: u32,
}

/// The fitted sheet a page was flattened with, and the decision behind it (§6.2 `sheet`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetFit {
    pub sheet: Sheet,
    pub decision: FlattenDecision,
}

/// The standard papers of A11 step 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paper {
    /// ISO A series, with its number (4 for A4).
    IsoA(u8),
    UsLetter,
    UsLegal,
    Id1,
    BusinessCard,
}

/// How A11 or A28 found a page's ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RatioSource {
    KnownFocal,
    EstimatedFocal,
    TypicalFocal,
    SideLengths,
    /// From an unrolled sheet (A28 step 2).
    Unrolled,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Proportions {
    /// Width ÷ height of the page as it lies in the photo.
    pub ratio: f64,
    pub source: RatioSource,
    /// Set when the ratio was snapped (A11 step 5).
    pub paper: Option<Paper>,
    /// A11 step 7's native size in pixels.
    pub native_size: (u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOrigin {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Look {
    Original,
    BlackWhite,
    Grayscale,
    #[default]
    ColorDocument,
}

impl Look {
    /// The default strength of §4.4, `None` for Original.
    pub const fn default_strength(self) -> Option<u8> {
        match self {
            Look::Original => None,
            Look::BlackWhite => Some(55),
            Look::Grayscale | Look::ColorDocument => Some(60),
        }
    }
}

/// One recognised word (A20 step 5), in page pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    /// left, top, right, bottom.
    pub rect: [f32; 4],
    pub confidence: f32,
    /// "vision" or "ocrs".
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextLine {
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecognizedText {
    pub engine: String,
    /// The engine's version, and for Vision the macOS version.
    pub engine_version: String,
    pub lines: Vec<TextLine>,
}

/// The page recipe (§6.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    /// The batch item it was cut from.
    pub photo: PhotoId,
    pub source: SourceId,
    /// TL, TR, BR, BL in photo pixels (§7.2).
    pub corners: Quad,
    pub corners_from: CornerOrigin,
    /// top, right, bottom, left.
    pub sides: [SideShape; 4],
    pub spread: Option<SpreadHalf>,
    pub flatten: FlattenMode,
    /// The fitted sheet when the page is flattened.
    pub sheet: Option<SheetFit>,
    pub proportions: Option<Proportions>,
    pub page_size_mm: Option<(f64, f64)>,
    /// 0..=3, clockwise.
    pub quarter_turns: u8,
    pub turns_from: TurnOrigin,
    pub look: Look,
    /// 0..=100.
    pub strength: u8,
    pub text: Option<RecognizedText>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageSizeOption {
    #[default]
    Auto,
    A4,
    UsLetter,
    UsLegal,
    A5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    Small,
    #[default]
    Balanced,
    Best,
}

/// Export options (§4.6), saved between sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub page_size: PageSizeOption,
    pub quality: Quality,
    pub searchable: bool,
    /// Empty for Automatic, else BCP 47 language tags.
    pub languages: Vec<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { page_size: PageSizeOption::Auto, quality: Quality::Balanced, searchable: true, languages: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageOrder {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// `session.json` (§6.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionFile {
    pub format: u32,
    pub photos: Vec<PhotoItem>,
    /// In tray order.
    pub pages: Vec<Page>,
    pub export: ExportOptions,
    pub book_spreads: bool,
    pub exported: bool,
}

#[cfg(test)]
mod tests {
    use super::{CornerOrigin, FlattenMode, Look, Page, PageId, PhotoId, SESSION_FORMAT, SessionFile, SideShape, SourceId};
    use crate::geom::{Point, Quad};

    #[test]
    fn a_session_round_trips_through_json() {
        let page = Page {
            id: PageId(1),
            photo: PhotoId(1),
            source: SourceId("ab".into()),
            corners: Quad([Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)]),
            corners_from: CornerOrigin::Manual,
            sides: [SideShape::Straight, SideShape::Spine, SideShape::Straight, SideShape::Straight],
            spread: None,
            flatten: FlattenMode::Auto,
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: super::TurnOrigin::Manual,
            look: Look::ColorDocument,
            strength: 60,
            text: None,
        };
        let s = SessionFile {
            format: SESSION_FORMAT,
            photos: vec![],
            pages: vec![page],
            export: Default::default(),
            book_spreads: false,
            exported: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
