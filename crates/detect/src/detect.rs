//! The whole detection of one photo: A3 work image, A5–A9, A10, and A25 on the result.
//! Owner: WP8 (A25's call is added by WP11).

use crate::DetectError;
use crate::refine::Refined;
use scancraft_core::image::Rgb8;
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{CornerOrigin, SideShape};
use scancraft_core::traits::PageDetector;
use scancraft_core::{Step, not_implemented};

#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// Full-resolution corners and A10's side information.
    pub refined: Refined,
    pub origin: CornerOrigin,
    pub sides: [SideShape; 4],
    /// Kept for A25, A26 and A30, which sample from it.
    pub pyramid: LumaPyramid,
}

/// Detects the page in a full-resolution photo; `apple` is A8 when the platform has it.
pub fn detect_page(photo: &Rgb8, apple: Option<&dyn PageDetector>, seed: u64) -> Result<Detection, DetectError> {
    let _ = (photo, apple, seed);
    not_implemented(Step::A9)
}
