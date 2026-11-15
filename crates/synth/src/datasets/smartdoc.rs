//! SmartDoc 2015, challenge 1 (ICDAR 2015; CC BY 4.0): video frames of documents on desks, with
//! the true page corners in `metadata.csv.gz`.
//!
//! ```text
//! <root>/frames/background01/datasheet001/frame_0001.jpeg …
//! <root>/frames/metadata.csv.gz
//! ```
//!
//! The CSV has a header row; the columns used are `image_path` (relative to the frames folder),
//! `model_width`, `model_height` and the corners `tl_x tl_y bl_x bl_y br_x br_y tr_x tr_y`, in the
//! order top-left, **bottom-left**, bottom-right, top-right: the loader reorders them to ScanCraft's
//! TL, TR, BR, BL. A plain `metadata.csv` is read too, for a file already gunzipped.

use super::{Dataset, MAX_FILE_BYTES, MAX_SAMPLES, Sample, read_capped, scene};
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The metadata file and the folder the frames' paths are relative to.
fn find_metadata(root: &Path) -> Result<(PathBuf, PathBuf), CoreError> {
    for dir in [root.join("frames"), root.to_path_buf()] {
        for name in ["metadata.csv.gz", "metadata.csv"] {
            let path = dir.join(name);
            if path.is_file() {
                return Ok((path, dir));
            }
        }
    }
    Err(scene(format!("{} does not look like SmartDoc 2015: no frames/metadata.csv.gz or metadata.csv", root.display())))
}

fn read_text(path: &Path) -> Result<String, CoreError> {
    let raw = read_capped(path)?;
    let bytes = if path.extension().is_some_and(|e| e == "gz") {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(raw.as_slice())
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut out)
            .map_err(|e| scene(format!("{}: not a gzip file: {e}", path.display())))?;
        if out.len() as u64 > MAX_FILE_BYTES {
            return Err(scene(format!("{} is larger than {MAX_FILE_BYTES} bytes once unzipped", path.display())));
        }
        out
    } else {
        raw
    };
    String::from_utf8(bytes).map_err(|_| scene(format!("{} is not text", path.display())))
}

const CORNER_COLUMNS: [&str; 8] = ["tl_x", "tl_y", "bl_x", "bl_y", "br_x", "br_y", "tr_x", "tr_y"];

/// Parses the CSV into a `Dataset` of frames; rows that can't be used are skipped and reported.
pub fn parse(csv: &str, frames_dir: &Path) -> Result<Dataset, CoreError> {
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().ok_or_else(|| scene("the SmartDoc metadata is empty"))?.split(',').map(str::trim).collect();
    let column = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let image_col = column("image_path").ok_or_else(|| scene("the SmartDoc metadata has no image_path column"))?;
    let mut corner_cols = [0usize; 8];
    for (slot, name) in corner_cols.iter_mut().zip(CORNER_COLUMNS) {
        *slot = column(name).ok_or_else(|| scene(format!("the SmartDoc metadata has no {name} column")))?;
    }
    let (width_col, height_col) = (column("model_width"), column("model_height"));
    let mut set = Dataset::new("smartdoc2015");
    for (n, line) in lines.enumerate() {
        let row = n + 2;
        if line.trim().is_empty() {
            continue;
        }
        if set.samples.len() >= MAX_SAMPLES {
            set.skip(format!("row {row}: more than {MAX_SAMPLES} frames; the rest are ignored"));
            break;
        }
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let Some(path) = fields.get(image_col).filter(|p| !p.is_empty()) else {
            set.skip(format!("row {row}: no image path"));
            continue;
        };
        if path.contains("..") || Path::new(path).is_absolute() {
            set.skip(format!("row {row}: the image path {path:?} leaves the frames folder"));
            continue;
        }
        let mut v = [0f64; 8];
        let mut bad = None;
        for (k, (slot, &col)) in v.iter_mut().zip(&corner_cols).enumerate() {
            match fields.get(col).and_then(|f| f.parse::<f64>().ok()).filter(|x| x.is_finite()) {
                Some(x) => *slot = x,
                None => bad = Some(CORNER_COLUMNS[k]),
            }
        }
        if let Some(name) = bad {
            set.skip(format!("row {row}: {name} is missing or not a number"));
            continue;
        }
        // The file's order is tl, bl, br, tr; ScanCraft's is TL, TR, BR, BL.
        let [tl, bl, br, tr] = [Point::new(v[0], v[1]), Point::new(v[2], v[3]), Point::new(v[4], v[5]), Point::new(v[6], v[7])];
        let mut sample = Sample::new(*path, frames_dir.join(path));
        sample.corners = Some(Quad([tl, tr, br, bl]));
        let model = |c: Option<usize>| c.and_then(|c| fields.get(c)).and_then(|f| f.parse::<f64>().ok()).filter(|x| *x > 0.0);
        sample.ratio = model(width_col).zip(model(height_col)).map(|(w, h)| w / h);
        sample.category = fields.get(header.iter().position(|h| h.eq_ignore_ascii_case("bg_name")).unwrap_or(usize::MAX)).map(|s| (*s).to_string());
        set.samples.push(sample);
    }
    Ok(set)
}

/// Loads SmartDoc 2015 from `root`.
pub fn load(root: &Path) -> Result<Dataset, CoreError> {
    if !root.is_dir() {
        return Err(scene(format!("{} is not a directory", root.display())));
    }
    let (metadata, frames) = find_metadata(root)?;
    parse(&read_text(&metadata)?, &frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const HEADER: &str = "bg_name,bg_id,model_name,model_id,modeltype_name,modeltype_id,model_subid,image_path,frame_index,model_width,model_height,tl_x,tl_y,bl_x,bl_y,br_x,br_y,tr_x,tr_y";

    fn row(path: &str, c: [f64; 8]) -> String {
        format!(
            "background01,1,datasheet001,1,datasheet,1,1,{path},1,2100,2970,{},{},{},{},{},{},{},{}",
            c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]
        )
    }

    #[test]
    fn the_loader_reorders_the_corners() {
        // tl = (10, 20), bl = (12, 400), br = (300, 410), tr = (310, 25).
        let csv = format!("{HEADER}\n{}\n", row("background01/datasheet001/frame_0001.jpeg", [10.0, 20.0, 12.0, 400.0, 300.0, 410.0, 310.0, 25.0]));
        let set = parse(&csv, Path::new("/data/frames")).unwrap();
        assert_eq!(set.samples.len(), 1);
        let s = &set.samples[0];
        let q = s.corners.unwrap();
        assert_eq!(
            (q.tl(), q.tr(), q.br(), q.bl()),
            (Point::new(10.0, 20.0), Point::new(310.0, 25.0), Point::new(300.0, 410.0), Point::new(12.0, 400.0))
        );
        assert_eq!(s.image, Path::new("/data/frames/background01/datasheet001/frame_0001.jpeg"));
        assert!((s.ratio.unwrap() - 2100.0 / 2970.0).abs() < 1e-12);
        assert_eq!(s.category.as_deref(), Some("background01"));
    }

    #[test]
    fn the_loader_reads_a_small_set_plain_and_gzipped() {
        let dir = std::env::temp_dir().join(format!("scancraft-smartdoc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        let csv = format!("{HEADER}\n{}\n{}\n", row("a/f1.jpeg", [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]), row("a/f2.jpeg", [2.0; 8]));
        std::fs::write(dir.join("frames/metadata.csv"), &csv).unwrap();
        assert_eq!(load(&dir).unwrap().samples.len(), 2);
        // The gzipped file takes precedence when both are there.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(format!("{HEADER}\n{}\n", row("a/only.jpeg", [1.0; 8])).as_bytes()).unwrap();
        std::fs::write(dir.join("frames/metadata.csv.gz"), gz.finish().unwrap()).unwrap();
        let set = load(&dir).unwrap();
        assert_eq!(set.samples.len(), 1);
        assert_eq!(set.samples[0].id, "a/only.jpeg");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_loader_rejects_a_directory_that_is_not_a_dataset() {
        let dir = std::env::temp_dir().join(format!("scancraft-smartdoc-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load(&dir), Err(CoreError::Scene(_))));
        assert!(matches!(load(&dir.join("missing")), Err(CoreError::Scene(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_loader_survives_a_malformed_row() {
        let good = row("a/ok.jpeg", [1.0; 8]);
        let csv = [
            HEADER.to_string(),
            good.clone(),
            "background01,1,x".to_string(),
            row("a/nan.jpeg", [1.0; 8]).replacen("1,1,1,1,1,1,1,1", "nan,1,1,1,1,1,1,1", 1),
            row("../escape.jpeg", [1.0; 8]),
            row("", [1.0; 8]),
            row("a/x.jpeg", [3.0; 8]).replacen("3,3,3,3,3,3,3,3", "3,3,x,3,3,3,3,3", 1),
            String::new(),
            row("a/ok2.jpeg", [2.0; 8]),
        ]
        .join("\n");
        let set = parse(&csv, Path::new("f")).unwrap();
        assert_eq!(set.samples.len(), 2, "{:?}", set.skipped);
        assert_eq!(set.skipped_count, 5, "{:?}", set.skipped);
        assert!(set.skipped.iter().any(|m| m.contains("not a number")));
        // A header without the columns is an error; so is an empty file.
        assert!(parse("a,b,c\n1,2,3\n", Path::new("f")).is_err());
        assert!(parse("", Path::new("f")).is_err());
    }

    #[test]
    fn junk_files_are_errors_not_panics() {
        let dir = std::env::temp_dir().join(format!("scancraft-smartdoc-junk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        for (name, bytes) in [
            ("empty", Vec::new()),
            ("random", (0..300u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect()),
            ("truncated", b"\x1f\x8b\x08\x00".to_vec()),
        ] {
            std::fs::write(dir.join("frames/metadata.csv.gz"), bytes).unwrap();
            assert!(load(&dir).is_err(), "{name}");
        }
        // A directory where the file should be.
        std::fs::remove_file(dir.join("frames/metadata.csv.gz")).unwrap();
        std::fs::create_dir_all(dir.join("frames/metadata.csv")).unwrap();
        assert!(load(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
