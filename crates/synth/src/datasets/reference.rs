//! The ScanCraft reference set (§10.2): at least 90 hand-labelled photos in 18 categories, kept
//! outside the repository because they show people's documents, and referenced by hash.
//!
//! The layout, which the specification leaves open, is a folder of photos and one `reference.json`:
//!
//! ```json
//! { "photos": [ {
//!     "file": "IMG_0412.HEIC",
//!     "sha256": "…hex…",
//!     "category": "dark desk",
//!     "corners": [[x, y], [x, y], [x, y], [x, y]],
//!     "ratio": 0.7071,
//!     "text": "the page's words, or", "text_file": "IMG_0412.txt",
//!     "spine": [[x, y], [x, y]],
//!     "order": "ltr"
//! } ] }
//! ```
//!
//! `corners` are TL, TR, BR, BL in photo pixels; `ratio`, the text and the spine are optional. A
//! photo whose bytes don't hash to `sha256` is skipped, so a changed file never passes for the
//! one that was labelled.

use super::{Dataset, MAX_SAMPLES, Sample, read_capped, scene};
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

/// The 18 categories of §10.2, from the shared types.
pub const CATEGORIES: [&str; 18] = scancraft_core::recipe::REFERENCE_CATEGORIES;

/// The six categories whose pages are bent (a spread is one category, in either reading order).
pub const BENT_CATEGORIES: [&str; 6] =
    ["open book page", "book spread", "curled receipt", "letter folded in three", "sheet held in the hand", "magazine"];

#[derive(Debug, Deserialize)]
struct Manifest {
    photos: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    file: String,
    sha256: String,
    category: String,
    corners: Option<[[f64; 2]; 4]>,
    ratio: Option<f64>,
    text: Option<String>,
    text_file: Option<String>,
    spine: Option<[[f64; 2]; 2]>,
    /// A spread's reading order, `"ltr"` or `"rtl"`; kept for the report, not scored here.
    #[allow(dead_code)]
    order: Option<String>,
}

/// The hex SHA-256 of a file's bytes, for writing a manifest.
pub fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && !Path::new(name).is_absolute() && !name.contains('\\')
}

/// Loads the reference set in `root`.
pub fn load(root: &Path) -> Result<Dataset, CoreError> {
    let manifest_path = root.join("reference.json");
    if !manifest_path.is_file() {
        return Err(scene(format!("{} has no reference.json", root.display())));
    }
    let text = String::from_utf8(read_capped(&manifest_path)?).map_err(|_| scene("reference.json is not text"))?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| scene(format!("reference.json: {e}")))?;
    let mut set = Dataset::new("reference");
    for entry in manifest.photos.into_iter().take(MAX_SAMPLES) {
        let file = &entry.file;
        if !safe_name(file) {
            set.skip(format!("{file}: the file name leaves the folder"));
            continue;
        }
        if !CATEGORIES.contains(&entry.category.as_str()) {
            set.skip(format!("{file}: {:?} is not one of the 18 categories", entry.category));
            continue;
        }
        let path = root.join(file);
        let bytes = match read_capped(&path) {
            Ok(b) => b,
            Err(e) => {
                set.skip(format!("{file}: {e}"));
                continue;
            }
        };
        if !hash_bytes(&bytes).eq_ignore_ascii_case(entry.sha256.trim()) {
            set.skip(format!("{file}: its bytes no longer hash to the labelled sha256"));
            continue;
        }
        let corners = entry.corners.map(|c| Quad(c.map(|[x, y]| Point::new(x, y))));
        if corners.is_some_and(|q| !q.is_finite()) || entry.ratio.is_some_and(|r| !r.is_finite() || r <= 0.0) {
            set.skip(format!("{file}: a corner or the ratio is not a usable number"));
            continue;
        }
        let mut sample = Sample::new(file.clone(), path);
        sample.category = Some(entry.category.clone());
        sample.corners = corners;
        sample.ratio = entry.ratio;
        sample.spine = entry.spine.map(|s| s.map(|[x, y]| Point::new(x, y)));
        sample.text = entry.text.or_else(|| entry.text_file.filter(|n| safe_name(n)).and_then(|n| std::fs::read_to_string(root.join(n)).ok()));
        set.samples.push(sample);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dir: &Path, manifest: &str) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"photo a").unwrap();
        std::fs::write(dir.join("a.txt"), "hello world").unwrap();
        std::fs::write(dir.join("reference.json"), manifest).unwrap();
    }

    fn entry(file: &str, sha: &str, category: &str, extra: &str) -> String {
        format!(r#"{{"file":"{file}","sha256":"{sha}","category":"{category}","corners":[[1,2],[3,2],[3,4],[1,4]]{extra}}}"#)
    }

    #[test]
    fn the_loader_reads_a_manifest_and_checks_the_hashes() {
        let dir = std::env::temp_dir().join(format!("scancraft-reference-{}", std::process::id()));
        let good = hash_bytes(b"photo a");
        assert_eq!(good.len(), 64);
        let manifest = format!(
            r#"{{"photos":[{},{},{},{},{}]}}"#,
            entry("a.jpg", &good, "dark desk", r#","ratio":0.7075,"text_file":"a.txt","spine":[[5,6],[7,8]]"#),
            entry("a.jpg", &"0".repeat(64), "dark desk", ""),
            entry("a.jpg", &good, "not a category", ""),
            entry("../a.jpg", &good, "wood", ""),
            entry("missing.jpg", &good, "wood", ""),
        );
        make(&dir, &manifest);
        let set = load(&dir).unwrap();
        assert_eq!(set.samples.len(), 1);
        assert_eq!(set.skipped_count, 4, "{:?}", set.skipped);
        let s = &set.samples[0];
        assert_eq!(s.category.as_deref(), Some("dark desk"));
        assert_eq!(s.text.as_deref(), Some("hello world"));
        assert_eq!(s.spine.unwrap()[1], Point::new(7.0, 8.0));
        assert_eq!(s.corners.unwrap().br(), Point::new(3.0, 4.0));
        assert!((s.ratio.unwrap() - 0.7075).abs() < 1e-12);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_or_broken_manifest_is_an_error() {
        let dir = std::env::temp_dir().join(format!("scancraft-reference-bad-{}", std::process::id()));
        make(&dir, "{ nope");
        assert!(matches!(load(&dir), Err(CoreError::Scene(_))));
        std::fs::remove_file(dir.join("reference.json")).unwrap();
        assert!(matches!(load(&dir), Err(CoreError::Scene(_))));
        std::fs::write(dir.join("reference.json"), r#"{"photos":"x"}"#).unwrap();
        assert!(load(&dir).is_err());
        std::fs::write(dir.join("reference.json"), []).unwrap();
        assert!(load(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn there_are_eighteen_categories_and_the_bent_ones_are_among_them() {
        assert_eq!(CATEGORIES.len(), 18);
        let unique: std::collections::BTreeSet<_> = CATEGORIES.iter().collect();
        assert_eq!(unique.len(), 18);
        assert!(BENT_CATEGORIES.iter().all(|c| CATEGORIES.contains(c)));
    }
}
