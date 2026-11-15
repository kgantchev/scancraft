//! The synthetic scenes `scancraft-cli synth` writes: `scene_00000.png` with its `.json` truth,
//! `.ink.png` and `.txt`.

use super::{Dataset, MAX_SAMPLES, Sample, scene};
use scancraft_core::CoreError;
use scancraft_core::scene::SceneTruth;
use std::path::Path;

/// Loads every scene in `dir`, in name order.
pub fn load(dir: &Path) -> Result<Dataset, CoreError> {
    if !dir.is_dir() {
        return Err(scene(format!("{} is not a directory", dir.display())));
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| scene(format!("{}: {e}", dir.display())))?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && !n.ends_with(".ink.json"))
        .map(|n| n.trim_end_matches(".json").to_string())
        .collect();
    names.sort();
    let mut set = Dataset::new("synthetic");
    for name in names.into_iter().take(MAX_SAMPLES) {
        let json = dir.join(format!("{name}.json"));
        let truth = match std::fs::read_to_string(&json).map_err(|e| scene(e.to_string())).and_then(|t| SceneTruth::from_json(&t)) {
            Ok(t) => t,
            Err(e) => {
                set.skip(format!("{name}: {e}"));
                continue;
            }
        };
        let mut sample = Sample::new(&name, dir.join(format!("{name}.png")));
        sample.corners = Some(truth.corners);
        sample.ratio = Some(truth.ratio);
        sample.grid = truth.page_grid.clone();
        sample.spine = truth.spine;
        sample.category = Some(truth.description.split(" on ").next().unwrap_or("scene").to_string());
        sample.text = std::fs::read_to_string(dir.join(format!("{name}.txt"))).ok();
        let ink = dir.join(format!("{name}.ink.png"));
        sample.ink = ink.is_file().then_some(ink);
        set.samples.push(sample);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneSpec;
    use crate::scene_io::generate_set;

    #[test]
    fn a_generated_set_loads_back_with_its_truth() {
        let dir = std::env::temp_dir().join(format!("scancraft-synthetic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        generate_set(&SceneSpec { count: 2, seed: 3, curved: true }, &dir).unwrap();
        let set = load(&dir).unwrap();
        assert_eq!(set.samples.len(), 2);
        let s = &set.samples[0];
        assert!(s.grid.is_some() && s.corners.is_some() && s.text.as_deref().is_some_and(|t| t.lines().count() > 20));
        assert!(s.ink.is_some() && s.image.is_file());
        assert!(s.size().unwrap().0 > 0);
        // A broken truth file is skipped with a reason.
        std::fs::write(dir.join("scene_00009.json"), "{ not json").unwrap();
        let set = load(&dir).unwrap();
        assert_eq!((set.samples.len(), set.skipped_count), (2, 1));
        assert!(load(&dir.join("nothing")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
