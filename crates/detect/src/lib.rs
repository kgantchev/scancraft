//! ScanCraft page detection and curved-page evidence (specification §6.1, L2).
//!
//! Owners (build plan §4), one module each so their branches don't meet:
//! - WP8: `edges` (A5), `contours` (A6), `lines` (A7), `select` (A9), `refine` (A10), `detect`;
//! - WP10: `text_lines` (A26), `spread` (A30);
//! - WP11: `curved_sides` (A25), and A9's change for curved sides inside `select`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod contours;
pub mod curved_sides;
pub mod detect;
pub mod edges;
pub mod lines;
pub mod refine;
pub mod select;
pub mod shapes;
pub mod spread;
pub mod text_lines;

use scancraft_core::CoreError;
use scancraft_core::traits::ServiceError;
use scancraft_geom::GeomError;
use scancraft_raster::RasterError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DetectError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Geom(#[from] GeomError),
    #[error(transparent)]
    Raster(#[from] RasterError),
    #[error(transparent)]
    Service(#[from] ServiceError),
}

/// Whether the page is lighter or darker than what lies outside a side (A10 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    PageLighter,
    PageDarker,
}
