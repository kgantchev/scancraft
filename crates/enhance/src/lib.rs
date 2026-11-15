//! ScanCraft page looks (specification §6.1, L2). Owner: WP9 (build plan §4).
//!
//! - [`even_light`] is A16: the lighting and shadow correction, with the object mask.
//! - [`looks`] is A17 (Color Document, Grayscale) and A18 (Black & White).
//! - [`upright`] is A15: the page turned upright, by Vision's vote or by the ink profiles.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod even_light;
pub mod helpers;
pub mod looks;
pub mod upright;

use scancraft_core::CoreError;
use scancraft_core::traits::ServiceError;
use scancraft_raster::RasterError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EnhanceError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Raster(#[from] RasterError),
    #[error(transparent)]
    Service(#[from] ServiceError),
}
