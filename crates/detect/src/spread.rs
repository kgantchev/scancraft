//! A30: book spreads (specification A30). Owner: WP10.

use crate::DetectError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::{Half, SideShape};
use scancraft_core::{Step, not_implemented};

/// A spine found by steps 1–5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spine {
    /// Top and bottom ends in photo pixels.
    pub ends: [Point; 2],
    /// Step 3's confidence; 0 for a cut at the centre line.
    pub confidence: f32,
}

/// Steps 1–5: `ratio` is A11's ratio of the whole outline. `None` means one page.
pub fn find_spine(pyramid: &LumaPyramid, outline: &Quad, ratio: f64) -> Result<Option<Spine>, DetectError> {
    let _ = (pyramid, outline, ratio);
    not_implemented(Step::A30)
}

/// One page of a split spread (step 6).
#[derive(Debug, Clone, PartialEq)]
pub struct HalfOutline {
    pub half: Half,
    pub corners: Quad,
    pub sides: [SideShape; 4],
}

/// Step 6: cuts the outline and its sides along the spine; left page first.
pub fn split(outline: &Quad, sides: &[SideShape; 4], spine: &Spine) -> Result<[HalfOutline; 2], DetectError> {
    let _ = (outline, sides, spine);
    not_implemented(Step::A30)
}
