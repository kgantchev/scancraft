//! The luma pyramid A10 builds and A25 and A26 sample from (specification A10).

use crate::image::PlaneF32;

/// Level 0 is the work image (long side 1024 px); each finer level doubles its size, made with
/// A3 from the full-resolution luma; the last level is the full-resolution luma itself.
#[derive(Debug, Clone, PartialEq)]
pub struct LumaPyramid {
    /// Coarse to fine.
    pub levels: Vec<PlaneF32>,
}

impl LumaPyramid {
    /// The finest level, the full-resolution luma.
    pub fn full(&self) -> Option<&PlaneF32> {
        self.levels.last()
    }

    /// The level whose long side is closest to `long_side` pixels (A25 asks for 2048).
    pub fn closest(&self, long_side: u32) -> Option<&PlaneF32> {
        self.levels.iter().min_by_key(|p| p.width().max(p.height()).abs_diff(long_side))
    }

    /// The coarsest level with at least `long_side` pixels on its long side, or the finest when
    /// none is that large (A26 step 1: enough pixels across the page that the copy isn't aliased).
    pub fn at_least(&self, long_side: u32) -> Option<&PlaneF32> {
        self.levels.iter().find(|p| p.width().max(p.height()) >= long_side).or(self.levels.last())
    }
}
