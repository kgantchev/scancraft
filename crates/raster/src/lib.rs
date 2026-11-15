//! ScanCraft pixel work (specification §6.1, L1).
//!
//! Owners: WP0 wrote A3 (`downscale`); WP2 owns everything else here (build plan §4).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod blur;
pub mod colour;
pub mod downscale;
pub mod helpers;
pub mod ink;
pub mod resample;
pub mod warp;

use scancraft_core::CoreError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RasterError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("bad parameter: {0}")]
    BadParameter(&'static str),
}
