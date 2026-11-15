//! Apple Vision behind ScanCraft's service traits (specification §6.7, L3). Owner: WP5.
//!
//! Only the two apps depend on this crate (`cargo xtask layers` checks it). Off macOS it holds
//! nothing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "macos")]
mod vision;

#[cfg(target_os = "macos")]
pub use vision::{VisionPageDetector, VisionRecognizer, VisionUprightVoter};
