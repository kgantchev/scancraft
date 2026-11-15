//! Writing and reading a scene on disk: `<name>.png` (the photo, 8-bit sRGB), `<name>.json` (a
//! `SceneTruth`), `<name>.ink.png` (the page's true ink, 8-bit grey, 255 = ink, in the flat page's
//! own 300 DPI pixels, the layout of `BitPlane`'s rows) and `<name>.txt` (the page's words).

use crate::generate::{Generated, generate_full};
use crate::{Scene, SceneSpec};
use scancraft_core::CoreError;
use scancraft_core::image::{BitPlane, Plane, Rgb8};
use scancraft_core::scene::SceneTruth;
use std::path::Path;

fn io_error(what: &Path, e: impl std::fmt::Display) -> CoreError {
    CoreError::Scene(format!("{}: {e}", what.display()))
}

/// Saves 8-bit pixels as a PNG, checking the length first (the encoder panics on a mismatch).
fn save_png(path: &Path, bytes: &[u8], width: u32, height: u32, grey: bool) -> Result<(), CoreError> {
    let channels = if grey { 1 } else { 3 };
    if bytes.len() != width as usize * height as usize * channels {
        return Err(io_error(path, "the pixel buffer has the wrong length"));
    }
    let color = if grey { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 };
    image::save_buffer(path, bytes, width, height, color).map_err(|e| io_error(path, e))
}

/// Writes the four files of a scene into `dir`.
pub fn write_scene(dir: &Path, name: &str, g: &Generated) -> Result<(), CoreError> {
    let photo = &g.scene.photo;
    let flat: Vec<u8> = photo.data().iter().flatten().copied().collect();
    save_png(&dir.join(format!("{name}.png")), &flat, photo.width(), photo.height(), false)?;
    let json = g.scene.truth.to_json()?;
    let json_path = dir.join(format!("{name}.json"));
    std::fs::write(&json_path, json + "\n").map_err(|e| io_error(&json_path, e))?;
    let mut ink = Vec::with_capacity(g.ink.width() as usize * g.ink.height() as usize);
    for y in 0..g.ink.height() {
        for x in 0..g.ink.width() {
            ink.push(if g.ink.get(x, y) == Some(true) { 255u8 } else { 0 });
        }
    }
    save_png(&dir.join(format!("{name}.ink.png")), &ink, g.ink.width(), g.ink.height(), true)?;
    let text_path = dir.join(format!("{name}.txt"));
    std::fs::write(&text_path, &g.text).map_err(|e| io_error(&text_path, e))
}

/// The name of scene `index`: `scene_00000`.
pub fn scene_name(index: u32) -> String {
    format!("scene_{index:05}")
}

/// Generates the whole set of `spec` into `out` (created if needed); returns how many scenes.
pub fn generate_set(spec: &SceneSpec, out: &Path) -> Result<usize, CoreError> {
    if spec.count == 0 {
        return Ok(0);
    }
    std::fs::create_dir_all(out).map_err(|e| io_error(out, e))?;
    for index in 0..spec.count {
        write_scene(out, &scene_name(index), &generate_full(spec, index)?)?;
    }
    Ok(spec.count as usize)
}

/// Reads a photo PNG back as 8-bit RGB.
pub fn read_photo(path: &Path) -> Result<Rgb8, CoreError> {
    let img = image::open(path).map_err(|e| io_error(path, e))?.to_rgb8();
    let (w, h) = img.dimensions();
    let (pixels, _) = img.as_raw().as_chunks::<3>();
    Ok(Plane::from_vec(w, h, pixels.to_vec())?)
}

/// Reads an ink mask PNG (any grey above the middle is ink).
pub fn read_ink(path: &Path) -> Result<BitPlane, CoreError> {
    let img = image::open(path).map_err(|e| io_error(path, e))?.to_luma8();
    let (w, h) = img.dimensions();
    let mut bits = BitPlane::new(w, h)?;
    for (x, y, p) in img.enumerate_pixels() {
        bits.set(x, y, p.0[0] > 127);
    }
    Ok(bits)
}

/// Reads a scene's photo and truth.
pub fn read_scene(dir: &Path, name: &str) -> Result<Scene, CoreError> {
    let photo = read_photo(&dir.join(format!("{name}.png")))?;
    let json_path = dir.join(format!("{name}.json"));
    let text = std::fs::read_to_string(&json_path).map_err(|e| io_error(&json_path, e))?;
    Ok(Scene { photo, truth: SceneTruth::from_json(&text)? })
}
