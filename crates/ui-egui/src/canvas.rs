//! Screen 2's canvas (§4.3): the display copy, the page outline, the scrim outside it and the
//! handles the user drags.
//!
//! The corners are stored in the full-resolution photo's pixels, and the canvas shows the display
//! copy, so every conversion goes through [`Canvas::photo_to_canvas`] and
//! [`Canvas::canvas_to_photo`]. Zoom and pan are part of the canvas, so those two stay the only
//! conversions there are.
//!
//! A frame handles the pointer first ([`interact_handles`]) and draws after it, so the handle,
//! the outline and the loupe all show the corner where the hand has just put it.

use egui::{Color32, Key, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EngineApi, OutlineView};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::SideShape;

use crate::a11y;
use crate::state::{CanvasView, EdgeDrag, Selection, UiState};
use crate::theme;

/// The padding between the canvas area and the photo (§4.3).
pub const PADDING: f32 = 32.0;
/// The radius of the hit area of a corner, a bend handle or a spine end (§4.3).
pub const HANDLE_HIT_RADIUS: f32 = 22.0;
/// How far outside the photo a corner may be dragged, as a share of the photo's width or height.
/// It matches the engine's limit, so the canvas always has room for every corner it can hold.
pub const VIRTUAL_MARGIN: f32 = 0.15;
/// The smallest zoom: the whole photo in the canvas.
pub const MIN_ZOOM: f32 = 1.0;
/// The largest zoom.
pub const MAX_ZOOM: f32 = 16.0;

/// Where the display copy is drawn, and how a photo pixel maps onto it.
#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    /// Everything the canvas may draw in and be touched in. Zoomed in, the photo is larger than
    /// this, and what lies outside it is neither drawn nor grabbed.
    pub area: Rect,
    /// The rectangle the display copy is drawn in, in points.
    pub rect: Rect,
    /// The photo's rectangle with the virtual margin around it: where a corner can be dragged to
    /// when the page's real corner lies outside the camera's view.
    pub workspace: Rect,
    /// Points per display-copy pixel.
    pub scale: f32,
    /// Display-copy pixels per full-resolution photo pixel: the display copy's downscale.
    pub k: f64,
    /// The full-resolution photo's size.
    pub photo_size: (u32, u32),
    /// The zoom and pan the canvas was made with, after they were kept within bounds.
    pub view: CanvasView,
}

impl Canvas {
    /// Fits the display copy into `area` with [`PADDING`] points of padding.
    pub fn new(area: Rect, display: &Rgb8, photo_size: (u32, u32)) -> Canvas {
        Canvas::with_view(area, display, photo_size, CanvasView::default())
    }

    /// The display copy in `area`, zoomed and panned by `view`.
    pub fn with_view(area: Rect, display: &Rgb8, photo_size: (u32, u32), view: CanvasView) -> Canvas {
        let dw = display.width() as f32;
        let dh = display.height() as f32;
        let inner = area.shrink(PADDING);
        let zoom = if view.zoom.is_finite() { view.zoom.clamp(MIN_ZOOM, MAX_ZOOM) } else { MIN_ZOOM };
        let (mut scale, mut shown) = (0.0, Vec2::ZERO);
        if dw > 0.0 && dh > 0.0 && inner.width() > 0.0 && inner.height() > 0.0 {
            // The margin is part of the fit, so the photo keeps its size while a corner leaves it.
            let wide = 1.0 + 2.0 * VIRTUAL_MARGIN;
            scale = (inner.width() / (dw * wide)).min(inner.height() / (dh * wide)) * zoom;
            shown = Vec2::new(dw * scale, dh * scale);
        }
        // The pan may take the margin's edge to the padded area's edge and no further, so the photo
        // cannot be lost off the canvas.
        let workspace_size = shown * (1.0 + 2.0 * VIRTUAL_MARGIN);
        let slack = ((workspace_size - inner.size()) / 2.0).max(Vec2::ZERO);
        let pan = if view.pan.x.is_finite() && view.pan.y.is_finite() {
            Vec2::new(view.pan.x.clamp(-slack.x, slack.x), view.pan.y.clamp(-slack.y, slack.y))
        } else {
            Vec2::ZERO
        };
        let rect = Rect::from_center_size(inner.center() + pan, shown);
        let workspace = rect.expand2(Vec2::new(rect.width() * VIRTUAL_MARGIN, rect.height() * VIRTUAL_MARGIN));
        let photo_long = photo_size.0.max(photo_size.1);
        let k = if photo_long > 0 { f64::from(display.width().max(display.height())) / f64::from(photo_long) } else { 1.0 };
        Canvas { area, rect, workspace, scale, k, photo_size, view: CanvasView { zoom, pan } }
    }

    /// A full-resolution photo position in canvas points.
    pub fn photo_to_canvas(&self, p: Point) -> Pos2 {
        self.rect.min + Vec2::new((p.x * self.k * f64::from(self.scale)) as f32, (p.y * self.k * f64::from(self.scale)) as f32)
    }

    /// A canvas position in full-resolution photo pixels.
    pub fn canvas_to_photo(&self, p: Pos2) -> Point {
        let k = self.k * f64::from(self.scale);
        if k <= 0.0 || !k.is_finite() {
            return Point::new(0.0, 0.0);
        }
        Point::new((f64::from(p.x) - f64::from(self.rect.min.x)) / k, (f64::from(p.y) - f64::from(self.rect.min.y)) / k)
    }

    /// Points per full-resolution photo pixel: the canvas's zoom on the photo.
    ///
    /// The display copy is `k` times the photo's size and is drawn at `scale` points a pixel, so
    /// a photo pixel is `scale × k` points. (It was `scale / k`, which made the loupe show the
    /// photo smaller than the canvas does.)
    pub fn points_per_photo_px(&self) -> f64 {
        if self.k <= 0.0 { 0.0 } else { f64::from(self.scale) * self.k }
    }
}

impl CanvasView {
    /// This view zoomed by `factor`, keeping the point of the photo under `about` where it is.
    /// `area` is the canvas's area, whose centre the photo is centred on when the pan is zero.
    pub fn zoomed(self, factor: f32, about: Pos2, area: Rect) -> CanvasView {
        if !factor.is_finite() || factor <= 0.0 {
            return self;
        }
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let applied = zoom / self.zoom;
        let from_centre = about - area.center();
        CanvasView { zoom, pan: from_centre * (1.0 - applied) + self.pan * applied }
    }
}

/// A picture of `size` points fitted into `area` with [`PADDING`] around it, zoomed and panned by
/// `view`: where it goes, and the view with its pan kept within bounds. The finished page on
/// Screen 3 is placed with this, as the photo is on Screen 2.
pub fn fit_view(area: Rect, size: (f32, f32), view: CanvasView) -> (Rect, CanvasView) {
    let inner = area.shrink(PADDING);
    let zoom = if view.zoom.is_finite() { view.zoom.clamp(MIN_ZOOM, MAX_ZOOM) } else { MIN_ZOOM };
    if size.0 <= 0.0 || size.1 <= 0.0 || inner.width() <= 0.0 || inner.height() <= 0.0 {
        return (Rect::from_center_size(area.center(), Vec2::ZERO), CanvasView { zoom, pan: Vec2::ZERO });
    }
    let scale = (inner.width() / size.0).min(inner.height() / size.1) * zoom;
    let shown = Vec2::new(size.0 * scale, size.1 * scale);
    let slack = ((shown - inner.size()) / 2.0).max(Vec2::ZERO);
    let pan = if view.pan.x.is_finite() && view.pan.y.is_finite() {
        Vec2::new(view.pan.x.clamp(-slack.x, slack.x), view.pan.y.clamp(-slack.y, slack.y))
    } else {
        Vec2::ZERO
    };
    (Rect::from_center_size(inner.center() + pan, shown), CanvasView { zoom, pan })
}

/// Zoom and pan from the pointer: a pinch or ⌘-scroll zooms about the pointer, a scroll or a drag
/// on the empty canvas pans, and ⌘0 fits the whole photo again.
///
/// `background` is the response of the canvas area's own widget, made before the handles so that
/// a drag that starts on a handle is the handle's.
pub fn navigate(ui: &Ui, area: Rect, background: &Response, view: &mut CanvasView) {
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| area.contains(*p));
    if let Some(pointer) = hover {
        let zoom = ui.input(|i| i.zoom_delta());
        if (zoom - 1.0).abs() > f32::EPSILON {
            *view = view.zoomed(zoom, pointer, area);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll != Vec2::ZERO && view.zoom > MIN_ZOOM {
            view.pan += scroll;
        }
    }
    if background.dragged() && view.zoom > MIN_ZOOM {
        view.pan += background.drag_delta();
    }
    if ui.input(|i| i.modifiers.command && i.key_pressed(Key::Num0)) {
        *view = CanvasView::default();
    }
}

/// The spline through `points` (centripetal Catmull–Rom, α = 0.5), `samples_per_span` points per
/// span.
///
/// The curve passes through every control point, which is what a bent side's five points (its two
/// corners and its three handles) need.
pub fn catmull_rom_centripetal(points: &[Point], samples_per_span: usize) -> Vec<Point> {
    let n = points.len();
    if n == 0 {
        return Vec::new();
    }
    if n < 3 {
        return points.to_vec();
    }
    let alpha = 0.5;
    let mut knots = vec![0.0f64; n];
    for i in 1..n {
        let d = points[i].distance(points[i - 1]);
        knots[i] = knots[i - 1] + if d.is_finite() && d > 0.0 { d.powf(alpha) } else { 0.0 };
    }
    if knots[n - 1] <= knots[0] {
        // Every point is in the same place: there is no curve to draw.
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n * samples_per_span.max(1));
    for i in 0..n - 1 {
        let p0 = if i == 0 { points[0] } else { points[i - 1] };
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = if i + 2 < n { points[i + 2] } else { points[n - 1] };
        // The end spans repeat the first or the last point, so their knot spacing is
        // extrapolated instead of zero.
        let (t0, t1, t2, t3) = if i == 0 {
            (knots[0] - (knots[1] - knots[0]), knots[0], knots[1], if n > 2 { knots[2] } else { knots[1] })
        } else if i + 2 >= n {
            (knots[i - 1], knots[i], knots[i + 1], knots[i + 1] + (knots[i + 1] - knots[i]))
        } else {
            (knots[i - 1], knots[i], knots[i + 1], knots[i + 2])
        };
        if t2 - t1 <= 0.0 {
            // A degenerate span: the two points are in the same place.
            out.push(p1);
            continue;
        }
        for s in 0..samples_per_span.max(1) {
            let u = f64::from(s as u32) / samples_per_span.max(1) as f64;
            let t = t1 + (t2 - t1) * u;
            out.push(evaluate([p0, p1, p2, p3], [t0, t1, t2, t3], t));
        }
    }
    out.push(points[n - 1]);
    out
}

/// One point of the non-uniform (Barry–Goldman) Catmull–Rom curve.
fn evaluate(points: [Point; 4], knots: [f64; 4], t: f64) -> Point {
    let [p0, p1, p2, p3] = points;
    let [t0, t1, t2, t3] = knots;
    let lerp = |a: Point, b: Point, wa: f64, wb: f64| Point::new(a.x * wa + b.x * wb, a.y * wa + b.y * wb);
    let a1 = if t1 - t0 > 0.0 { lerp(p0, p1, (t1 - t) / (t1 - t0), (t - t0) / (t1 - t0)) } else { p1 };
    let a2 = lerp(p1, p2, (t2 - t) / (t2 - t1), (t - t1) / (t2 - t1));
    let a3 = if t3 - t2 > 0.0 { lerp(p2, p3, (t3 - t) / (t3 - t2), (t - t2) / (t3 - t2)) } else { p2 };
    let b1 = if t2 - t0 > 0.0 { lerp(a1, a2, (t2 - t) / (t2 - t0), (t - t0) / (t2 - t0)) } else { a2 };
    let b2 = if t3 - t1 > 0.0 { lerp(a2, a3, (t3 - t) / (t3 - t1), (t - t1) / (t3 - t1)) } else { a2 };
    lerp(b1, b2, (t2 - t) / (t2 - t1), (t - t1) / (t2 - t1))
}

/// One corner of a quadrilateral, by index.
fn quad_point(quad: &Quad, index: usize) -> Point {
    quad.0.get(index).copied().unwrap_or_default()
}

/// The polyline of one side of the outline, in photo pixels.
///
/// A straight side is a straight line, and a bent side is the spline through its two corners and
/// its three handles, which the interface keeps on one parabola so a single vertex moves them.
pub fn side_points(outline: &OutlineView, side: Side) -> Vec<Point> {
    let (a, b) = side.corners();
    let start = quad_point(&outline.corners, a);
    let end = quad_point(&outline.corners, b);
    match &outline.sides[side.index()] {
        // A side the engine traced is evidence for flattening the page, not an outline to show:
        // the outline is the four corners, and only a side the user bent has a curve.
        SideShape::Straight | SideShape::Spine | SideShape::Traced { .. } => vec![start, end],
        SideShape::Bent { handles } => {
            let mut control = Vec::with_capacity(5);
            control.push(start);
            control.extend_from_slice(handles);
            control.push(end);
            catmull_rom_centripetal(&control, 16)
        }
    }
}

/// Whether a side is curved, and so has bend handles (§4.3).
pub fn side_is_curved(outline: &OutlineView, side: Side) -> bool {
    matches!(&outline.sides[side.index()], SideShape::Bent { .. })
}

/// The bend handles of a side, in photo pixels: the side's own handles when it is bent, and the
/// points at a quarter, half and three quarters of its traced curve otherwise.
pub fn bend_handles(outline: &OutlineView, side: Side) -> Vec<Point> {
    match &outline.sides[side.index()] {
        SideShape::Bent { handles } => handles.to_vec(),
        _ => Vec::new(),
    }
}

/// The index of the one bend handle the interface shows: the middle one.
pub const BEND_VERTEX: usize = 1;

/// The three handles of a side bent through `middle`: a parabola from one corner to the other that
/// passes through it, sampled at a quarter, half and three quarters.
pub fn bend_through(outline: &OutlineView, side: Side, middle: Point) -> [Point; 3] {
    let (a, b) = side.corners();
    let (start, end) = (quad_point(&outline.corners, a), quad_point(&outline.corners, b));
    let chord_middle = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
    let (dx, dy) = (middle.x - chord_middle.x, middle.y - chord_middle.y);
    [0.25, 0.5, 0.75].map(|t: f64| {
        let k = 4.0 * t * (1.0 - t);
        Point::new(start.x + (end.x - start.x) * t + dx * k, start.y + (end.y - start.y) * t + dy * k)
    })
}

/// The points at the given fractions of a polyline's arc length.
pub fn points_at_fractions(points: &[Point], fractions: &[f64]) -> Vec<Point> {
    let mut out = Vec::with_capacity(fractions.len());
    if points.len() < 2 {
        return out;
    }
    let mut lengths = Vec::with_capacity(points.len().saturating_sub(1));
    let mut total = 0.0;
    for i in 1..points.len() {
        let d = points[i].distance(points[i - 1]);
        lengths.push(d);
        total += d;
    }
    if total <= 0.0 {
        for _ in fractions {
            out.push(points[0]);
        }
        return out;
    }
    for fraction in fractions {
        let target = total * fraction.clamp(0.0, 1.0);
        let mut walked = 0.0;
        let mut found = points[points.len() - 1];
        for i in 0..lengths.len() {
            let d = lengths[i];
            if walked + d >= target && d > 0.0 {
                let t = (target - walked) / d;
                let a = points[i];
                let b = points[i + 1];
                found = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                break;
            }
            walked += d;
        }
        out.push(found);
    }
    out
}

/// The interior of the outline, as a closed polygon in photo points: the four corners, with the
/// curve of a side the user bent.
pub fn outline_polygon(outline: &OutlineView) -> Vec<Point> {
    let top = side_points(outline, Side::Top);
    let right = side_points(outline, Side::Right);
    let bottom = side_points(outline, Side::Bottom);
    let left = side_points(outline, Side::Left);
    let mut points = Vec::new();
    points.extend(top);
    points.extend(right.into_iter().skip(1));
    points.extend(bottom.into_iter().rev().skip(1));
    points.extend(left.into_iter().rev().skip(1));
    // The walk ends on the first point; a polygon does not repeat it, and the triangulation drew
    // only half of the page's inside when it did.
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

/// The page's centre, in photo pixels.
pub fn centroid(corners: &Quad) -> Point {
    let n = 4.0;
    Point::new(corners.0.iter().map(|p| p.x).sum::<f64>() / n, corners.0.iter().map(|p| p.y).sum::<f64>() / n)
}

/// The unit normal of a side, pointing away from the page's centre.
pub fn outward_normal(corners: &Quad, side: Side) -> Option<Point> {
    let (a, b) = side.corners();
    let c0 = quad_point(corners, a);
    let c1 = quad_point(corners, b);
    let dx = c1.x - c0.x;
    let dy = c1.y - c0.y;
    let length = dx.hypot(dy);
    if length <= 1e-9 {
        return None;
    }
    let mut n = Point::new(dy / length, -dx / length);
    let middle = Point::new((c0.x + c1.x) / 2.0, (c0.y + c1.y) / 2.0);
    let centre = centroid(corners);
    if n.x * (middle.x - centre.x) + n.y * (middle.y - centre.y) < 0.0 {
        n = Point::new(-n.x, -n.y);
    }
    Some(n)
}

/// The middle of a side, in photo pixels.
pub fn side_middle(corners: &Quad, side: Side) -> Point {
    let (a, b) = side.corners();
    let c0 = quad_point(corners, a);
    let c1 = quad_point(corners, b);
    Point::new((c0.x + c1.x) / 2.0, (c0.y + c1.y) / 2.0)
}

/// Triangulates a simple polygon. The triangles' vertex indices are into `points`.
///
/// The outline's interior may be concave (a bent side can dip inwards), so the polygon is
/// clipped ear by ear. When the clipping gets stuck, a fan from the first corner is used, which
/// is right for a convex polygon and close enough for the rest.
pub fn triangulate(points: &[Pos2]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    if signed_area(points, &order) < 0.0 {
        order.reverse();
    }
    let mut triangles = Vec::new();
    let mut guard = 0;
    while order.len() > 3 && guard <= n * n + 16 {
        guard += 1;
        let mut clipped = false;
        for i in 0..order.len() {
            let len = order.len();
            let prev = order[(i + len - 1) % len];
            let curr = order[i];
            let next = order[(i + 1) % len];
            let (a, b, c) = (points[prev], points[curr], points[next]);
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            let inside = order.iter().any(|&p| p != prev && p != curr && p != next && point_in_triangle(points[p], a, b, c));
            if inside {
                continue;
            }
            triangles.push([prev, curr, next]);
            order.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if order.len() == 3 {
        triangles.push([order[0], order[1], order[2]]);
    } else if triangles.is_empty() {
        // The clipping made no progress: a fan from the first corner.
        for i in 1..n.saturating_sub(1) {
            triangles.push([0, i, i + 1]);
        }
    }
    triangles
}

/// Twice the signed area of a polygon, with the given vertex order.
fn signed_area(points: &[Pos2], order: &[usize]) -> f64 {
    let mut area = 0.0;
    for i in 0..order.len() {
        let a = points[order[i]];
        let b = points[order[(i + 1) % order.len()]];
        area += f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y);
    }
    area
}

/// The cross product of the vectors `b - a` and `c - b`.
fn cross(a: Pos2, b: Pos2, c: Pos2) -> f64 {
    let abx = f64::from(b.x - a.x);
    let aby = f64::from(b.y - a.y);
    let bcx = f64::from(c.x - b.x);
    let bcy = f64::from(c.y - b.y);
    abx * bcy - aby * bcx
}

/// Whether `p` is inside the triangle `a`, `b`, `c`.
fn point_in_triangle(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

/// Says that a canvas is waiting for its picture: a spinner and a line of text in the middle of
/// `rect`, so a canvas is never blank without saying why.
pub fn waiting(ui: &mut Ui, rect: Rect, label: &str) {
    let spinner = Rect::from_center_size(rect.center() - Vec2::new(0.0, 14.0), Vec2::splat(28.0));
    theme::spinner(ui, spinner, Color32::from_white_alpha(200));
    let line = Rect::from_center_size(rect.center() + Vec2::new(0.0, 24.0), Vec2::new(rect.width().min(360.0), 22.0));
    ui.put(line, egui::Label::new(egui::RichText::new(label).color(Color32::from_white_alpha(200))).halign(egui::Align::Center));
}

/// Draws the canvas: the background, the display copy, the scrim outside the outline and the
/// outline itself.
pub fn draw_canvas(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) {
    // Edge to edge: the dark fills the whole canvas, not a box inside it.
    theme::canvas_background(ui, canvas.area);
    draw_virtual_area(ui, canvas, crop);
    let Some(texture) = state.texture(ui, engine, scancraft_core::engine_api::ImageKey::DisplayCopy(crop.photo), egui::TextureOptions::LINEAR) else {
        return;
    };
    let id = texture.id();
    let painter = ui.painter_at(canvas.rect.intersect(canvas.area));
    // The photo, then the scrim over it, then the photo again inside the outline: the page stays
    // at full brightness and everything outside it is dimmed (§4.3).
    painter.image(id, canvas.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    painter.rect_filled(canvas.rect, 0.0, theme::SCRIM);
    for outline in &crop.outlines {
        draw_photo_inside(&painter, canvas, outline, id);
    }
}

/// Draws the room around the photo, where a corner can be put when the page's real corner is out
/// of the camera's view: a lighter field, the photo's own edge, and the part of the page outline
/// that lies out there, tinted, so the "virtual" page reads as a continuation of the photo.
fn draw_virtual_area(ui: &Ui, canvas: &Canvas, crop: &CropView) {
    let painter = ui.painter_at(canvas.workspace.intersect(canvas.area));
    painter.rect_filled(canvas.workspace, 0.0, Color32::from_white_alpha(10));
    // Dashed edges mark where the camera's view ends.
    let edge = Stroke::new(1.0, Color32::from_white_alpha(90));
    let corners = [canvas.rect.left_top(), canvas.rect.right_top(), canvas.rect.right_bottom(), canvas.rect.left_bottom(), canvas.rect.left_top()];
    painter.extend(Shape::dashed_line(&corners, edge, 6.0, 4.0));
    for outline in &crop.outlines {
        let polygon: Vec<Pos2> = outline_polygon(outline).iter().map(|p| canvas.photo_to_canvas(*p)).collect();
        if polygon.len() < 3 {
            continue;
        }
        let mut mesh = Mesh::default();
        let tint = Color32::from_rgba_unmultiplied(theme::SCAN_CYAN.r(), theme::SCAN_CYAN.g(), theme::SCAN_CYAN.b(), 40);
        for p in &polygon {
            mesh.colored_vertex(*p, tint);
        }
        for triangle in triangulate(&polygon) {
            mesh.add_triangle(triangle[0] as u32, triangle[1] as u32, triangle[2] as u32);
        }
        if !mesh.indices.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }
}

/// Draws the photo again, over the outline's interior only.
fn draw_photo_inside(painter: &Painter, canvas: &Canvas, outline: &OutlineView, texture: egui::TextureId) {
    let polygon: Vec<Pos2> = outline_polygon(outline).iter().map(|p| canvas.photo_to_canvas(*p)).collect();
    if polygon.len() < 3 || canvas.rect.width() <= 0.0 || canvas.rect.height() <= 0.0 {
        return;
    }
    let mut mesh = Mesh::with_texture(texture);
    for p in &polygon {
        let uv = Pos2::new((p.x - canvas.rect.min.x) / canvas.rect.width(), (p.y - canvas.rect.min.y) / canvas.rect.height());
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: Color32::WHITE });
    }
    for triangle in triangulate(&polygon) {
        mesh.indices.push(triangle[0] as u32);
        mesh.indices.push(triangle[1] as u32);
        mesh.indices.push(triangle[2] as u32);
    }
    if mesh.indices.is_empty() {
        return;
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the page outline: a 2 pt Scan Cyan line over a 4 pt black halo at 50 %, so a dark edge
/// shows on both sides of the line (§4.3).
pub fn draw_outline(painter: &Painter, canvas: &Canvas, outline: &OutlineView) {
    let colour = if outline.invalid.is_some() { theme::INVALID } else { theme::SCAN_CYAN };
    let points = outline_path(outline);
    if points.len() < 2 {
        return;
    }
    let canvas_points: Vec<Pos2> = points.iter().map(|p| canvas.photo_to_canvas(*p)).collect();
    painter.add(Shape::line(canvas_points.clone(), Stroke::new(theme::HALO_WIDTH, theme::HALO)));
    painter.add(Shape::line(canvas_points, Stroke::new(theme::OUTLINE_WIDTH, colour)));
}

/// The outline's path in photo pixels: the four sides, walked clockwise from the top-left corner
/// back to it. A side is stored in its drawing order (bottom from the left corner, left from the
/// top one), so the bottom and the left are walked backwards; walking them as stored made the
/// outline double back across the page and left the left side open.
pub fn outline_path(outline: &OutlineView) -> Vec<Point> {
    let mut points: Vec<Point> = Vec::new();
    for (side, backwards) in [(Side::Top, false), (Side::Right, false), (Side::Bottom, true), (Side::Left, true)] {
        // The spine is drawn on its own, dashed (§4.3): its chord is all the outline needs here.
        let mut line = side_points(outline, side);
        if backwards {
            line.reverse();
        }
        let skip = usize::from(!points.is_empty());
        points.extend(line.into_iter().skip(skip));
    }
    points
}

/// The four corners, in the order of a quadrilateral's points.
const CORNERS: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft];

/// The hit area of a handle at `position`, in the part of it that lies inside the canvas. A
/// handle that has scrolled out of the canvas has none, so it cannot take a press that belongs to
/// whatever lies there.
fn grip(ui: &Ui, canvas: &Canvas, id: egui::Id, position: Pos2) -> Option<Response> {
    let hit = Rect::from_center_size(position, Vec2::splat(HANDLE_HIT_RADIUS * 2.0)).intersect(canvas.area);
    if hit.width() <= 0.0 || hit.height() <= 0.0 {
        return None;
    }
    Some(ui.interact(hit, id, Sense::click_and_drag()))
}

/// What a press on a handle does to the loupe and the cursor: the loupe opens on the handle while
/// the button is down, and the mouse cursor goes, so it does not hide what the loupe shows.
fn track_press(ui: &Ui, state: &mut UiState, response: &Response, id: egui::Id, focus: Point) {
    if response.is_pointer_button_down_on() {
        state.pressed_handle = Some(id);
        state.hold_loupe(focus);
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
}

/// Where a dragged handle's centre should go: where the pointer is, plus how far off the handle's
/// centre the press began. The handle moves with the pointer instead of jumping to it, so a grab
/// off the middle of a 44 pt hit area does not move the corner by up to 22 pt.
fn drag_target(ui: &Ui, state: &mut UiState, response: &Response, position: Pos2) -> Option<Pos2> {
    if response.drag_started() {
        let press = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos()).unwrap_or(position);
        state.grab = position - press;
    }
    if response.dragged() {
        let pointer = response.interact_pointer_pos()?;
        return Some(pointer + state.grab);
    }
    None
}

/// Handles the pointer on every handle of the crop screen, before anything is drawn. Returns
/// whether a command went to the engine, so the caller reads the view again.
pub fn interact_handles(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) -> bool {
    state.pressed_handle = None;
    let mut changed = false;
    for outline in &crop.outlines {
        // The sides first, so a corner that overlaps one is on top of it.
        for side in Side::ALL {
            changed |= if side_is_curved(outline, side) {
                bend_handle(ui, canvas, outline, side, state, engine)
            } else {
                side_handle(ui, canvas, outline, side, state, engine)
            };
        }
        for (index, corner) in CORNERS.into_iter().enumerate() {
            changed |= corner_handle(ui, canvas, outline, index, corner, state, engine);
        }
    }
    changed |= spine_handles(ui, canvas, crop, state, engine);
    changed
}

/// One corner: a press opens the loupe, a drag moves the corner (§4.3).
fn corner_handle(
    ui: &Ui,
    canvas: &Canvas,
    outline: &OutlineView,
    index: usize,
    corner: Corner,
    state: &mut UiState,
    engine: &mut dyn EngineApi,
) -> bool {
    let page = outline.page;
    let point = quad_point(&outline.corners, index);
    let position = canvas.photo_to_canvas(point);
    let id = ui.make_persistent_id(("scancraft.corner", page, corner.index()));
    let Some(response) = grip(ui, canvas, id, position) else { return false };
    track_press(ui, state, &response, id, point);
    if response.clicked() || response.drag_started() {
        state.selection = Some(Selection::Corner { page, corner });
    }
    let mut changed = false;
    // A drag that has begun on the node stays with it, however far the pointer jumps in a frame.
    if let Some(target) = drag_target(ui, state, &response, position) {
        let moved = canvas.canvas_to_photo(target);
        dispatch(state, engine, Command::CropSetCorner { page, corner, x: moved.x, y: moved.y });
        state.hold_loupe(moved);
        changed = true;
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::corner_label(corner, point)));
    changed
}

/// The distance of `p` from `origin` along the unit vector `normal`, in photo pixels.
fn along(p: Point, origin: Point, normal: Point) -> f64 {
    (p.x - origin.x) * normal.x + (p.y - origin.y) * normal.y
}

/// One straight side: a pill at its middle. A drag moves the side along its outward normal
/// (§4.3), and a double-click gives it its one vertex, which then bends it.
fn side_handle(ui: &Ui, canvas: &Canvas, outline: &OutlineView, side: Side, state: &mut UiState, engine: &mut dyn EngineApi) -> bool {
    let page = outline.page;
    let middle = side_middle(&outline.corners, side);
    let position = canvas.photo_to_canvas(middle);
    let id = ui.make_persistent_id(("scancraft.edge", page, side.index()));
    let Some(response) = grip(ui, canvas, id, position) else { return false };
    track_press(ui, state, &response, id, middle);
    let mut changed = false;
    if response.double_clicked() {
        let handles = bend_through(outline, side, middle);
        dispatch(state, engine, Command::CropBendSide { page, side, handles });
        changed = true;
    } else {
        if response.drag_started()
            && let Some(normal) = outward_normal(&outline.corners, side)
        {
            let press = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos()).unwrap_or(position);
            let last = along(canvas.canvas_to_photo(press), middle, normal);
            state.edge_drag = Some(EdgeDrag { page, side, middle, normal, last });
        }
        if response.dragged()
            && let Some(drag) = state.edge_drag.filter(|d| d.page == page && d.side == side)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let reached = along(canvas.canvas_to_photo(pointer), drag.middle, drag.normal);
            let offset = reached - drag.last;
            if offset != 0.0 && offset.is_finite() {
                dispatch(state, engine, Command::CropMoveSide { page, side, offset });
                state.edge_drag = Some(EdgeDrag { last: reached, ..drag });
                state.hold_loupe(Point::new(middle.x + drag.normal.x * offset, middle.y + drag.normal.y * offset));
                changed = true;
            }
        }
    }
    if response.drag_stopped() {
        state.edge_drag = None;
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Slider, true, format!("{} side handle: drag to move, double-click to bend", a11y::side_name(side)))
    });
    changed
}

/// The one vertex of a bent side: a press opens the loupe, a drag bends the side (§4.3).
fn bend_handle(ui: &Ui, canvas: &Canvas, outline: &OutlineView, side: Side, state: &mut UiState, engine: &mut dyn EngineApi) -> bool {
    let page = outline.page;
    let handles = bend_handles(outline, side);
    let Some(handle) = handles.get(BEND_VERTEX).copied() else { return false };
    let position = canvas.photo_to_canvas(handle);
    let id = ui.make_persistent_id(("scancraft.bend", page, side.index(), BEND_VERTEX));
    let Some(response) = grip(ui, canvas, id, position) else { return false };
    track_press(ui, state, &response, id, handle);
    if response.clicked() || response.drag_started() {
        state.selection = Some(Selection::Bend { page, side, index: BEND_VERTEX });
    }
    let mut changed = false;
    if let Some(target) = drag_target(ui, state, &response, position) {
        let vertex = canvas.canvas_to_photo(target);
        dispatch(state, engine, Command::CropBendSide { page, side, handles: bend_through(outline, side, vertex) });
        state.hold_loupe(vertex);
        changed = true;
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::bend_label(side, BEND_VERTEX)));
    response.context_menu(|ui| {
        if ui.button(state.text.t("Straighten Side")).clicked() {
            dispatch(state, engine, Command::CropStraightenSide { page, side });
            changed = true;
        }
    });
    changed
}

/// The two ends of the spine of a split spread: a press opens the loupe, a drag moves the end
/// (§4.3).
fn spine_handles(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) -> bool {
    let Some(spine) = crop.spine else { return false };
    let mut changed = false;
    for (index, point) in spine.iter().enumerate() {
        let position = canvas.photo_to_canvas(*point);
        let id = ui.make_persistent_id(("scancraft.spine", crop.photo, index));
        let Some(response) = grip(ui, canvas, id, position) else { continue };
        track_press(ui, state, &response, id, *point);
        if response.clicked() || response.drag_started() {
            state.selection = Some(Selection::Spine { photo: crop.photo, end: index });
        }
        if let Some(target) = drag_target(ui, state, &response, position) {
            let moved = canvas.canvas_to_photo(target);
            let (top, bottom) = if index == 0 { (moved, spine[1]) } else { (spine[0], moved) };
            dispatch(state, engine, Command::CropSetSpine { photo: crop.photo, top, bottom });
            state.hold_loupe(moved);
            changed = true;
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::spine_label(index)));
    }
    changed
}

/// Sends a command to the engine and shows its error as a message (§4.8). Returns whether the
/// engine accepted it.
pub fn dispatch(state: &mut UiState, engine: &mut dyn EngineApi, command: Command) -> bool {
    match engine.dispatch(command) {
        Ok(()) => true,
        Err(error) => {
            state.notice(error, true);
            false
        }
    }
}

/// Draws a ring around a handle the keyboard has selected, so it can be told from the rest.
fn selection_ring(painter: &Painter, position: Pos2, radius: f32) {
    painter.circle_stroke(position, radius + 7.0, Stroke::new(4.0, theme::HALO));
    painter.circle_stroke(position, radius + 7.0, Stroke::new(1.5, theme::HANDLE));
}

/// Draws a round handle: a white dot in a cyan ring over a dark halo. While it is held it is a
/// ring only, so the corner it marks shows through (§4.3).
fn round_handle(painter: &Painter, position: Pos2, radius: f32, held: bool) {
    painter.circle_stroke(position, radius + theme::CORNER_RING / 2.0, Stroke::new(theme::CORNER_RING + 2.0, theme::HALO));
    if !held {
        painter.circle_filled(position, radius, theme::HANDLE);
    }
    painter.circle_stroke(position, radius + theme::CORNER_RING / 2.0, Stroke::new(theme::CORNER_RING, theme::SCAN_CYAN));
    if held {
        painter.circle_stroke(position, radius - 1.0, Stroke::new(1.5, theme::HANDLE));
    }
}

/// Draws the pill of a straight side: a capsule along the side, white in a cyan edge.
fn pill(painter: &Painter, position: Pos2, along: Vec2) {
    let length = theme::EDGE_HANDLE.y;
    let width = theme::EDGE_HANDLE.x;
    let reach = (length - width) / 2.0;
    let (a, b) = (position - along * reach, position + along * reach);
    for (extra, colour) in [(5.0, theme::HALO), (3.0, theme::SCAN_CYAN), (0.0, theme::HANDLE)] {
        let w = width + extra;
        painter.line_segment([a, b], Stroke::new(w, colour));
        painter.circle_filled(a, w / 2.0, colour);
        painter.circle_filled(b, w / 2.0, colour);
    }
}

/// Draws the handles of every outline: a pill on each straight side, the one vertex of each bent
/// side, the four corners, and the spine's ends. The handles are drawn after
/// [`interact_handles`], from the same view, so they are where the hand has just put them.
pub fn draw_handles(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &UiState) {
    let painter = ui.painter_at(canvas.area);
    let held = |id: egui::Id| state.pressed_handle == Some(id);
    for outline in &crop.outlines {
        let page = outline.page;
        for side in Side::ALL {
            if side_is_curved(outline, side) {
                let handles = bend_handles(outline, side);
                let Some(handle) = handles.get(BEND_VERTEX) else { continue };
                let position = canvas.photo_to_canvas(*handle);
                let id = ui.make_persistent_id(("scancraft.bend", page, side.index(), BEND_VERTEX));
                round_handle(&painter, position, theme::BEND_RADIUS, held(id));
                if state.selection == Some(Selection::Bend { page, side, index: BEND_VERTEX }) {
                    selection_ring(&painter, position, theme::BEND_RADIUS);
                }
            } else {
                let (a, b) = side.corners();
                let direction = canvas.photo_to_canvas(quad_point(&outline.corners, b)) - canvas.photo_to_canvas(quad_point(&outline.corners, a));
                let along = if direction.length() > f32::EPSILON { direction.normalized() } else { Vec2::X };
                pill(&painter, canvas.photo_to_canvas(side_middle(&outline.corners, side)), along);
            }
        }
        for (index, corner) in CORNERS.into_iter().enumerate() {
            let position = canvas.photo_to_canvas(quad_point(&outline.corners, index));
            let id = ui.make_persistent_id(("scancraft.corner", page, corner.index()));
            round_handle(&painter, position, theme::CORNER_RADIUS, held(id));
            if state.selection == Some(Selection::Corner { page, corner }) {
                selection_ring(&painter, position, theme::CORNER_RADIUS);
            }
        }
    }
    draw_spine(ui, canvas, crop, state);
}

/// Draws the spine of a split spread: a dashed Scan Cyan line with a round handle at each end
/// (§4.3).
fn draw_spine(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &UiState) {
    let Some(spine) = crop.spine else { return };
    let painter = ui.painter_at(canvas.area);
    let a = canvas.photo_to_canvas(spine[0]);
    let b = canvas.photo_to_canvas(spine[1]);
    for shape in Shape::dashed_line(&[a, b], Stroke::new(theme::HALO_WIDTH, theme::HALO), 6.0, 4.0) {
        painter.add(shape);
    }
    for shape in Shape::dashed_line(&[a, b], Stroke::new(theme::OUTLINE_WIDTH, theme::SCAN_CYAN), 6.0, 4.0) {
        painter.add(shape);
    }
    for (index, point) in spine.iter().enumerate() {
        let position = canvas.photo_to_canvas(*point);
        let id = ui.make_persistent_id(("scancraft.spine", crop.photo, index));
        let held = state.pressed_handle == Some(id);
        if !held {
            painter.circle_filled(position, theme::SPINE_RADIUS, theme::SCAN_CYAN);
        }
        painter.circle_stroke(position, theme::SPINE_RADIUS + 1.0, Stroke::new(2.0, theme::HANDLE));
        if state.selection == (Some(Selection::Spine { photo: crop.photo, end: index })) {
            selection_ring(&painter, position, theme::SPINE_RADIUS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::recipe::{CornerOrigin, FlattenMode, PageId};

    fn outline() -> OutlineView {
        OutlineView {
            page: PageId(1),
            corners: Quad([Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 200.0), Point::new(0.0, 200.0)]),
            corners_from: CornerOrigin::Manual,
            sides: Default::default(),
            fitted: None,
            half: None,
            invalid: None,
            flatten: FlattenMode::Auto,
            flatten_decision: None,
        }
    }

    #[test]
    fn a_vertex_on_the_chord_keeps_the_side_straight() {
        let handles = bend_through(&outline(), Side::Top, Point::new(50.0, 0.0));
        assert_eq!(handles, [Point::new(25.0, 0.0), Point::new(50.0, 0.0), Point::new(75.0, 0.0)]);
    }

    #[test]
    fn a_dragged_vertex_bends_the_side_on_one_parabola() {
        let handles = bend_through(&outline(), Side::Top, Point::new(50.0, -20.0));
        assert_eq!(handles[1], Point::new(50.0, -20.0));
        // A quarter along, a parabola has gone three quarters of the way to the vertex.
        assert!((handles[0].y + 15.0).abs() < 1e-9 && (handles[2].y + 15.0).abs() < 1e-9, "{handles:?}");
    }

    #[test]
    fn the_outline_is_one_closed_loop_of_the_four_corners() {
        let o = outline();
        let c = o.corners.0;
        assert_eq!(outline_path(&o), vec![c[0], c[1], c[2], c[3], c[0]]);
    }

    #[test]
    fn a_traced_side_is_drawn_straight_and_has_no_vertex() {
        let mut o = outline();
        o.sides[Side::Top.index()] = SideShape::Traced {
            samples: vec![scancraft_core::recipe::SideSample { point: Point::new(50.0, 40.0), weight: 1.0 }],
            bow: 0.2,
            curved: true,
        };
        assert_eq!(side_points(&o, Side::Top).len(), 2);
        assert!(bend_handles(&o, Side::Top).is_empty());
        assert!(!side_is_curved(&o, Side::Top));
    }
}
