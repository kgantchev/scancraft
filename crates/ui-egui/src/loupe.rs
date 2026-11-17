//! The magnifier loupe of Screen 2 (§4.3).
//!
//! It opens while a corner, a side's pill, a bend handle or a spine end is pressed, and shuts the
//! moment the button is released; a keyboard nudge, which has no button, shows it for a second. It
//! is a 180 pt circle that shows the photo at three times the canvas zoom, centred on the handle
//! (not on the pointer, which may be a few points off it), and set on the far side of the handle
//! from the page, so it never covers the sides that meet there.
//!
//! The picture comes at once from the display copy that is already on the GPU, and is drawn over
//! with the engine's full-resolution crop as soon as that has been cut. Wherever neither holds
//! the photo, the lens is dark, not the white of paper.

use std::time::Duration;

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use scancraft_core::engine_api::{CropView, EngineApi, ImageKey, LoupeCrop, OutlineView};
use scancraft_core::geom::{Point, Side};
use scancraft_core::recipe::SideShape;

use crate::canvas::{self, Canvas};
use crate::state::UiState;
use crate::theme;

/// The loupe's diameter (§4.3).
pub const DIAMETER: f32 = 180.0;
/// The loupe's radius.
pub const RADIUS: f32 = DIAMETER / 2.0;
/// How far the lens's centre sits from the handle it magnifies: a hand's breadth from the
/// handle's ring to the lens's edge (§4.3).
pub const DISTANCE: f32 = 100.0;
/// How close to the window's edge the lens may come.
pub const WINDOW_MARGIN: f32 = 8.0;
/// The magnification over the canvas zoom (§4.3).
pub const MAGNIFICATION: f64 = 3.0;
/// The largest crop the engine is asked for, in photo pixels (§6.5).
pub const MAX_CROP: f64 = 1024.0;
/// How much more than the lens shows the crop is asked to hold. The engine cuts a crop 1.22 times
/// wider than that again and keeps it while the handle stays within a quarter of its width of the
/// crop's centre, so the lens stays inside the crop for the whole of that range.
pub const CROP_MARGIN: f64 = 1.7;
/// The gap in the middle of the crosshair, in points (§4.3).
pub const CROSSHAIR_GAP: f32 = 8.0;
/// The length of each of the crosshair's four ticks, in points.
pub const CROSSHAIR_TICK: f32 = 14.0;
/// The width of the page outline inside the lens, in points.
pub const LINE_WIDTH: f32 = 1.0;
/// The width of the dark halo under it.
pub const LINE_HALO: f32 = 3.0;
/// The number of segments the loupe's circle is drawn with (§6.5).
pub const SEGMENTS: usize = 64;

/// Where the lens is, what it looks at and how much it magnifies.
#[derive(Debug, Clone, Copy)]
pub struct Lens {
    /// The circle's centre, in window points.
    pub centre: Pos2,
    /// The photo position at the centre of the lens: the handle's.
    pub focus: Point,
    /// Points per full-resolution photo pixel inside the lens: [`MAGNIFICATION`] times the canvas's.
    pub points_per_photo_px: f64,
}

impl Lens {
    /// A lens at `centre` looking at `focus`, over a canvas that shows `canvas_points_per_photo_px`
    /// points to a photo pixel.
    pub fn new(centre: Pos2, focus: Point, canvas_points_per_photo_px: f64) -> Lens {
        Lens { centre, focus, points_per_photo_px: canvas_points_per_photo_px * MAGNIFICATION }
    }

    /// A photo position in the lens's points.
    pub fn point(&self, photo: Point) -> Pos2 {
        self.centre
            + Vec2::new(((photo.x - self.focus.x) * self.points_per_photo_px) as f32, ((photo.y - self.focus.y) * self.points_per_photo_px) as f32)
    }

    /// A position in the lens's points as a photo position.
    pub fn photo(&self, p: Pos2) -> Point {
        Point::new(
            self.focus.x + f64::from(p.x - self.centre.x) / self.points_per_photo_px,
            self.focus.y + f64::from(p.y - self.centre.y) / self.points_per_photo_px,
        )
    }
}

/// The side of the crop the engine is asked for, in photo pixels: what the lens shows, with
/// [`CROP_MARGIN`] to spare, and at most [`MAX_CROP`] (§6.5). `canvas_points_per_photo_px` is the
/// canvas's zoom, not the lens's.
pub fn crop_side(canvas_points_per_photo_px: f64) -> f64 {
    if !canvas_points_per_photo_px.is_finite() || canvas_points_per_photo_px <= 0.0 {
        return MAX_CROP;
    }
    let across = f64::from(DIAMETER) / (canvas_points_per_photo_px * MAGNIFICATION);
    (CROP_MARGIN * across).min(MAX_CROP)
}

/// The centre of the lens for a handle at `handle`: [`DISTANCE`] away from it in the direction
/// `away` (the handle's offset from the page's centre), inside `window`.
///
/// Where that would leave the window, the lens slides round the handle to the nearest angle that
/// fits, instead of jumping to its other side: it stays the same distance from the handle, so it
/// still covers neither of the sides that meet there.
pub fn lens_centre(handle: Pos2, away: Vec2, window: Rect) -> Pos2 {
    let direction = if away.length() > 1e-3 { away.normalized() } else { Vec2::new(0.0, -1.0) };
    let room = window.shrink(WINDOW_MARGIN + RADIUS);
    let ideal = handle + direction * DISTANCE;
    if room.width() < 0.0 || room.height() < 0.0 {
        // A window smaller than the lens: the middle of it is as good as any place.
        return window.center();
    }
    let rotate = |v: Vec2, angle: f32| Vec2::new(v.x * angle.cos() - v.y * angle.sin(), v.x * angle.sin() + v.y * angle.cos());
    for step in 0..=18 {
        let angle = (step as f32 * 10.0).to_radians();
        let mut fits: Vec<Pos2> = Vec::new();
        for sign in [1.0f32, -1.0] {
            if step == 0 && sign < 0.0 {
                continue;
            }
            let candidate = handle + rotate(direction, sign * angle) * DISTANCE;
            if room.contains(candidate) {
                fits.push(candidate);
            }
        }
        // Of two that fit, the one nearer the middle of the window.
        let nearest = fits.into_iter().min_by(|a, b| a.distance(window.center()).total_cmp(&b.distance(window.center())));
        if let Some(centre) = nearest {
            return centre;
        }
    }
    Pos2::new(ideal.x.clamp(room.min.x, room.max.x), ideal.y.clamp(room.min.y, room.max.y))
}

/// The part of the photo a texture holds: its top-left pixel and its size, in photo pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coverage {
    /// The photo position of the texture's top-left corner.
    pub origin: Point,
    /// The texture's width and height, in photo pixels.
    pub size: (f64, f64),
}

impl Coverage {
    /// A texture of the whole photo, as the display copy is.
    pub fn photo(size: (u32, u32)) -> Coverage {
        Coverage { origin: Point::new(0.0, 0.0), size: (f64::from(size.0), f64::from(size.1)) }
    }

    /// The part of the photo the engine's crop holds.
    pub fn of_crop(crop: &LoupeCrop) -> Coverage {
        Coverage { origin: crop.origin, size: (f64::from(crop.image.width()) * crop.scale, f64::from(crop.image.height()) * crop.scale) }
    }
}

/// The lens's circle, cut to the part of the photo a texture holds: the vertices of a convex
/// polygon, each with its position in the window and its 0–1 texture coordinate.
///
/// A texture is only ever sampled where it has picture: outside `coverage`, and outside the
/// photo itself, the polygon stops, and what is under it shows. Sampling past the edge instead
/// smears the edge pixels outward.
pub fn layer_polygon(lens: &Lens, coverage: Coverage, photo: (u32, u32)) -> Vec<(Pos2, Pos2)> {
    let scale = lens.points_per_photo_px;
    if !scale.is_finite() || scale <= 0.0 || coverage.size.0.is_nan() || coverage.size.0 <= 0.0 || coverage.size.1.is_nan() || coverage.size.1 <= 0.0
    {
        return Vec::new();
    }
    let min = Point::new(coverage.origin.x.max(0.0), coverage.origin.y.max(0.0));
    let max =
        Point::new((coverage.origin.x + coverage.size.0).min(f64::from(photo.0)), (coverage.origin.y + coverage.size.1).min(f64::from(photo.1)));
    if max.x.is_nan() || max.y.is_nan() || max.x <= min.x || max.y <= min.y {
        return Vec::new();
    }
    let rect = Rect::from_min_max(lens.point(min), lens.point(max));
    let circle: Vec<Pos2> = (0..SEGMENTS)
        .map(|i| {
            let angle = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
            lens.centre + Vec2::new(angle.cos(), angle.sin()) * RADIUS
        })
        .collect();
    let mut polygon = circle;
    for (axis, bound, keep_greater) in [(0, rect.min.x, true), (0, rect.max.x, false), (1, rect.min.y, true), (1, rect.max.y, false)] {
        polygon = clip_to_half_plane(&polygon, axis, bound, keep_greater);
    }
    polygon
        .into_iter()
        .map(|p| {
            let photo = lens.photo(p);
            let uv = Pos2::new(((photo.x - coverage.origin.x) / coverage.size.0) as f32, ((photo.y - coverage.origin.y) / coverage.size.1) as f32);
            (p, uv)
        })
        .collect()
}

/// A convex polygon cut by the line `x = bound` (axis 0) or `y = bound` (axis 1), keeping the
/// side where the coordinate is larger (`keep_greater`) or smaller.
fn clip_to_half_plane(polygon: &[Pos2], axis: usize, bound: f32, keep_greater: bool) -> Vec<Pos2> {
    let value = |p: Pos2| if axis == 0 { p.x } else { p.y };
    let inside = |p: Pos2| if keep_greater { value(p) >= bound } else { value(p) <= bound };
    let cut = |a: Pos2, b: Pos2| {
        let span = value(b) - value(a);
        let t = if span.abs() > f32::EPSILON { (bound - value(a)) / span } else { 0.0 };
        a + (b - a) * t
    };
    let mut out = Vec::with_capacity(polygon.len() + 4);
    for (i, current) in polygon.iter().enumerate() {
        let previous = polygon[(i + polygon.len() - 1) % polygon.len()];
        if inside(*current) {
            if !inside(previous) {
                out.push(cut(previous, *current));
            }
            out.push(*current);
        } else if inside(previous) {
            out.push(cut(previous, *current));
        }
    }
    out
}

/// The pieces of a polyline that lie in the ring between two circles around `centre`.
///
/// The lens draws the page outline with this: the part outside the lens's rim is cut off, and so
/// is the part inside the crosshair's gap, which leaves the spot the user is trying to see clear.
pub fn pieces_in_ring(points: &[Pos2], centre: Pos2, inner: f32, outer: f32) -> Vec<Vec<Pos2>> {
    let mut pieces: Vec<Vec<Pos2>> = Vec::new();
    let mut current: Option<Vec<Pos2>> = None;
    for pair in points.windows(2) {
        let (p0, p1) = (pair[0], pair[1]);
        let origin = (f64::from(p0.x - centre.x), f64::from(p0.y - centre.y));
        let direction = (f64::from(p1.x - p0.x), f64::from(p1.y - p0.y));
        let a = direction.0 * direction.0 + direction.1 * direction.1;
        let mut cuts = vec![0.0f64, 1.0];
        if a > 1e-12 {
            let b = 2.0 * (direction.0 * origin.0 + direction.1 * origin.1);
            for radius in [f64::from(inner), f64::from(outer)] {
                let c = origin.0 * origin.0 + origin.1 * origin.1 - radius * radius;
                let discriminant = b * b - 4.0 * a * c;
                if discriminant >= 0.0 {
                    let root = discriminant.sqrt();
                    for t in [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)] {
                        if t > 0.0 && t < 1.0 {
                            cuts.push(t);
                        }
                    }
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        for span in cuts.windows(2) {
            let (t0, t1) = (span[0], span[1]);
            if t1 - t0 < 1e-9 {
                continue;
            }
            let at = |t: f64| Pos2::new((f64::from(p0.x) + direction.0 * t) as f32, (f64::from(p0.y) + direction.1 * t) as f32);
            let (from, to) = (at(t0), at(t1));
            let middle = at((t0 + t1) / 2.0);
            let distance = middle.distance(centre);
            if distance >= inner - 1e-3 && distance <= outer + 1e-3 {
                match current.as_mut() {
                    Some(piece) => piece.push(to),
                    None => current = Some(vec![from, to]),
                }
            } else if let Some(piece) = current.take() {
                pieces.push(piece);
            }
        }
    }
    if let Some(piece) = current {
        pieces.push(piece);
    }
    pieces
}

/// Draws the loupe, when it is showing.
pub fn draw(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) {
    let now = ui.input(|i| i.time);
    // A nudge's second ends with nothing to say so: ask for the frame that removes the lens.
    if let Some(until) = state.loupe.until
        && now < until
    {
        ui.ctx().request_repaint_after(Duration::from_secs_f64(until - now));
    }
    if !state.loupe_visible(now) {
        return;
    }
    let Some(focus) = state.loupe.focus else {
        return;
    };
    let canvas_scale = canvas.points_per_photo_px();
    if !canvas_scale.is_finite() || canvas_scale <= 0.0 {
        return;
    }
    let handle = canvas.photo_to_canvas(focus);
    let window = ui.ctx().viewport_rect();
    let centre = lens_centre(handle, away_from_page(crop, canvas, focus, handle), window);
    let lens = Lens::new(centre, focus, canvas_scale);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.make_persistent_id("scancraft.loupe")));
    draw_shadow(&painter, &lens);
    painter.circle_filled(lens.centre, RADIUS, theme::CANVAS_BACKGROUND);
    // The display copy is already on the GPU: the lens has its picture on the first frame. The
    // engine's crop, once it is cut from the full-resolution photo, goes over it.
    if let Some(display) = state.texture(ui, engine, ImageKey::DisplayCopy(crop.photo), egui::TextureOptions::LINEAR) {
        draw_layer(&painter, display.id(), layer_polygon(&lens, Coverage::photo(crop.photo_size), crop.photo_size));
    }
    if let Some(cut) = engine.loupe(crop.photo, focus, crop_side(canvas_scale))
        && let Some(texture) = state.loupe_texture(ui, &cut)
    {
        draw_layer(&painter, texture.id(), layer_polygon(&lens, Coverage::of_crop(&cut), crop.photo_size));
    }
    draw_outlines(&painter, &lens, crop);
    draw_crosshair(&painter, &lens);
    draw_rim(&painter, &lens);
}

/// Which way the lens goes from the handle: away from the page's middle, so the lens sits beside
/// the corner and not over the page's sides. The page is the one nearest the handle; with none,
/// the photo's middle stands in.
fn away_from_page(crop: &CropView, canvas: &Canvas, focus: Point, handle: Pos2) -> Vec2 {
    let middle = crop
        .outlines
        .iter()
        .map(|outline| canvas::centroid(&outline.corners))
        .min_by(|a, b| a.distance(focus).total_cmp(&b.distance(focus)))
        .unwrap_or_else(|| Point::new(f64::from(crop.photo_size.0) / 2.0, f64::from(crop.photo_size.1) / 2.0));
    handle - canvas.photo_to_canvas(middle)
}

/// Draws a soft shadow under the lens, so its edge is plain on a photo of any colour.
fn draw_shadow(painter: &Painter, lens: &Lens) {
    let centre = lens.centre + Vec2::new(0.0, 4.0);
    for step in 0..8 {
        painter.circle_filled(centre, RADIUS + 2.0 + step as f32 * 1.5, Color32::from_black_alpha(9));
    }
}

/// Draws one layer of the lens: the texture over the polygon, as a fan of triangles.
fn draw_layer(painter: &Painter, texture: egui::TextureId, polygon: Vec<(Pos2, Pos2)>) {
    if polygon.len() < 3 {
        return;
    }
    let mut mesh = Mesh::with_texture(texture);
    for (pos, uv) in &polygon {
        mesh.vertices.push(egui::epaint::Vertex { pos: *pos, uv: *uv, color: Color32::WHITE });
    }
    for i in 1..polygon.len() - 1 {
        mesh.indices.extend_from_slice(&[0, i as u32, i as u32 + 1]);
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the page's outline and the spine inside the lens: 1 pt over a dark halo, stopping short
/// of the crosshair, so the spot itself is clear.
fn draw_outlines(painter: &Painter, lens: &Lens, crop: &CropView) {
    let mut paths: Vec<(Vec<Point>, Color32)> = Vec::new();
    for outline in &crop.outlines {
        let colour = if outline.invalid.is_some() { theme::INVALID } else { theme::SCAN_CYAN };
        paths.push((canvas::outline_path(outline), colour));
    }
    if let Some(spine) = crop.spine {
        paths.push((vec![spine[0], spine[1]], theme::SCAN_CYAN));
    }
    for (path, colour) in paths {
        let points: Vec<Pos2> = path.iter().map(|p| lens.point(*p)).collect();
        for piece in pieces_in_ring(&points, lens.centre, CROSSHAIR_GAP, RADIUS - 1.0) {
            let more = theme::extra_width(painter.ctx());
            painter.add(Shape::line(piece.clone(), Stroke::new(LINE_HALO + more, theme::halo_stroke(painter.ctx()).color)));
            painter.add(Shape::line(piece, Stroke::new(LINE_WIDTH + more / 2.0, colour)));
        }
    }
}

/// Draws the crosshair: four short ticks with a gap between them, light over dark, so they show
/// on white paper and on a black background both. It marks the handle, which is where the lens is
/// centred.
fn draw_crosshair(painter: &Painter, lens: &Lens) {
    let c = lens.centre;
    for direction in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
        let (from, to) = (c + direction * CROSSHAIR_GAP, c + direction * (CROSSHAIR_GAP + CROSSHAIR_TICK));
        painter.add(Shape::line_segment([from, to], Stroke::new(3.0, Color32::from_black_alpha(170))));
        painter.add(Shape::line_segment([from, to], Stroke::new(1.5, Color32::WHITE)));
    }
}

/// Draws the rim: a white ring between two dark hairlines.
fn draw_rim(painter: &Painter, lens: &Lens) {
    painter.circle_stroke(lens.centre, RADIUS + 1.75, Stroke::new(1.0, Color32::from_black_alpha(110)));
    painter.circle_stroke(lens.centre, RADIUS, Stroke::new(2.5, Color32::WHITE));
    painter.circle_stroke(lens.centre, RADIUS - 1.75, Stroke::new(1.0, Color32::from_black_alpha(70)));
}

/// Whether a side is drawn dashed: the spine of a spread (§4.3).
pub fn is_spine(outline: &OutlineView, side: Side) -> bool {
    matches!(outline.sides[side.index()], SideShape::Spine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::image::Plane;
    use std::sync::Arc;

    fn lens_at(focus: Point) -> Lens {
        Lens::new(Pos2::new(400.0, 300.0), focus, 0.1)
    }

    #[test]
    fn the_lens_magnifies_three_times_the_canvas_zoom() {
        let lens = lens_at(Point::new(1000.0, 2000.0));
        assert!((lens.points_per_photo_px - 0.3).abs() < 1e-12);
        // A point 10 photo pixels to the right of the focus is 10 × 0.1 × 3 points from the centre.
        let p = lens.point(Point::new(1010.0, 2000.0));
        assert!((p.x - lens.centre.x - 3.0).abs() < 1e-4 && (p.y - lens.centre.y).abs() < 1e-4, "{p:?}");
    }

    #[test]
    fn the_lens_maps_points_both_ways() {
        let lens = lens_at(Point::new(120.0, 80.0));
        let photo = Point::new(150.5, 61.25);
        let back = lens.photo(lens.point(photo));
        assert!((back.x - photo.x).abs() < 1e-3 && (back.y - photo.y).abs() < 1e-3, "{back:?}");
    }

    #[test]
    fn a_crop_is_asked_for_in_proportion_to_what_the_lens_shows() {
        // The lens shows 180 / (0.1 × 3) = 600 photo pixels across.
        assert!((crop_side(0.1) - CROP_MARGIN * 600.0).abs() < 1e-9);
        assert_eq!(crop_side(0.0001), MAX_CROP);
        assert_eq!(crop_side(0.0), MAX_CROP);
        assert_eq!(crop_side(f64::NAN), MAX_CROP);
    }

    #[test]
    fn a_layer_never_samples_outside_its_texture_or_the_photo() {
        let photo = (4000, 3000);
        // The corner sits a little inside the photo's top-left: the lens reaches over its edge.
        for focus in [Point::new(30.0, 20.0), Point::new(3990.0, 2990.0), Point::new(2000.0, 1500.0), Point::new(-200.0, -100.0)] {
            let lens = Lens::new(Pos2::new(500.0, 400.0), focus, 0.1);
            for coverage in [Coverage::photo(photo), Coverage { origin: Point::new(focus.x - 300.0, focus.y - 300.0), size: (600.0, 600.0) }] {
                for (pos, uv) in layer_polygon(&lens, coverage, photo) {
                    assert!((-1e-4..=1.0 + 1e-4).contains(&uv.x) && (-1e-4..=1.0 + 1e-4).contains(&uv.y), "uv {uv:?} at {pos:?} for {focus:?}");
                    // And no vertex is outside the lens.
                    assert!(pos.distance(lens.centre) <= RADIUS + 1e-3, "{pos:?} is outside the lens");
                }
            }
        }
    }

    #[test]
    fn the_lens_is_dark_where_the_photo_ends() {
        // A focus on the photo's left edge: the left half of the lens has no photo, so the layer
        // is the right half of the circle.
        let lens = Lens::new(Pos2::new(500.0, 400.0), Point::new(0.0, 1500.0), 0.1);
        let polygon = layer_polygon(&lens, Coverage::photo((4000, 3000)), (4000, 3000));
        assert!(polygon.len() >= 3);
        assert!(polygon.iter().all(|(pos, _)| pos.x >= lens.centre.x - 1e-3), "the layer reaches left of the photo's edge");
        assert!(polygon.iter().any(|(pos, _)| pos.x > lens.centre.x + RADIUS - 1.0), "the layer stops short on the photo's side");
    }

    #[test]
    fn a_layer_with_no_overlap_draws_nothing() {
        let lens = Lens::new(Pos2::new(500.0, 400.0), Point::new(100.0, 100.0), 0.1);
        let far = Coverage { origin: Point::new(5000.0, 5000.0), size: (100.0, 100.0) };
        assert!(layer_polygon(&lens, far, (4000, 3000)).is_empty());
    }

    #[test]
    fn a_crops_coverage_is_its_size_in_photo_pixels() {
        let crop = LoupeCrop { image: Arc::new(Plane::new(200, 100, [0u8; 3]).unwrap()), origin: Point::new(10.0, 20.0), scale: 2.0 };
        assert_eq!(Coverage::of_crop(&crop), Coverage { origin: Point::new(10.0, 20.0), size: (400.0, 200.0) });
    }

    #[test]
    fn the_lens_sits_on_the_far_side_of_the_handle_from_the_page() {
        let window = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        // A top-left corner: the page is to the right and below, so the lens goes up and to the left.
        let handle = Pos2::new(600.0, 400.0);
        let centre = lens_centre(handle, Vec2::new(-1.0, -1.0), window);
        assert!((centre.distance(handle) - DISTANCE).abs() < 0.01, "{centre:?}");
        assert!(centre.x < handle.x && centre.y < handle.y, "{centre:?}");
    }

    #[test]
    fn the_lens_slides_round_the_handle_instead_of_flipping() {
        let window = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        // A handle near the window's top: straight up does not fit. The lens swings to the side
        // and stays 100 pt from the handle, and it does not end up below it.
        let handle = Pos2::new(600.0, 120.0);
        let centre = lens_centre(handle, Vec2::new(0.0, -1.0), window);
        assert!((centre.distance(handle) - DISTANCE).abs() < 0.01, "the lens is {} pt away", centre.distance(handle));
        assert!(centre.y >= window.min.y + WINDOW_MARGIN + RADIUS - 0.01, "the lens leaves the window: {centre:?}");
        assert!(centre.y <= handle.y + 0.01, "the lens flipped below the handle: {centre:?}");
    }

    #[test]
    fn the_lens_stays_inside_the_window() {
        let window = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        for handle in [Pos2::new(0.0, 0.0), Pos2::new(1200.0, 800.0), Pos2::new(5.0, 795.0), Pos2::new(600.0, 400.0)] {
            for away in [Vec2::new(0.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 0.0), Vec2::ZERO] {
                let centre = lens_centre(handle, away, window);
                assert!(centre.x - RADIUS >= window.min.x - 0.01 && centre.x + RADIUS <= window.max.x + 0.01, "{handle:?} {away:?} -> {centre:?}");
                assert!(centre.y - RADIUS >= window.min.y - 0.01 && centre.y + RADIUS <= window.max.y + 0.01, "{handle:?} {away:?} -> {centre:?}");
            }
        }
    }

    #[test]
    fn a_line_through_the_centre_is_cut_into_two_pieces_with_a_gap() {
        let line = [Pos2::new(-200.0, 0.0), Pos2::new(200.0, 0.0)];
        let pieces = pieces_in_ring(&line, Pos2::ZERO, 8.0, 89.0);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        assert!((pieces[0][0].x + 89.0).abs() < 0.01 && (pieces[0].last().map_or(0.0, |p| p.x) + 8.0).abs() < 0.01, "{pieces:?}");
        assert!((pieces[1][0].x - 8.0).abs() < 0.01 && (pieces[1].last().map_or(0.0, |p| p.x) - 89.0).abs() < 0.01, "{pieces:?}");
    }

    #[test]
    fn a_line_that_misses_the_ring_is_dropped() {
        let line = [Pos2::new(-200.0, 150.0), Pos2::new(200.0, 150.0)];
        assert!(pieces_in_ring(&line, Pos2::ZERO, 8.0, 89.0).is_empty());
    }

    #[test]
    fn a_bent_line_stays_one_piece_while_it_is_in_the_ring() {
        // An elbow at (50, 0): both legs are inside the ring, so it is one piece.
        let line = [Pos2::new(20.0, 0.0), Pos2::new(50.0, 0.0), Pos2::new(50.0, 40.0)];
        let pieces = pieces_in_ring(&line, Pos2::ZERO, 8.0, 89.0);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].len(), 3);
    }
}
