//! A13: inverse-mapping warp with Catmull–Rom sampling (specification A13). Owner: WP2.
//! The mapping is A12's homography or A28's unrolled sheet, through `PageMap`.

use crate::RasterError;
use scancraft_core::image::{PlaneF32, RgbF32};
use scancraft_core::map::PageMap;
use scancraft_core::{Step, not_implemented};

/// Renders an `out_w × out_h` page from `src`; points outside the photo are white.
pub fn warp_rgb(src: &RgbF32, map: &dyn PageMap, out_w: u32, out_h: u32) -> Result<RgbF32, RasterError> {
    let _ = (src, map, out_w, out_h);
    not_implemented(Step::A13)
}

/// The same for one channel (analysis copies, A26, A29, A30).
pub fn warp_luma(src: &PlaneF32, map: &dyn PageMap, out_w: u32, out_h: u32) -> Result<PlaneF32, RasterError> {
    let _ = (src, map, out_w, out_h);
    not_implemented(Step::A13)
}
