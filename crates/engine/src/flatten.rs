//! A29: when to flatten (specification A29). Owner: WP11.

use scancraft_core::recipe::{FlattenDecision, FlattenMode, Sheet, SideShape};
use scancraft_core::{CoreError, Step, not_implemented};
use scancraft_detect::text_lines::CopyLines;

/// What A29 weighs.
#[derive(Debug, Clone, PartialEq)]
pub struct FlattenInput<'a> {
    pub mode: FlattenMode,
    pub plane: Option<&'a Sheet>,
    pub curved: Option<&'a Sheet>,
    pub sides: &'a [SideShape; 4],
    /// A26's lines on the corner-straightened copy, and on the copy flattened by the curved fit.
    pub lines_corners: Option<&'a CopyLines>,
    pub lines_flat: Option<&'a CopyLines>,
    /// The page's diagonal in the photo, in pixels, for D95.
    pub diagonal_px: f64,
    /// Use Full Image or a PDF page: flattening unavailable.
    pub unavailable: bool,
}

pub fn decide(input: &FlattenInput<'_>) -> Result<FlattenDecision, CoreError> {
    let _ = input;
    not_implemented(Step::A29)
}

/// A29 check 4's straightness: RMS distance of line points from their own line's mean height.
/// `None` with no lines.
pub fn straightness(lines: &CopyLines) -> Option<f64> {
    let _ = lines;
    None
}
