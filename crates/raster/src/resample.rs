//! A14: separable Lanczos-3 resampling (specification A14). Owner: WP2.

use crate::RasterError;
use scancraft_core::image::{PlaneF32, RgbF32};
use scancraft_core::{Step, not_implemented};

pub fn lanczos3_rgb(src: &RgbF32, out_w: u32, out_h: u32) -> Result<RgbF32, RasterError> {
    let _ = (src, out_w, out_h);
    not_implemented(Step::A14)
}

pub fn lanczos3(src: &PlaneF32, out_w: u32, out_h: u32) -> Result<PlaneF32, RasterError> {
    let _ = (src, out_w, out_h);
    not_implemented(Step::A14)
}
