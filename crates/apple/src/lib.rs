//! Apple Vision behind ScanCraft's service traits (specification §6.7, L3). Owner: WP5.
//!
//! Only the two apps depend on this crate (`cargo xtask layers` checks it). The Vision calls
//! exist on macOS only. The steps around them (PNG encoding, coordinate conversion, word
//! splitting, score arithmetic, the proportional fallback, the quarter-turn rotation) do not
//! touch Apple frameworks, so they are built and tested on Linux as well.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// Off macOS the Vision module is not built, so the modules below are used by their tests only.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod parse;
mod pixels;
mod png;

#[cfg(target_os = "macos")]
mod vision;

#[cfg(test)]
mod testpage;

#[cfg(test)]
mod recorded;

#[cfg(target_os = "macos")]
pub use vision::{VisionPageDetector, VisionRecognizer, VisionUprightVoter};
