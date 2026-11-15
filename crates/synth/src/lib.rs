//! Synthetic scenes and evaluation (specification §10). Owner: WP4.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod metrics;

use scancraft_core::CoreError;
use scancraft_core::image::Rgb8;
use scancraft_core::scene::SceneTruth;

/// What to generate (§10.1).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSpec {
    pub count: u32,
    pub seed: u64,
    /// Bent pages (`--curved`) instead of flat ones.
    pub curved: bool,
}

/// One scene: the photo and its truth.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub photo: Rgb8,
    pub truth: SceneTruth,
}

/// Scene number `index` of `spec`; the same inputs always give the same bytes.
pub fn generate(spec: &SceneSpec, index: u32) -> Result<Scene, CoreError> {
    let _ = (spec, index);
    Err(CoreError::Scene("the generator is not implemented yet (WP4)".into()))
}
