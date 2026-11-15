//! A25: curved sides (specification A25). Owner: WP11.

use crate::{DetectError, Polarity};
use scancraft_core::geom::Quad;
use scancraft_core::pyramid::LumaPyramid;
use scancraft_core::recipe::SideShape;
use scancraft_core::{Step, not_implemented};

/// Traces every side not marked in `skip` (border sides, a spine). `inward` is Book Spreads' rule
/// (A25 step 4). Returns top, right, bottom, left; skipped sides come back as `Straight`, and the
/// caller marks a spine as `Spine`.
pub fn trace_sides(
    corners: &Quad,
    polarity: &[Polarity; 4],
    pyramid: &LumaPyramid,
    skip: [bool; 4],
    inward: bool,
) -> Result<[SideShape; 4], DetectError> {
    let _ = (corners, polarity, pyramid, skip, inward);
    not_implemented(Step::A25)
}
