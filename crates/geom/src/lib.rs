//! ScanCraft geometry (specification §6.1, L0).
//!
//! Owners: WP0 wrote A12; WP1 owns everything else here (build plan §4).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod homography;
pub mod proportions;
pub mod sheet;

use scancraft_core::CoreError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GeomError {
    #[error(transparent)]
    Core(#[from] CoreError),
    /// A degenerate input, such as a quadrilateral with three corners on a line.
    #[error("degenerate geometry: {0}")]
    Degenerate(&'static str),
}
