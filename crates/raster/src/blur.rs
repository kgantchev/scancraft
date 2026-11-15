//! A4: Gaussian blur, direct for σ ≤ 8 px and three boxes above (specification A4). Owner: WP2.

use crate::RasterError;
use scancraft_core::image::PlaneF32;
use scancraft_core::{Step, not_implemented};

/// Blurs one channel; σ must lie in [0.3, 256].
pub fn gaussian(src: &PlaneF32, sigma: f32) -> Result<PlaneF32, RasterError> {
    let _ = (src, sigma);
    not_implemented(Step::A4)
}
