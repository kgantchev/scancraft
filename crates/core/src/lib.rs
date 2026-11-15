//! ScanCraft's shared contract (build plan WP0).
//!
//! Every type and trait that more than one work package uses lives here, so the packages can be
//! built in parallel against fixed definitions. Changing anything in this crate is a `contract:`
//! pull request (build plan §2 rule 2), never part of a package's branch.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod camera;
pub mod command;
pub mod determinism;
pub mod engine_api;
pub mod error;
pub mod geom;
pub mod image;
pub mod map;
pub mod pyramid;
pub mod recipe;
pub mod scene;
pub mod traits;

pub use error::{CoreError, Step, not_implemented};

/// Changes whenever any algorithm's output changes (specification §7.6). Part of every cache key
/// and written into the PDF's Producer field.
pub const ALGO_VERSION: u32 = 1;
