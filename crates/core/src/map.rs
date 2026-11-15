//! The mapping from the flat output page to the photo, shared by A12's homography and A28's
//! unrolled sheet, and used by A13's warp.

use crate::geom::Point;

/// Which way a mapping is cheapest to evaluate step by step: along rows (a homography, A13) or
/// along columns (a sheet bent across the page, A28 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOrder {
    Rows,
    Columns,
}

/// Maps page coordinates (u, v) ∈ [0, 1]², u across and v down, to photo pixels.
pub trait PageMap: Send + Sync {
    /// The photo position of page point (u, v), or `None` where the mapping is undefined.
    fn map(&self, u: f64, v: f64) -> Option<Point>;

    /// The order in which [`PageMap::map_span`] is fastest.
    fn scan_order(&self) -> ScanOrder {
        ScanOrder::Rows
    }

    /// Fills `out` with the positions of `out.len()` pixel centres along one row (`order` Rows,
    /// `fixed` = v) or one column (`order` Columns, `fixed` = u) of an output `out.len()` pixels
    /// long: sample k sits at (k + ½) / len. Implementations may evaluate incrementally; this
    /// default calls [`PageMap::map`] for each sample.
    fn map_span(&self, order: ScanOrder, fixed: f64, out: &mut [Option<Point>]) {
        let n = out.len().max(1) as f64;
        for (k, slot) in out.iter_mut().enumerate() {
            let t = (k as f64 + 0.5) / n;
            *slot = match order {
                ScanOrder::Rows => self.map(t, fixed),
                ScanOrder::Columns => self.map(fixed, t),
            };
        }
    }
}
