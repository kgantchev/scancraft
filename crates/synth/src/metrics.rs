//! The metrics of §10.3. Owner: WP4.

use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::map::PageMap;
use scancraft_core::scene::SceneTruth;

fn unfinished<T>(what: &str) -> Result<T, CoreError> {
    Err(CoreError::Scene(format!("{what} is not implemented yet (WP4)")))
}

/// Overlap area over union area, by SmartDoc's protocol.
pub fn jaccard(found: &Quad, truth: &Quad) -> Result<f64, CoreError> {
    let _ = (found, truth);
    unfinished("jaccard")
}

/// The largest corner distance as a fraction of the photo's diagonal.
pub fn corner_error(found: &Quad, truth: &Quad, photo: (u32, u32)) -> Result<f64, CoreError> {
    let _ = (found, truth, photo);
    unfinished("corner_error")
}

/// The flattening error: the 95th percentile over the truth's page grid of the distance between
/// where `map` puts each page point and where it truly is, as a fraction of the page's diagonal.
pub fn flattening_error(map: &dyn PageMap, truth: &SceneTruth) -> Result<f64, CoreError> {
    let _ = (map, truth);
    unfinished("flattening_error")
}

/// A point helper kept here so metrics don't depend on the pipeline's geometry crate.
pub fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}
