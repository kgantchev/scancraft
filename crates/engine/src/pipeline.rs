//! The recipe → pixels driver (§6.5, §8): decode, detect, flatten, enhance, encode and recognise.
//!
//! Every stage is a call into the crate that owns its algorithm. Where a stage is not finished yet
//! (`NotImplemented`), the engine keeps going with the best thing it has and records a note that
//! becomes a notice, so the app stays usable while the work packages land: no page is lost to a
//! missing step, and the notes say which step to wait for. When a stage is finished the same call
//! picks it up with nothing to change here.

use crate::Services;
use crate::cache::{Cache, PhotoKey};
use crate::error::EngineError;
use crate::jobs::CancellationToken;
use crate::settings::Settings;
use scancraft_core::ALGO_VERSION;
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::{BitPlane, Gray8, MAX_PAGE_PIXELS, Plane, PlaneF32, Rgb8, RgbF32};
use scancraft_core::recipe::{
    CornerOrigin, Detector, FocalInput, ImportOrigin, Look, Page, PageSizeOption, PhotoItem, Proportions, Quality, RecognizedText, SideShape,
    SourceId,
};
use scancraft_core::traits::RecognizeOptions;
use scancraft_core::{Step, determinism::seed_for};
use scancraft_geom::homography::Homography;
use scancraft_geom::proportions::{ProportionsInput, true_proportions};
use scancraft_geom::sheet::unroll;
use scancraft_io::encode::{PageImage, encode};
use scancraft_io::pdf_import::PdfDocument;
use scancraft_io::pdf_write::{DocInfo, PdfPage, write_pdf};
use scancraft_io::text_layer::text_layer;
use scancraft_raster::colour::{luma_plane, to_f32, to_u8, to_u8_plane};
use scancraft_raster::downscale::{area_downscale_rgb, area_downscale_rgb8, fit_long_side};
use scancraft_raster::resample::lanczos3_rgb;
use scancraft_raster::warp::warp_rgb;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The paths of §6.5, as long sides in pixels.
pub const WORK_LONG_SIDE: u32 = 1024;
pub const DISPLAY_LONG_SIDE: u32 = 3072;
pub const PREVIEW_LONG_SIDE: u32 = 1280;
pub const FILTER_THUMB_LONG_SIDE: u32 = 512;
pub const TRAY_THUMB_LONG_SIDE: u32 = 256;
/// Output density by quality (§4.6), and Black & White's own.
pub fn output_dpi(quality: Quality, look: Look) -> u32 {
    match (quality, look) {
        (Quality::Small, _) => 200,
        (Quality::Best, Look::BlackWhite) => 400,
        _ => 300,
    }
}

/// What every job needs: the session's files, the platform services and the settings, all cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    pub cache: Arc<Cache>,
    pub sources: PathBuf,
    pub services: Services,
    pub settings: Settings,
}

/// A note about a step that isn't finished, in the words a notice uses.
pub fn unfinished_note(step: Step) -> String {
    let what = match step {
        Step::A9 | Step::A10 | Step::A5 | Step::A6 | Step::A7 => "Finding the page edges",
        Step::A15 => "Turning pages upright",
        Step::A16 => "Even Light",
        Step::A17 => "The Color Document and Grayscale looks",
        Step::A18 => "The Black & White look",
        Step::A25 => "Tracing curved sides",
        Step::A26 | Step::A27 | Step::A29 => "Flattening curved pages",
        Step::A30 => "Finding a book's spine",
        _ => "A step of the pipeline",
    };
    format!("{what} isn't ready yet ({step}).")
}

// ---- decoding ----

/// A decoded photo, and what its file said about the camera when it was decoded fresh.
pub struct LoadedPhoto {
    pub image: Arc<Rgb8>,
    pub focal: Option<FocalInput>,
}

fn origin_index(origin: &ImportOrigin) -> u32 {
    match origin {
        ImportOrigin::File => 0,
        ImportOrigin::PdfPage { index } | ImportOrigin::TiffPage { index } => *index,
    }
}

pub fn photo_key(source: &SourceId, origin: &ImportOrigin) -> PhotoKey {
    (source.clone(), origin_index(origin))
}

/// A1 and A2 (or A24) on a source file in the session folder; the decoded photo goes in the LRU.
pub fn load_photo(ctx: &Ctx, source: &SourceId, origin: &ImportOrigin, token: &CancellationToken) -> Result<LoadedPhoto, EngineError> {
    let key = photo_key(source, origin);
    if let Some(image) = ctx.cache.photo(&key) {
        return Ok(LoadedPhoto { image, focal: None });
    }
    token.check()?;
    let path = ctx.sources.join(&source.0);
    let bytes = std::fs::read(&path).map_err(|e| EngineError::Fs(format!("the imported file could not be read: {e}")))?;
    token.check()?;
    let (image, focal) = match origin {
        ImportOrigin::PdfPage { index } => (PdfDocument::open(bytes)?.render(*index)?, FocalInput::default()),
        ImportOrigin::File | ImportOrigin::TiffPage { .. } => {
            let mut loaded = scancraft_io::import::load(&bytes)?;
            let index = origin_index(origin) as usize;
            if index >= loaded.len() {
                return Err(EngineError::msg("the file has no such page"));
            }
            let one = loaded.swap_remove(index);
            (one.rgb, one.focal)
        }
    };
    let image = Arc::new(image);
    ctx.cache.put_photo(key, image.clone());
    Ok(LoadedPhoto { image, focal: Some(focal) })
}

// ---- detection ----

/// What detection (or its fallback) found in a photo.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedPhoto {
    pub size: (u32, u32),
    pub focal: FocalInput,
    pub corners: Quad,
    pub origin: CornerOrigin,
    pub sides: [SideShape; 4],
    pub notes: Vec<String>,
}

fn straight() -> [SideShape; 4] {
    [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight]
}

/// Decodes the photo and finds its page (A1–A10). A9 is not finished: until it is, Vision's finding
/// is used when the platform has it, and otherwise the corners start 3% inside the photo (§4.3).
pub fn detect_photo(ctx: &Ctx, item: &PhotoItem, token: &CancellationToken) -> Result<DetectedPhoto, EngineError> {
    use crate::crop::{full_image_corners, not_found_corners};
    let loaded = load_photo(ctx, &item.source, &item.origin, token)?;
    let photo = loaded.image;
    let size = (photo.width(), photo.height());
    let focal = loaded.focal.unwrap_or(item.focal);
    let mut notes = Vec::new();
    if matches!(item.origin, ImportOrigin::PdfPage { .. }) {
        // PDF pages are usually flat already: Use Full Image is their default (§4.3).
        return Ok(DetectedPhoto { size, focal, corners: full_image_corners(size), origin: CornerOrigin::FullImage, sides: straight(), notes });
    }
    token.check()?;
    let seed = seed_for(item.id.0, Step::A9, 0);
    let apple = ctx.services.detector.as_deref();
    match scancraft_detect::detect::detect_page(&photo, apple, seed) {
        Ok(d) => return Ok(DetectedPhoto { size, focal, corners: d.refined.corners, origin: d.origin, sides: d.sides, notes }),
        Err(e) => {
            let e = EngineError::from(e);
            if let Some(step) = e.unfinished_step() {
                notes.push(unfinished_note(step));
            } else {
                notes.push(format!("Page detection failed: {e}"));
            }
        }
    }
    token.check()?;
    // Vision's answer through the service trait, on the work image, scaled back up.
    if let Some(detector) = &ctx.services.detector {
        let (ww, wh) = fit_long_side(size.0, size.1, WORK_LONG_SIDE);
        let work = if (ww, wh) == size { (*photo).clone() } else { area_downscale_rgb8(&photo, ww, wh)? };
        if let Ok(Some(quad)) = detector.detect(&work) {
            let k = f64::from(size.0) / f64::from(ww);
            let scaled = Quad::canonical(quad.0.map(|p| Point::new(p.x * k, p.y * k)));
            if scaled.is_finite() {
                return Ok(DetectedPhoto {
                    size,
                    focal,
                    corners: scaled,
                    origin: CornerOrigin::Detected { by: Detector::Apple, score: 1.0 },
                    sides: straight(),
                    notes,
                });
            }
        }
    }
    Ok(DetectedPhoto { size, focal, corners: not_found_corners(size), origin: CornerOrigin::NotFound, sides: straight(), notes })
}

// ---- the display copy and the loupe ----

/// Screen 2's display copy: the photo with its long side at most 3072 px (A3).
pub fn display_copy(photo: &Rgb8) -> Result<Rgb8, EngineError> {
    let (w, h) = fit_long_side(photo.width(), photo.height(), DISPLAY_LONG_SIDE);
    if (w, h) == (photo.width(), photo.height()) {
        return Ok(photo.clone());
    }
    Ok(area_downscale_rgb8(photo, w, h)?)
}

/// A `side × side` square of `photo` whose top-left pixel is at (`x0`, `y0`) in photo pixels, white
/// where it lies outside the photo (as a page's missing part is, §4.3). `scale` is photo pixels per
/// crop pixel: 1 up to 1024 px, larger when the loupe shows a bigger area (nearest sample).
pub fn crop_square(photo: &Rgb8, x0: f64, y0: f64, side: u32, scale: f64) -> Result<Rgb8, EngineError> {
    let mut data = Vec::with_capacity(side as usize * side as usize);
    for j in 0..side {
        for i in 0..side {
            let (x, y) = ((x0 + (f64::from(i) + 0.5) * scale).floor(), (y0 + (f64::from(j) + 0.5) * scale).floor());
            let inside = x >= 0.0 && y >= 0.0 && x < f64::from(photo.width()) && y < f64::from(photo.height());
            data.push(if inside { photo.get(x as u32, y as u32).unwrap_or([255, 255, 255]) } else { [255, 255, 255] });
        }
    }
    Ok(Plane::from_vec(side, side, data)?)
}

// ---- the page ----

/// The page's size and camera, from A11; a page that already has its proportions keeps them.
#[derive(Debug, Clone, PartialEq)]
pub struct PageGeometry {
    pub proportions: Proportions,
    pub page_size_mm: (f64, f64),
    pub camera: scancraft_core::camera::Camera,
}

pub fn page_geometry(page: &Page, photo_size: (u32, u32), focal: FocalInput, settings: &Settings) -> Result<PageGeometry, EngineError> {
    let out = true_proportions(&ProportionsInput {
        corners: page.corners,
        photo: photo_size,
        focal,
        snap: settings.snap_to_paper,
        default_paper: settings.default_paper,
    })?;
    Ok(PageGeometry {
        proportions: page.proportions.unwrap_or(out.proportions),
        page_size_mm: page.page_size_mm.unwrap_or(out.page_size_mm),
        camera: out.camera,
    })
}

/// The longest side of the quadrilateral in the photo, in pixels.
fn longest_side(q: &Quad) -> f64 {
    (0..4).map(|k| q.0[k].distance(q.0[(k + 1) % 4])).fold(0.0, f64::max)
}

/// The final page's size in pixels, in its turned orientation: what the look is rendered at.
pub fn final_size(geometry: &PageGeometry, quarter_turns: u8, dpi: u32, look: Look) -> (u32, u32) {
    let (mw, mh) = geometry.page_size_mm;
    let target = ((mw / 25.4 * f64::from(dpi)).round().max(1.0), (mh / 25.4 * f64::from(dpi)).round().max(1.0));
    let native = (f64::from(geometry.proportions.native_size.0).max(1.0), f64::from(geometry.proportions.native_size.1).max(1.0));
    // The warp renders at the native size when the output density is at or below it (A13); a page
    // with fewer pixels than the output asks for stays as it is, except Black & White, which is
    // always rendered at its own density (A18).
    let (w, h) = if look == Look::BlackWhite || (native.0 >= target.0 && native.1 >= target.1) {
        target
    } else {
        (native.0.min(target.0), native.1.min(target.1))
    };
    let (mut w, mut h) = (w as u64, h as u64);
    while w * h > MAX_PAGE_PIXELS {
        w = (w as f64 * 0.95) as u64;
        h = (h as f64 * 0.95) as u64;
    }
    if quarter_turns % 2 == 1 { (h.max(1) as u32, w.max(1) as u32) } else { (w.max(1) as u32, h.max(1) as u32) }
}

/// The part of the photo the page needs, with the corners moved to match: the quad's bounding box
/// plus a margin, so a 48-megapixel photo is not converted whole to floats.
fn crop_to_page(photo: &Rgb8, corners: &Quad, margin: f64) -> Result<(Rgb8, Quad, (f64, f64)), EngineError> {
    let xs = corners.0.iter().map(|p| p.x);
    let ys = corners.0.iter().map(|p| p.y);
    let (min_x, max_x) = (xs.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max));
    let (min_y, max_y) = (ys.clone().fold(f64::INFINITY, f64::min), ys.fold(f64::NEG_INFINITY, f64::max));
    let x0 = (min_x - margin).floor().clamp(0.0, f64::from(photo.width()) - 1.0) as u32;
    let y0 = (min_y - margin).floor().clamp(0.0, f64::from(photo.height()) - 1.0) as u32;
    let x1 = ((max_x + margin).ceil().clamp(1.0, f64::from(photo.width())) as u32).max(x0 + 1);
    let y1 = ((max_y + margin).ceil().clamp(1.0, f64::from(photo.height())) as u32).max(y0 + 1);
    let (w, h) = (x1 - x0, y1 - y0);
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for y in y0..y1 {
        let row = photo.row(y).ok_or_else(|| EngineError::msg("the photo has no such row"))?;
        data.extend_from_slice(row.get(x0 as usize..x1 as usize).ok_or_else(|| EngineError::msg("the photo has no such column"))?);
    }
    let moved = Quad(corners.0.map(|p| Point::new(p.x - f64::from(x0), p.y - f64::from(y0))));
    Ok((Plane::from_vec(w, h, data)?, moved, (f64::from(x0), f64::from(y0))))
}

/// Warps the page out of the photo at `out` pixels (A12, or A28 for a flattened page, then A13); with
/// `supersample` the warp is made twice as large and reduced by area, for a clean small page.
pub fn warp_page(
    photo: &Rgb8,
    page: &Page,
    geometry: &PageGeometry,
    out: (u32, u32),
    supersample: bool,
    token: &CancellationToken,
) -> Result<RgbF32, EngineError> {
    let sheet = page.sheet.as_ref().filter(|fit| fit.decision.flattened);
    let margin = if sheet.is_some() { 0.05 * longest_side(&page.corners) + 8.0 } else { 6.0 };
    let (mut region, mut corners, origin) = crop_to_page(photo, &page.corners, margin)?;
    let mut shrink = 1.0;
    // A page of the photo much larger than the output is first reduced by area (A3): the warp samples
    // by bicubic interpolation, which alone aliases when it reduces by much.
    let wanted = f64::from(out.0.max(out.1)) * if supersample { 2.0 } else { 1.0 };
    let have = longest_side(&corners).max(1.0);
    if have > 1.5 * wanted {
        shrink = wanted * 1.2 / have;
        let (w, h) = (((f64::from(region.width()) * shrink).round() as u32).max(1), ((f64::from(region.height()) * shrink).round() as u32).max(1));
        let (sx, sy) = (f64::from(w) / f64::from(region.width()), f64::from(h) / f64::from(region.height()));
        region = area_downscale_rgb8(&region, w, h)?;
        corners = Quad(corners.0.map(|p| Point::new(p.x * sx, p.y * sy)));
    }
    token.check()?;
    let source = to_f32(&region)?;
    let long = f64::from(region.width().max(region.height()));
    let (ow, oh) = if supersample { (out.0 * 2, out.1 * 2) } else { out };
    let warped = if let Some(fit) = sheet {
        // A flattened page: the unrolled sheet, in a camera scaled like the region the page was cut from.
        let mut camera = geometry.camera;
        camera.f = fit.sheet.focal;
        camera.width = f64::from(photo.width());
        camera.height = f64::from(photo.height());
        let map = unroll(&fit.sheet, &camera)?.turned(page.quarter_turns);
        // The sheet maps into photo pixels; move and scale them into the cropped, reduced region's.
        warp_rgb(&source, &ShiftedMap { inner: map, dx: -origin.0, dy: -origin.1, k: shrink }, ow, oh)?
    } else {
        let map = Homography::from_unit_square_turned(&corners, long, page.quarter_turns)?;
        warp_rgb(&source, &map, ow, oh)?
    };
    if supersample { Ok(area_downscale_rgb(&warped, out.0, out.1)?) } else { Ok(warped) }
}

/// Moves and scales a mapping's photo positions into a cropped, reduced copy of the photo.
struct ShiftedMap<M: scancraft_core::map::PageMap> {
    inner: M,
    dx: f64,
    dy: f64,
    k: f64,
}

impl<M: scancraft_core::map::PageMap> scancraft_core::map::PageMap for ShiftedMap<M> {
    fn map(&self, u: f64, v: f64) -> Option<Point> {
        self.inner.map(u, v).map(|p| Point::new((p.x + self.dx) * self.k, (p.y + self.dy) * self.k))
    }
}

/// A finished page image in the form its look gives it.
#[derive(Debug, Clone)]
pub enum PageOutput {
    Colour(RgbF32),
    Grey(PlaneF32),
    Bits(BitPlane),
}

impl PageOutput {
    pub fn size(&self) -> (u32, u32) {
        match self {
            PageOutput::Colour(p) => (p.width(), p.height()),
            PageOutput::Grey(p) => (p.width(), p.height()),
            PageOutput::Bits(p) => (p.width(), p.height()),
        }
    }

    /// For the screen: grey and bits as RGB (ink is black on white).
    pub fn to_rgb8(&self) -> Result<Rgb8, EngineError> {
        Ok(match self {
            PageOutput::Colour(p) => to_u8(p)?,
            PageOutput::Grey(p) => {
                let g = to_u8_plane(p)?;
                Plane::from_vec(g.width(), g.height(), g.data().iter().map(|&v| [v, v, v]).collect())?
            }
            PageOutput::Bits(b) => {
                let mut data = Vec::with_capacity(b.width() as usize * b.height() as usize);
                for y in 0..b.height() {
                    for x in 0..b.width() {
                        data.push(if b.get(x, y) == Some(true) { [0, 0, 0] } else { [255, 255, 255] });
                    }
                }
                Plane::from_vec(b.width(), b.height(), data)?
            }
        })
    }

    /// The 8-bit grey page text recognition reads.
    pub fn to_gray8(&self) -> Result<Gray8, EngineError> {
        Ok(match self {
            PageOutput::Colour(p) => to_u8_plane(&luma_plane(p)?)?,
            PageOutput::Grey(p) => to_u8_plane(p)?,
            PageOutput::Bits(_) => {
                let rgb = self.to_rgb8()?;
                Plane::from_vec(rgb.width(), rgb.height(), rgb.data().iter().map(|p| p[0]).collect())?
            }
        })
    }

    pub fn into_page_image(self) -> Result<PageImage, EngineError> {
        Ok(match self {
            PageOutput::Colour(p) => PageImage::Colour(to_u8(&p)?),
            PageOutput::Grey(p) => PageImage::Grey(to_u8_plane(&p)?),
            PageOutput::Bits(b) => PageImage::Bits(b),
        })
    }
}

/// Applies the page's look (A16–A18) to a warped page. A step that isn't finished leaves the page as
/// it is, or, for Black & White, takes A18's rule without its photo and speck steps, and says so.
pub fn apply_look(page_rgb: RgbF32, page: &Page, dpi: f64, settings: &Settings, notes: &mut Vec<String>) -> Result<PageOutput, EngineError> {
    use scancraft_enhance::{even_light::even_light, looks};
    if page.look == Look::Original {
        return Ok(PageOutput::Colour(page_rgb));
    }
    let light = match even_light(&page_rgb, dpi, settings.keep_paper_colour) {
        Ok(light) => Some(light),
        Err(e) => {
            let e = EngineError::from(e);
            let Some(step) = e.unfinished_step() else { return Err(e) };
            notes.push(unfinished_note(step));
            None
        }
    };
    let fallback_colour = |rgb: RgbF32| PageOutput::Colour(rgb);
    let finished =
        |result: Result<PageOutput, scancraft_enhance::EnhanceError>, notes: &mut Vec<String>| -> Result<Option<PageOutput>, EngineError> {
            match result {
                Ok(out) => Ok(Some(out)),
                Err(e) => {
                    let e = EngineError::from(e);
                    let step = e.unfinished_step().ok_or(e)?;
                    notes.push(unfinished_note(step));
                    Ok(None)
                }
            }
        };
    let enhanced = match (&light, page.look) {
        (Some(l), Look::ColorDocument) => finished(looks::color_document(l, page.strength, dpi).map(PageOutput::Colour), notes)?,
        (Some(l), Look::Grayscale) => finished(looks::grayscale(l, page.strength, dpi).map(PageOutput::Grey), notes)?,
        (Some(l), Look::BlackWhite) => finished(looks::black_white(l, page.strength, dpi).map(PageOutput::Bits), notes)?,
        _ => None,
    };
    if let Some(out) = enhanced {
        return Ok(out);
    }
    // The fallbacks: the warped page, its luma, or A18's midpoint rule on its luma.
    Ok(match page.look {
        Look::Grayscale => PageOutput::Grey(luma_plane(&page_rgb)?),
        Look::BlackWhite => PageOutput::Bits(scancraft_raster::ink::midpoint_ink(&luma_plane(&page_rgb)?, dpi.max(50.0), page.strength.min(100))?),
        _ => fallback_colour(page_rgb),
    })
}

/// What a render asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSpec {
    /// The output size in pixels, in the page's turned orientation.
    pub size: (u32, u32),
    pub dpi: f64,
    /// A small image made from a reduced copy, with a supersampled warp.
    pub preview: bool,
}

/// Renders a page: the warp, the resample to the output size, and the look.
pub fn render_page(
    ctx: &Ctx,
    photo: &Rgb8,
    item: &PhotoItem,
    page: &Page,
    spec: RenderSpec,
    token: &CancellationToken,
) -> Result<(PageOutput, Vec<String>), EngineError> {
    let geometry = page_geometry(page, (photo.width(), photo.height()), item.focal, &ctx.settings)?;
    let mut notes = Vec::new();
    // The warp's natural size, then A14 down to the output when it is larger by more than 2%.
    let natural = if spec.preview { spec.size } else { warp_size(&geometry, page.quarter_turns, spec.size) };
    let warped = warp_page(photo, page, &geometry, natural, spec.preview, token)?;
    token.check()?;
    let resized = if natural != spec.size && f64::from(natural.0) > 1.02 * f64::from(spec.size.0) {
        lanczos3_rgb(&warped, spec.size.0, spec.size.1)?
    } else {
        warped
    };
    token.check()?;
    let out = apply_look(resized, page, spec.dpi, &ctx.settings, &mut notes)?;
    Ok((out, notes))
}

/// The size A13 warps at for an output size: the page's native pixels when there are more of them.
fn warp_size(geometry: &PageGeometry, quarter_turns: u8, out: (u32, u32)) -> (u32, u32) {
    let native = geometry.proportions.native_size;
    let native = if quarter_turns % 2 == 1 { (native.1, native.0) } else { native };
    let area = u64::from(native.0.max(1)) * u64::from(native.1.max(1));
    if native.0 > out.0 && native.1 > out.1 && area <= MAX_PAGE_PIXELS { native } else { out }
}

/// A look preview or thumbnail of `long_side` pixels, from the decoded photo.
pub fn render_small(
    ctx: &Ctx,
    photo: &Rgb8,
    item: &PhotoItem,
    page: &Page,
    long_side: u32,
    token: &CancellationToken,
) -> Result<(Rgb8, Vec<String>), EngineError> {
    let geometry = page_geometry(page, (photo.width(), photo.height()), item.focal, &ctx.settings)?;
    let (full_w, full_h) = final_size(&geometry, page.quarter_turns, 300, page.look);
    let (w, h) = fit_long_side(full_w, full_h, long_side);
    let dpi = f64::from(w.max(h)) / (geometry.page_size_mm.0.max(geometry.page_size_mm.1) / 25.4);
    let (out, notes) = render_page(ctx, photo, item, page, RenderSpec { size: (w, h), dpi: dpi.max(30.0), preview: true }, token)?;
    Ok((out.to_rgb8()?, notes))
}

// ---- recognition ----

/// A20 on the finished page at 300 DPI; `None` when the platform has no recogniser.
pub fn recognize_page(
    ctx: &Ctx,
    photo: &Rgb8,
    item: &PhotoItem,
    page: &Page,
    languages: &[String],
    token: &CancellationToken,
) -> Result<Option<RecognizedText>, EngineError> {
    let Some(recognizer) = ctx.services.recognizer.clone() else { return Ok(None) };
    let geometry = page_geometry(page, (photo.width(), photo.height()), item.focal, &ctx.settings)?;
    let size = final_size(&geometry, page.quarter_turns, 300, Look::Grayscale);
    let mut grey_page = page.clone();
    grey_page.look = Look::Grayscale;
    let (out, _) = render_page(ctx, photo, item, &grey_page, RenderSpec { size, dpi: 300.0, preview: false }, token)?;
    token.check()?;
    let gray = out.to_gray8()?;
    let options = RecognizeOptions {
        languages: languages.to_vec(),
        language_correction: ctx.settings.language_correction,
        minimum_text_height: (12.0 / gray.height().max(1) as f32).max(0.0025),
    };
    recognizer.recognize(&gray, &options).map(Some).map_err(|e| EngineError::msg(format!("text recognition failed: {e}")))
}

// ---- export ----

/// The fixed page sizes of Export Options, in points, in portrait.
fn fixed_size_pt(option: PageSizeOption) -> Option<(f64, f64)> {
    match option {
        PageSizeOption::Auto => None,
        PageSizeOption::A4 => Some((595.28, 841.89)),
        PageSizeOption::UsLetter => Some((612.0, 792.0)),
        PageSizeOption::UsLegal => Some((612.0, 1008.0)),
        PageSizeOption::A5 => Some((419.53, 595.28)),
    }
}

/// The page's MediaBox and where its image goes: the image fitted inside a fixed size and centred,
/// never stretched; with Auto the image fills the page.
pub fn page_layout(image_size_pt: (f64, f64), option: PageSizeOption) -> ((f64, f64), [f64; 4]) {
    let Some((pw, ph)) = fixed_size_pt(option) else { return (image_size_pt, [0.0, 0.0, image_size_pt.0, image_size_pt.1]) };
    // The fixed size takes the image's orientation.
    let (pw, ph) = if image_size_pt.0 > image_size_pt.1 { (pw.max(ph), pw.min(ph)) } else { (pw.min(ph), pw.max(ph)) };
    let k = (pw / image_size_pt.0).min(ph / image_size_pt.1);
    let (w, h) = (image_size_pt.0 * k, image_size_pt.1 * k);
    ((pw, ph), [(pw - w) / 2.0, (ph - h) / 2.0, w, h])
}

/// What an export job reports.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    pub pages: usize,
    /// The 1-based tray numbers of pages with no text layer, though searchable text was asked for.
    pub not_searchable: Vec<u32>,
    pub notes: Vec<String>,
}

/// Renders, encodes and writes the PDF (A22, A21, A23). `pages` are in tray order.
pub fn export_pdf(
    ctx: &Ctx,
    pages: &[(PhotoItem, Page)],
    options: &scancraft_core::recipe::ExportOptions,
    title: &str,
    dest: &Path,
    token: &CancellationToken,
) -> Result<ExportReport, EngineError> {
    let mut out = Vec::with_capacity(pages.len());
    let mut report = ExportReport { pages: pages.len(), not_searchable: Vec::new(), notes: Vec::new() };
    for (number, (item, page)) in pages.iter().enumerate() {
        token.check()?;
        let loaded = load_photo(ctx, &item.source, &item.origin, token)?;
        let photo = loaded.image;
        let geometry = page_geometry(page, (photo.width(), photo.height()), item.focal, &ctx.settings)?;
        let dpi = output_dpi(options.quality, page.look);
        let size = final_size(&geometry, page.quarter_turns, dpi, page.look);
        let (rendered, notes) = render_page(ctx, &photo, item, page, RenderSpec { size, dpi: f64::from(dpi), preview: false }, token)?;
        for n in notes {
            if !report.notes.contains(&n) {
                report.notes.push(n);
            }
        }
        let (width, height) = rendered.size();
        let image_pt = (f64::from(width) / f64::from(dpi) * 72.0, f64::from(height) / f64::from(dpi) * 72.0);
        let encoded = encode(&rendered.into_page_image()?, options.quality)?;
        let ((pw, ph), rect) = page_layout(image_pt, options.page_size);
        let text = match (&page.text, options.searchable) {
            (Some(t), true) => Some(text_layer(&t.lines, (width, height), (rect[2], rect[3]))?),
            (None, true) => {
                report.not_searchable.push(number as u32 + 1);
                None
            }
            _ => None,
        };
        out.push(PdfPage { image: encoded, size_pt: (pw, ph), image_rect_pt: rect, text });
    }
    token.check()?;
    let producer = format!("ScanCraft {} (algorithms {ALGO_VERSION})", env!("CARGO_PKG_VERSION"));
    write_pdf(&out, &DocInfo { title: title.to_string(), producer }, dest)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::recipe::FlattenMode;

    #[test]
    fn the_output_density_follows_the_quality_and_look() {
        assert_eq!(output_dpi(Quality::Small, Look::ColorDocument), 200);
        assert_eq!(output_dpi(Quality::Balanced, Look::ColorDocument), 300);
        assert_eq!(output_dpi(Quality::Best, Look::ColorDocument), 300);
        assert_eq!(output_dpi(Quality::Best, Look::BlackWhite), 400);
        assert_eq!(output_dpi(Quality::Balanced, Look::BlackWhite), 300);
    }

    #[test]
    fn a_fixed_page_size_fits_the_image_centred_without_stretching() {
        // Auto: the image fills the page.
        let (page, rect) = page_layout((300.0, 400.0), PageSizeOption::Auto);
        assert_eq!((page, rect), ((300.0, 400.0), [0.0, 0.0, 300.0, 400.0]));
        // A wide image on A4: fitted to the width, centred, the aspect ratio kept.
        let (page, rect) = page_layout((400.0, 200.0), PageSizeOption::A4);
        assert_eq!(page, (841.89, 595.28));
        assert!((rect[2] / rect[3] - 2.0).abs() < 1e-9);
        assert!((rect[0] * 2.0 + rect[2] - page.0).abs() < 1e-9 && (rect[1] * 2.0 + rect[3] - page.1).abs() < 1e-9);
        assert!(rect[2] <= page.0 + 1e-9 && rect[3] <= page.1 + 1e-9);
        // A tall image on Letter takes the portrait orientation.
        let (page, _) = page_layout((100.0, 300.0), PageSizeOption::UsLetter);
        assert_eq!(page, (612.0, 792.0));
    }

    /// A drawn scene: a bright sheet on a dark desk, seen at an angle.
    fn scene() -> (Rgb8, PhotoItem, Page) {
        let w = 400;
        let h = 300;
        let mut data: Vec<[u8; 3]> = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let on_sheet = (70..330).contains(&x) && (50..250).contains(&y);
                let p: [u8; 3] = if on_sheet {
                    let text = (x / 17 + y / 23) % 5 == 0;
                    if text { [40, 40, 40] } else { [238, 236, 230] }
                } else {
                    [32, 30, 28]
                };
                data.push(p);
            }
        }
        let photo = Plane::from_vec(w, h, data).unwrap();
        let corners = Quad([Point::new(70.0, 50.0), Point::new(330.0, 50.0), Point::new(330.0, 250.0), Point::new(70.0, 250.0)]);
        let item = PhotoItem {
            id: scancraft_core::recipe::PhotoId(1),
            source: SourceId("test".to_string()),
            name: "scene".to_string(),
            origin: ImportOrigin::File,
            size: (w, h),
            focal: FocalInput::default(),
        };
        let page = Page {
            id: scancraft_core::recipe::PageId(1),
            photo: item.id,
            source: item.source.clone(),
            corners,
            corners_from: CornerOrigin::Manual,
            sides: [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight],
            spread: None,
            flatten: FlattenMode::Off,
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: scancraft_core::recipe::TurnOrigin::Manual,
            look: Look::Original,
            strength: 0,
            text: None,
        };
        (photo, item, page)
    }

    #[test]
    fn the_look_preview_matches_the_final_page_within_six_grey_levels() {
        let dir = std::env::temp_dir().join(format!("scancraft-preview-{}-{}", std::process::id(), std::thread::current().name().unwrap_or("test")));
        let cache = Arc::new(crate::cache::Cache::unchecked(dir.clone()));
        let ctx = Ctx { cache, sources: dir.clone(), services: crate::Services::default(), settings: crate::settings::Settings::default() };
        let (photo, item, page) = scene();
        let token = CancellationToken::new();
        let (preview, _) = render_small(&ctx, &photo, &item, &page, PREVIEW_LONG_SIDE, &token).unwrap();
        let geometry = page_geometry(&page, (photo.width(), photo.height()), item.focal, &ctx.settings).unwrap();
        let size = final_size(&geometry, page.quarter_turns, 300, page.look);
        let (final_page, _) = render_page(&ctx, &photo, &item, &page, RenderSpec { size, dpi: 300.0, preview: false }, &token).unwrap();
        let full = final_page.to_rgb8().unwrap();
        let same = fit_long_side(full.width(), full.height(), preview.width().max(preview.height()));
        let small = area_downscale_rgb8(&full, same.0, same.1).unwrap();
        // The mean difference over the page, in grey levels (§6.5).
        let mut total = 0f64;
        let mut count = 0f64;
        for (a, b) in preview.data().iter().zip(small.data().iter()) {
            let (ga, gb) = (f64::from(a[0]) + f64::from(a[1]) + f64::from(a[2]), f64::from(b[0]) + f64::from(b[1]) + f64::from(b[2]));
            total += (ga - gb).abs() / 3.0;
            count += 1.0;
        }
        let mean = total / count;
        assert!(mean < 6.0, "the preview differs from the final page by {mean} grey levels");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_square_of_the_photo_is_white_outside_it() {
        let photo: Rgb8 = Plane::new(10, 10, [10, 20, 30]).unwrap();
        let crop = crop_square(&photo, -3.0, -3.0, 6, 1.0).unwrap();
        assert_eq!(crop.get(0, 0), Some([255, 255, 255]));
        assert_eq!(crop.get(4, 4), Some([10, 20, 30]));
        // At a larger scale one crop pixel covers more of the photo.
        let big = crop_square(&photo, 0.0, 0.0, 5, 2.0).unwrap();
        assert!(big.data().iter().all(|p| *p == [10, 20, 30]));
        // Nothing here panics for junk.
        assert!(crop_square(&photo, f64::NAN, f64::INFINITY, 3, 1.0).is_ok());
    }
}
