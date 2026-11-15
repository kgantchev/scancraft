//! The truth file that comes with every fixture and synthetic scene (specification §10.1).
//!
//! A scene is a photo (`<name>.png`) plus `<name>.json` holding a [`SceneTruth`]. WP0's fixtures
//! and WP4's generator both write this format, and tests read it.

use crate::CoreError;
use crate::geom::{Point, Quad};
use crate::recipe::BendMode;
use serde::{Deserialize, Serialize};

pub const SCENE_FORMAT: u32 = 1;

/// The shape the page was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneShape {
    Flat,
    /// Bent without stretching: `profile` holds the cross-section's turning angle (radians) at
    /// evenly spaced points along its unrolled length.
    Bent {
        mode: BendMode,
        profile_turning: Vec<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneTruth {
    pub format: u32,
    /// What the scene is for, in words ("flat page on a dark desk").
    pub description: String,
    /// Photo size in pixels.
    pub size: (u32, u32),
    /// The camera's focal length in units of the long side.
    pub focal: f64,
    /// True page corners in photo pixels, TL, TR, BR, BL.
    pub corners: Quad,
    /// True width ÷ height of the flat page.
    pub ratio: f64,
    pub shape: SceneShape,
    /// For a spread: the spine's top and bottom ends in photo pixels.
    pub spine: Option<[Point; 2]>,
    /// Where each point of a grid over the flat page lies in the photo: `grid[j][i]` is page
    /// point (i / (n − 1), j / (n − 1)). Present for bent pages, so flattening can be scored.
    pub page_grid: Option<Vec<Vec<Point>>>,
}

impl SceneTruth {
    pub fn from_json(text: &str) -> Result<Self, CoreError> {
        let t: SceneTruth = serde_json::from_str(text).map_err(|e| CoreError::Scene(e.to_string()))?;
        if t.format != SCENE_FORMAT {
            return Err(CoreError::Scene(format!("format {} not supported", t.format)));
        }
        Ok(t)
    }

    pub fn to_json(&self) -> Result<String, CoreError> {
        serde_json::to_string_pretty(self).map_err(|e| CoreError::Scene(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{SceneShape, SceneTruth};

    fn fixture(name: &str) -> SceneTruth {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(format!("{name}.json"));
        SceneTruth::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// WP0's fixtures parse, their corners lie in the photo, and for bent pages the page grid's
    /// corners are the truth's corners.
    #[test]
    fn the_fixtures_are_consistent() {
        for name in ["flat", "curved", "spread"] {
            let t = fixture(name);
            let (w, h) = (f64::from(t.size.0), f64::from(t.size.1));
            assert!(t.corners.0.iter().all(|p| p.x > 0.0 && p.y > 0.0 && p.x < w && p.y < h), "{name}");
            if let Some(grid) = &t.page_grid {
                let n = grid.len() - 1;
                let ends = [grid[0][0], grid[0][n], grid[n][n], grid[n][0]];
                for (a, b) in ends.iter().zip(t.corners.0) {
                    assert!(a.distance(b) < 1e-9, "{name}");
                }
            }
            assert_eq!(matches!(t.shape, SceneShape::Flat), name == "flat");
            assert_eq!(t.spine.is_some(), name == "spread");
        }
    }
}
