//! The UVDoc benchmark (MIT): photographs of real bent and crumpled sheets with document images
//! mapped onto them, and an exact grid saying where each point of the flat page lies.
//!
//! ```text
//! <root>/grid2d/<name>.mat      89 × 61 × 2: x and y in the photo of each point of the flat page
//! <root>/img/<name>.png         the photo
//! ```
//!
//! The released `.mat` grids are written with `hdf5storage`, so they are **HDF5**, which no reader
//! in the workspace opens. The loader reads three formats for a grid and says so plainly when it
//! meets HDF5: a MATLAB v5 `.mat`, a NumPy `.npy` and a `.json`. To use the released files, convert
//! them once with Python: `np.save(name + ".npy", hdf5storage.loadmat(path)["grid2d"])`.

use super::{Dataset, MAX_SAMPLES, Sample, read_capped, scene};
use scancraft_core::CoreError;
use scancraft_core::geom::{Point, Quad};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The most points a grid may hold: a grid is a few thousand points, so a header that promises
/// millions is junk.
const MAX_GRID_VALUES: usize = 4_000_000;

/// A grid read from a file: rows × columns of photo positions.
pub type Grid = Vec<Vec<Point>>;

/// Turns the values of an `(a, b, 2)` array into rows × columns of points. `get(a, b, c)` reads
/// element (a, b, c).
fn grid_from(dims: &[usize], get: impl Fn(usize, usize, usize) -> f64) -> Result<Grid, CoreError> {
    let [rows, cols, parts] = dims else { return Err(scene(format!("a grid must have three dimensions, this has {}", dims.len()))) };
    if *parts != 2 || *rows < 2 || *cols < 2 {
        return Err(scene(format!("a grid must be at least 2 × 2 × 2, this is {rows} × {cols} × {parts}")));
    }
    let mut grid = Vec::with_capacity(*rows);
    for j in 0..*rows {
        let mut row = Vec::with_capacity(*cols);
        for i in 0..*cols {
            let p = Point::new(get(j, i, 0), get(j, i, 1));
            if !p.is_finite() {
                return Err(scene("a grid point is not finite"));
            }
            row.push(p);
        }
        grid.push(row);
    }
    Ok(grid)
}

// ---- NumPy .npy ----

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?)))
}

/// Reads a `.npy` file holding a float array of shape (rows, cols, 2).
pub fn read_npy(bytes: &[u8]) -> Result<Grid, CoreError> {
    let bad = |why: &str| scene(format!("not a usable .npy grid: {why}"));
    if bytes.get(..6) != Some(b"\x93NUMPY") {
        return Err(bad("no NUMPY magic"));
    }
    let major = *bytes.get(6).ok_or_else(|| bad("truncated"))?;
    let (len, start) = match major {
        1 => (u16_at(bytes, 8).ok_or_else(|| bad("truncated"))?, 10),
        2 | 3 => (u32::from_le_bytes(bytes.get(8..12).ok_or_else(|| bad("truncated"))?.try_into().map_err(|_| bad("truncated"))?) as usize, 12),
        _ => return Err(bad("an unknown version")),
    };
    let header =
        std::str::from_utf8(bytes.get(start..start + len).ok_or_else(|| bad("truncated header"))?).map_err(|_| bad("a header that is not text"))?;
    let descr = header.split("'descr'").nth(1).and_then(|t| t.split('\'').nth(1)).ok_or_else(|| bad("no descr"))?;
    let width = match descr {
        "<f8" | "=f8" | "float64" => 8,
        "<f4" | "=f4" | "float32" => 4,
        other => return Err(bad(&format!("the type {other} (only little-endian floats are read)"))),
    };
    let fortran = header.split("'fortran_order'").nth(1).is_some_and(|t| t.trim_start_matches([':', ' ']).starts_with("True"));
    let shape_text =
        header.split("'shape'").nth(1).and_then(|t| t.split('(').nth(1)).and_then(|t| t.split(')').next()).ok_or_else(|| bad("no shape"))?;
    let dims: Vec<usize> = shape_text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().map_err(|_| bad("a shape that is not numbers")))
        .collect::<Result<_, _>>()?;
    // The released grids are (rows, cols, 2); a channel-first (2, rows, cols) array is turned round.
    let channel_first = dims.len() == 3 && dims[0] == 2 && dims[2] != 2;
    let total =
        dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).filter(|&t| t <= MAX_GRID_VALUES).ok_or_else(|| bad("an unreasonable size"))?;
    let data = bytes.get(start + len..start + len + total * width).ok_or_else(|| bad("fewer values than the shape says"))?;
    let value = |k: usize| -> f64 {
        let chunk = &data[k * width..(k + 1) * width];
        if width == 8 {
            f64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]))
        } else {
            f64::from(f32::from_le_bytes(chunk.try_into().unwrap_or([0; 4])))
        }
    };
    let index = |d: &[usize], a: usize, b: usize, c: usize| if fortran { a + d[0] * (b + d[1] * c) } else { (a * d[1] + b) * d[2] + c };
    if channel_first {
        let (rows, cols) = (dims[1], dims[2]);
        grid_from(&[rows, cols, 2], |j, i, c| value(index(&dims, c, j, i)))
    } else {
        grid_from(&dims, |j, i, c| value(index(&dims, j, i, c)))
    }
}

// ---- JSON ----

/// Reads `{"grid2d": [[[x, y], …], …]}` or a bare array of rows of [x, y] points.
pub fn read_json(bytes: &[u8]) -> Result<Grid, CoreError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| scene(format!("not a usable JSON grid: {e}")))?;
    let rows = value.get("grid2d").unwrap_or(&value).as_array().ok_or_else(|| scene("a JSON grid is a list of rows"))?;
    let cols = rows.first().and_then(|r| r.as_array()).map_or(0, Vec::len);
    if rows.len() < 2 || cols < 2 || rows.len().saturating_mul(cols) > MAX_GRID_VALUES / 2 {
        return Err(scene("a JSON grid needs at least 2 × 2 points and not millions"));
    }
    let mut grid = Vec::with_capacity(rows.len());
    for row in rows {
        let row = row.as_array().filter(|r| r.len() == cols).ok_or_else(|| scene("the rows of a JSON grid differ in length"))?;
        let mut out = Vec::with_capacity(cols);
        for p in row {
            let xy = p.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)));
            let (x, y) = xy.ok_or_else(|| scene("a JSON grid point is not [x, y]"))?;
            let p = Point::new(x, y);
            if !p.is_finite() {
                return Err(scene("a grid point is not finite"));
            }
            out.push(p);
        }
        grid.push(out);
    }
    Ok(grid)
}

// ---- MATLAB v5 .mat ----

const MI_COMPRESSED: u32 = 15;
const MI_MATRIX: u32 = 14;

/// One data element: its type and bytes, and the offset of the next.
fn mat_element(b: &[u8], at: usize) -> Option<(u32, &[u8], usize)> {
    let word = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?);
    if word >> 16 != 0 {
        // The small element format: the length is in the top half, the data in the next 4 bytes.
        let len = (word >> 16) as usize;
        return Some((word & 0xFFFF, b.get(at + 4..at + 4 + len.min(4))?, at + 8));
    }
    let len = u32::from_le_bytes(b.get(at + 4..at + 8)?.try_into().ok()?) as usize;
    let data = b.get(at + 8..at + 8 + len)?;
    Some((word, data, at + 8 + len.div_ceil(8) * 8))
}

fn mat_numbers(kind: u32, data: &[u8]) -> Option<Vec<f64>> {
    Some(match kind {
        9 => data.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect(),
        7 => data.as_chunks::<4>().0.iter().map(|c| f64::from(f32::from_le_bytes(*c))).collect(),
        1 => data.iter().map(|&v| f64::from(v as i8)).collect(),
        2 => data.iter().map(|&v| f64::from(v)).collect(),
        3 => data.as_chunks::<2>().0.iter().map(|c| f64::from(i16::from_le_bytes(*c))).collect(),
        4 => data.as_chunks::<2>().0.iter().map(|c| f64::from(u16::from_le_bytes(*c))).collect(),
        5 => data.as_chunks::<4>().0.iter().map(|c| f64::from(i32::from_le_bytes(*c))).collect(),
        6 => data.as_chunks::<4>().0.iter().map(|c| f64::from(u32::from_le_bytes(*c))).collect(),
        _ => return None,
    })
}

/// The first numeric 3-D array of a v5 `.mat` file, or the one named `grid2d`.
pub fn read_mat(bytes: &[u8]) -> Result<Grid, CoreError> {
    let bad = |why: &str| scene(format!("not a usable MATLAB v5 .mat grid: {why}"));
    if bytes.get(..8) == Some(b"\x89HDF\r\n\x1a\n") || bytes.get(..19) == Some(b"MATLAB 7.3 MAT-file") {
        return Err(scene(
            "this .mat file is HDF5 (MATLAB v7.3), which nothing in the workspace reads; convert it once with np.save(name + \".npy\", hdf5storage.loadmat(path)[\"grid2d\"]) and point the loader at the .npy files",
        ));
    }
    if bytes.get(..14) != Some(b"MATLAB 5.0 MAT") || bytes.get(126..128) != Some(b"IM") {
        return Err(bad("not a little-endian MATLAB 5.0 file"));
    }
    let mut found: Option<(String, Vec<usize>, Vec<f64>)> = None;
    let mut at = 128;
    while at + 8 <= bytes.len() {
        let (kind, data, next) = mat_element(bytes, at).ok_or_else(|| bad("a truncated element"))?;
        at = next;
        let inflated;
        let element: &[u8] = if kind == MI_COMPRESSED {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(data)
                .take(super::MAX_FILE_BYTES)
                .read_to_end(&mut out)
                .map_err(|e| bad(&format!("a damaged compressed element: {e}")))?;
            inflated = out;
            &inflated
        } else {
            &[]
        };
        let (kind, body) = if kind == MI_COMPRESSED {
            let (k, d, _) = mat_element(element, 0).ok_or_else(|| bad("an empty compressed element"))?;
            (k, d)
        } else {
            (kind, data)
        };
        if kind != MI_MATRIX {
            continue;
        }
        // Array flags, dimensions, name, then the real part.
        let (_, _, p) = mat_element(body, 0).ok_or_else(|| bad("no array flags"))?;
        let (dk, dims_data, p) = mat_element(body, p).ok_or_else(|| bad("no dimensions"))?;
        let dims: Vec<usize> = mat_numbers(dk, dims_data).ok_or_else(|| bad("odd dimensions"))?.into_iter().map(|v| v as usize).collect();
        let (_, name, p) = mat_element(body, p).ok_or_else(|| bad("no name"))?;
        let (vk, values, _) = mat_element(body, p).ok_or_else(|| bad("no values"))?;
        let name = String::from_utf8_lossy(name).to_string();
        if dims.len() == 3 && dims[2] == 2 && (found.is_none() || name == "grid2d") {
            let numbers = mat_numbers(vk, values).ok_or_else(|| bad("an unsupported number type"))?;
            found = Some((name, dims, numbers));
        }
    }
    let (_, dims, numbers) = found.ok_or_else(|| bad("no array of shape rows × columns × 2"))?;
    let total =
        dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).filter(|&t| t <= MAX_GRID_VALUES).ok_or_else(|| bad("an unreasonable size"))?;
    if numbers.len() < total {
        return Err(bad("fewer values than the dimensions say"));
    }
    // MATLAB stores columns first.
    grid_from(&dims, |j, i, c| numbers[j + dims[0] * (i + dims[1] * c)])
}

// ---- the loader ----

fn find_root(root: &Path) -> Option<PathBuf> {
    ["", "UVDoc_benchmark", "UVDoc_final"].iter().map(|sub| root.join(sub)).find(|dir| dir.join("grid2d").is_dir())
}

fn read_grid(path: &Path) -> Result<Grid, CoreError> {
    let bytes = read_capped(path)?;
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("npy") => read_npy(&bytes),
        Some("json") => read_json(&bytes),
        Some("mat") => read_mat(&bytes),
        _ => Err(scene(format!("{}: not a .mat, .npy or .json grid", path.display()))),
    }
}

/// Loads the benchmark in `root` (or in `root/UVDoc_benchmark`).
pub fn load(root: &Path) -> Result<Dataset, CoreError> {
    let base = find_root(root).ok_or_else(|| scene(format!("{} does not look like the UVDoc benchmark: no grid2d folder", root.display())))?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(base.join("grid2d"))
        .map_err(|e| scene(format!("{}: {e}", base.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "mat" | "npy" | "json"))
        })
        .collect();
    files.sort();
    let mut set = Dataset::new("uvdoc");
    for path in files.into_iter().take(MAX_SAMPLES) {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let grid = match read_grid(&path) {
            Ok(g) => g,
            Err(e) => {
                set.skip(format!("{stem}: {e}"));
                continue;
            }
        };
        let candidates = ["img", "images", "."]
            .iter()
            .flat_map(|d| ["png", "jpg", "jpeg"].iter().map(|e| base.join(d).join(format!("{stem}.{e}"))).collect::<Vec<_>>());
        let image = candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| base.join("img").join(format!("{stem}.png")));
        let (rows, cols) = (grid.len(), grid[0].len());
        let mut sample = Sample::new(stem, image);
        sample.corners = Some(Quad([grid[0][0], grid[0][cols - 1], grid[rows - 1][cols - 1], grid[rows - 1][0]]));
        sample.category = Some("bent".to_string());
        sample.grid = Some(grid);
        set.samples.push(sample);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A (rows, cols, 2) grid whose point (j, i) is (10 i + j / 10, 20 j + i / 10).
    fn truth(rows: usize, cols: usize) -> Grid {
        (0..rows).map(|j| (0..cols).map(|i| Point::new(10.0 * i as f64 + j as f64 / 10.0, 20.0 * j as f64 + i as f64 / 10.0)).collect()).collect()
    }

    fn npy(g: &Grid, descr: &str, fortran: bool) -> Vec<u8> {
        let (rows, cols) = (g.len(), g[0].len());
        let header = format!("{{'descr': '{descr}', 'fortran_order': {}, 'shape': ({rows}, {cols}, 2), }}", if fortran { "True" } else { "False" });
        let mut padded = header.into_bytes();
        while (10 + padded.len() + 1) % 64 != 0 {
            padded.push(b' ');
        }
        padded.push(b'\n');
        let mut out = b"\x93NUMPY\x01\x00".to_vec();
        out.extend((padded.len() as u16).to_le_bytes());
        out.extend(padded);
        let value = |j: usize, i: usize, c: usize| if c == 0 { g[j][i].x } else { g[j][i].y };
        let mut order: Vec<(usize, usize, usize)> = Vec::new();
        if fortran {
            for c in 0..2 {
                for i in 0..cols {
                    order.extend((0..rows).map(|j| (j, i, c)));
                }
            }
        } else {
            for j in 0..rows {
                for i in 0..cols {
                    order.extend((0..2).map(|c| (j, i, c)));
                }
            }
        }
        for (j, i, c) in order {
            if descr == "<f8" { out.extend(value(j, i, c).to_le_bytes()) } else { out.extend((value(j, i, c) as f32).to_le_bytes()) }
        }
        out
    }

    fn element(kind: u32, data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_le_bytes().to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out.resize(out.len().div_ceil(8) * 8, 0);
        out
    }

    #[allow(clippy::needless_range_loop)]
    /// A MATLAB v5 file with one double array `name` of shape (rows, cols, 2), column-major.
    fn mat(g: &Grid, name: &str, compress: bool) -> Vec<u8> {
        let (rows, cols) = (g.len(), g[0].len());
        let mut body = element(6, &[6, 0, 0, 0, 0, 0, 0, 0]);
        body.extend(element(5, &[rows as u32, cols as u32, 2].iter().flat_map(|d| d.to_le_bytes()).collect::<Vec<u8>>()));
        body.extend(element(1, name.as_bytes()));
        let mut values = Vec::new();
        for c in 0..2 {
            for i in 0..cols {
                for j in 0..rows {
                    values.extend((if c == 0 { g[j][i].x } else { g[j][i].y }).to_le_bytes());
                }
            }
        }
        body.extend(element(9, &values));
        let matrix = element(14, &body);
        let mut out = b"MATLAB 5.0 MAT-file, made by the tests".to_vec();
        out.resize(124, b' ');
        out.extend([0, 1, b'I', b'M']);
        if compress {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(&matrix).unwrap();
            out.extend(element(15, &z.finish().unwrap()));
        } else {
            out.extend(matrix);
        }
        out
    }

    #[test]
    fn the_three_formats_read_the_same_grid() {
        let g = truth(5, 4);
        for (what, got) in [
            ("npy f8", read_npy(&npy(&g, "<f8", false))),
            ("npy f8 fortran", read_npy(&npy(&g, "<f8", true))),
            ("mat", read_mat(&mat(&g, "grid2d", false))),
            ("mat compressed", read_mat(&mat(&g, "grid2d", true))),
            (
                "json",
                read_json(
                    serde_json::json!({"grid2d": g.iter().map(|r| r.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>()).collect::<Vec<_>>()})
                        .to_string()
                        .as_bytes(),
                ),
            ),
        ] {
            assert_eq!(got.unwrap(), g, "{what}");
        }
        // Single precision keeps what a float can hold.
        let single = read_npy(&npy(&g, "<f4", false)).unwrap();
        assert!(single.iter().flatten().zip(g.iter().flatten()).all(|(a, b)| a.distance(*b) < 1e-4));
    }

    #[test]
    fn the_released_hdf5_files_are_named_for_what_they_are() {
        let mut hdf = b"\x89HDF\r\n\x1a\n".to_vec();
        hdf.extend([0u8; 64]);
        let Err(CoreError::Scene(message)) = read_mat(&hdf) else { panic!("not an error") };
        assert!(message.contains("HDF5") && message.contains("np.save"), "{message}");
    }

    #[test]
    fn the_loader_reads_a_small_set() {
        let dir = std::env::temp_dir().join(format!("scancraft-uvdoc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("UVDoc_benchmark/grid2d")).unwrap();
        std::fs::create_dir_all(dir.join("UVDoc_benchmark/img")).unwrap();
        let g = truth(6, 4);
        std::fs::write(dir.join("UVDoc_benchmark/grid2d/0001.npy"), npy(&g, "<f8", false)).unwrap();
        std::fs::write(dir.join("UVDoc_benchmark/grid2d/0002.mat"), mat(&g, "grid2d", true)).unwrap();
        std::fs::write(dir.join("UVDoc_benchmark/grid2d/0003.mat"), b"\x89HDF\r\n\x1a\n and more").unwrap();
        std::fs::write(dir.join("UVDoc_benchmark/img/0001.png"), b"x").unwrap();
        let set = load(&dir).unwrap();
        assert_eq!(set.samples.len(), 2);
        assert_eq!(set.skipped_count, 1);
        assert!(set.skipped[0].contains("HDF5"));
        let s = &set.samples[0];
        assert_eq!(s.id, "0001");
        assert_eq!(s.grid.as_ref().unwrap(), &g);
        assert!(s.image.ends_with("img/0001.png"));
        assert_eq!(s.corners.unwrap().br(), g[5][3]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_loader_rejects_a_directory_that_is_not_a_dataset() {
        let dir = std::env::temp_dir().join(format!("scancraft-uvdoc-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load(&dir), Err(CoreError::Scene(_))));
        assert!(load(&dir.join("nope")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn junk_grids_are_errors_not_panics() {
        let g = truth(3, 3);
        let good = npy(&g, "<f8", false);
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", Vec::new()),
            ("random", (0..400u32).map(|i| (i.wrapping_mul(2654435761) >> 20) as u8).collect()),
            ("truncated npy", good[..good.len() - 20].to_vec()),
            (
                "huge shape",
                b"\x93NUMPY\x01\x00\x46\x00{'descr': '<f8', 'fortran_order': False, 'shape': (99999, 99999, 2), }                   \n".to_vec(),
            ),
            ("truncated mat", mat(&g, "grid2d", false)[..200].to_vec()),
            ("mat with a bad zlib body", {
                let mut m = mat(&g, "grid2d", true);
                let n = m.len();
                m[n - 8] ^= 0xFF;
                m
            }),
        ];
        for (name, bytes) in &cases {
            assert!(read_npy(bytes).is_err() || name.contains("mat"), "{name} as npy");
            assert!(read_mat(bytes).is_err(), "{name} as mat");
            assert!(read_json(bytes).is_err(), "{name} as json");
        }
        // JSON of the wrong shapes.
        for text in
            ["[]", "[[]]", "[[[1,2],[3,4]],[[1,2]]]", r#"[[[1,2],[3,4]],[[1,"x"],[3,4]]]"#, "{\"grid2d\": 3}", "[[[1,2],[3,4]],[[1,2],[3,4e999]]]"]
        {
            assert!(read_json(text.as_bytes()).is_err(), "{text}");
        }
        // A directory where a grid file should be is skipped by the loader, not fatal.
        let dir = std::env::temp_dir().join(format!("scancraft-uvdoc-junk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("grid2d/0001.npy")).unwrap();
        std::fs::write(dir.join("grid2d/0002.npy"), b"junk").unwrap();
        let set = load(&dir).unwrap();
        assert!(set.samples.is_empty() && set.skipped_count >= 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
