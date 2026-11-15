//! Screen 2's canvas (§4.3): the display copy, the page outline, the scrim outside it and the
//! handles the user drags.
//!
//! The corners are stored in the full-resolution photo's pixels, and the canvas shows the display
//! copy, so every conversion goes through [`Canvas::photo_to_canvas`] and
//! [`Canvas::canvas_to_photo`].

use egui::{Color32, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EngineApi, OutlineView};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::SideShape;

use crate::a11y;
use crate::state::{Selection, UiState};
use crate::theme;

/// The padding between the canvas area and the photo (§4.3).
pub const PADDING: f32 = 32.0;
/// The radius of the hit area of a corner, a bend handle or a spine end (§4.3).
pub const HANDLE_HIT_RADIUS: f32 = 22.0;

/// Where the display copy is drawn, and how a photo pixel maps onto it.
#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    /// The rectangle the display copy is drawn in, in points.
    pub rect: Rect,
    /// Points per display-copy pixel.
    pub scale: f32,
    /// Display-copy pixels per full-resolution photo pixel: the display copy's downscale.
    pub k: f64,
    /// The full-resolution photo's size.
    pub photo_size: (u32, u32),
}

impl Canvas {
    /// Fits the display copy into `area` with [`PADDING`] points of padding.
    pub fn new(area: Rect, display: &Rgb8, photo_size: (u32, u32)) -> Canvas {
        let dw = display.width() as f32;
        let dh = display.height() as f32;
        let inner = area.shrink(PADDING);
        let (mut scale, mut shown) = (0.0, Vec2::ZERO);
        if dw > 0.0 && dh > 0.0 && inner.width() > 0.0 && inner.height() > 0.0 {
            scale = (inner.width() / dw).min(inner.height() / dh);
            shown = Vec2::new(dw * scale, dh * scale);
        }
        let rect = Rect::from_min_size(inner.min + (inner.size() - shown) / 2.0, shown);
        let photo_long = photo_size.0.max(photo_size.1);
        let k = if photo_long > 0 { f64::from(display.width().max(display.height())) / f64::from(photo_long) } else { 1.0 };
        Canvas { rect, scale, k, photo_size }
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

    /// Points per full-resolution photo pixel.
    pub fn points_per_photo_px(&self) -> f64 {
        if self.k <= 0.0 { 0.0 } else { f64::from(self.scale) / self.k }
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
/// A straight side is a straight line, a traced side follows its samples, and a bent side is the
/// spline through its two corners and its three handles.
pub fn side_points(outline: &OutlineView, side: Side) -> Vec<Point> {
    let (a, b) = side.corners();
    let start = quad_point(&outline.corners, a);
    let end = quad_point(&outline.corners, b);
    match &outline.sides[side.index()] {
        SideShape::Straight | SideShape::Spine => vec![start, end],
        SideShape::Traced { samples, .. } => {
            let mut points = Vec::with_capacity(samples.len() + 2);
            points.push(start);
            points.extend(samples.iter().map(|s| s.point));
            points.push(end);
            points
        }
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
    match &outline.sides[side.index()] {
        SideShape::Traced { curved, .. } => *curved,
        SideShape::Bent { .. } => true,
        _ => false,
    }
}

/// The bend handles of a side, in photo pixels: the side's own handles when it is bent, and the
/// points at a quarter, half and three quarters of its traced curve otherwise.
pub fn bend_handles(outline: &OutlineView, side: Side) -> Vec<Point> {
    match &outline.sides[side.index()] {
        SideShape::Bent { handles } => handles.to_vec(),
        SideShape::Traced { .. } => {
            let points = side_points(outline, side);
            points_at_fractions(&points, &[0.25, 0.5, 0.75])
        }
        _ => Vec::new(),
    }
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

/// The interior of the outline, as a closed polygon in photo points: the fitted outline when the
/// page is flattened, and the four corners with their curves otherwise.
pub fn outline_polygon(outline: &OutlineView) -> Vec<Point> {
    if let Some(fitted) = &outline.fitted
        && fitted.len() >= 3
    {
        return fitted.clone();
    }
    let top = side_points(outline, Side::Top);
    let right = side_points(outline, Side::Right);
    let bottom = side_points(outline, Side::Bottom);
    let left = side_points(outline, Side::Left);
    let mut points = Vec::new();
    points.extend(top);
    points.extend(right.into_iter().skip(1));
    points.extend(bottom.into_iter().rev().skip(1));
    points.extend(left.into_iter().rev().skip(1));
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

/// Draws the canvas: the background, the display copy, the scrim outside the outline and the
/// outline itself.
pub fn draw_canvas(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) {
    theme::canvas_background(ui, canvas.rect.expand(PADDING));
    let Some(texture) = state.texture(ui, engine, scancraft_core::engine_api::ImageKey::DisplayCopy(crop.photo), egui::TextureOptions::LINEAR) else {
        return;
    };
    let id = texture.id();
    let painter = ui.painter_at(canvas.rect);
    // The photo, then the scrim over it, then the photo again inside the outline: the page stays
    // at full brightness and everything outside it is dimmed (§4.3).
    painter.image(id, canvas.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    painter.rect_filled(canvas.rect, 0.0, theme::SCRIM);
    for outline in &crop.outlines {
        draw_photo_inside(&painter, canvas, outline, id);
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

/// Draws the page outline: a 2 pt Scan Cyan line over a 1 pt black halo at 50 % (§4.3).
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

/// The outline's path in photo pixels: the fitted outline when the page is flattened, and the
/// four sides otherwise.
pub fn outline_path(outline: &OutlineView) -> Vec<Point> {
    if let Some(fitted) = &outline.fitted
        && fitted.len() >= 3
    {
        return fitted.clone();
    }
    let mut points = Vec::new();
    for side in Side::ALL {
        let side_points = side_points(outline, side);
        if matches!(outline.sides[side.index()], SideShape::Spine) {
            // The spine is drawn on its own, dashed (§4.3).
            continue;
        }
        let skip = if points.is_empty() { 0 } else { 1 };
        points.extend(side_points.into_iter().skip(skip));
    }
    points
}

/// Draws the handles of one outline: the four corners, the edge handles and the bend handles.
pub fn draw_handles(ui: &Ui, canvas: &Canvas, outline: &OutlineView, state: &mut UiState, engine: &mut dyn EngineApi) {
    let now = ui.input(|i| i.time);
    for (index, corner) in [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft].into_iter().enumerate() {
        let point = quad_point(&outline.corners, index);
        let position = canvas.photo_to_canvas(point);
        draw_corner(ui, canvas, outline.page, corner, point, position, state, engine, now);
    }
    for side in Side::ALL {
        if side_is_curved(outline, side) {
            draw_bend_handles(ui, canvas, outline, side, state, engine, now);
        } else {
            draw_edge_handle(ui, canvas, outline, side, state, engine, now);
        }
    }
}

/// Draws one corner node: a 16 pt white circle with a 2 pt cyan ring (§4.3).
#[allow(clippy::too_many_arguments)]
fn draw_corner(
    ui: &Ui,
    canvas: &Canvas,
    page: scancraft_core::recipe::PageId,
    corner: Corner,
    point: Point,
    position: Pos2,
    state: &mut UiState,
    engine: &mut dyn EngineApi,
    now: f64,
) {
    let painter = ui.painter_at(canvas.rect);
    painter.circle_filled(position, theme::CORNER_RADIUS, theme::HANDLE);
    painter.circle_stroke(position, theme::CORNER_RADIUS + theme::CORNER_RING / 2.0, Stroke::new(theme::CORNER_RING, theme::SCAN_CYAN));
    let id = ui.make_persistent_id(("scancraft.corner", page, corner.index()));
    let hit = Rect::from_center_size(position, Vec2::splat(theme::CORNER_HIT_RADIUS * 2.0));
    let response = ui.interact(hit, id, Sense::click_and_drag());
    let mut dragging = false;
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
        && (pointer - position).length() <= theme::CORNER_HIT_RADIUS
    {
        let photo_point = canvas.canvas_to_photo(pointer);
        dispatch(state, engine, Command::CropSetCorner { page, corner, x: photo_point.x, y: photo_point.y });
        dragging = true;
        state.selection = Some(Selection::Corner { page, corner });
        state.show_loupe(now, pointer);
    }
    if response.hovered() || dragging {
        state.selection = Some(Selection::Corner { page, corner });
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::corner_label(corner, point)));
}

/// Draws the edge handle of a side: a 6 × 22 pt pill at the side's middle (§4.3).
fn draw_edge_handle(ui: &Ui, canvas: &Canvas, outline: &OutlineView, side: Side, state: &mut UiState, engine: &mut dyn EngineApi, now: f64) {
    let middle = side_middle(&outline.corners, side);
    let position = canvas.photo_to_canvas(middle);
    let Some(normal) = outward_normal(&outline.corners, side) else {
        return;
    };
    let painter = ui.painter_at(canvas.rect);
    // The pill lies along the side.
    let along = Pos2::new(normal.y as f32, -normal.x as f32);
    let size = if along.x.abs() > along.y.abs() { Vec2::new(theme::EDGE_HANDLE.y, theme::EDGE_HANDLE.x) } else { theme::EDGE_HANDLE };
    painter.rect_filled(Rect::from_center_size(position, size), size.x / 2.0, theme::HANDLE);
    painter.rect_stroke(
        Rect::from_center_size(position, size),
        size.x / 2.0,
        Stroke::new(theme::CORNER_RING, theme::SCAN_CYAN),
        egui::StrokeKind::Middle,
    );
    let id = ui.make_persistent_id(("scancraft.edge", outline.page, side.index()));
    let hit = Rect::from_center_size(position, Vec2::splat(theme::CORNER_HIT_RADIUS * 2.0));
    let response = ui.interact(hit, id, Sense::click_and_drag());
    if response.double_clicked() {
        // A double-click on a straight side gives it bend handles (§4.3).
        let (a, b) = side.corners();
        let start = quad_point(&outline.corners, a);
        let end = quad_point(&outline.corners, b);
        let points = points_at_fractions(&[start, end], &[0.25, 0.5, 0.75]);
        if points.len() == 3 {
            dispatch(state, engine, Command::CropBendSide { page: outline.page, side, handles: [points[0], points[1], points[2]] });
        }
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let pointer_photo = canvas.canvas_to_photo(pointer);
        let projection = (pointer_photo.x - middle.x) * normal.x + (pointer_photo.y - middle.y) * normal.y;
        // The engine's offset moves the side from where it is now, so the interface sends the
        // change since the last frame.
        let delta = match &state.edge_drag {
            Some(drag) if drag.page == outline.page && drag.side == side => projection - drag.last,
            _ => projection,
        };
        if delta.abs() > 1e-9 {
            dispatch(state, engine, Command::CropMoveSide { page: outline.page, side, offset: delta });
        }
        state.edge_drag = Some(crate::state::EdgeDrag { page: outline.page, side, middle, normal, last: projection });
        state.show_loupe(now, pointer);
    } else {
        state.edge_drag = None;
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, format!("{} side handle", a11y::side_name(side))));
}

/// Draws the three bend handles of a curved side: 10 pt white dots with a 2 pt cyan ring, at a
/// quarter, half and three quarters of the side's length (§4.3).
fn draw_bend_handles(ui: &Ui, canvas: &Canvas, outline: &OutlineView, side: Side, state: &mut UiState, engine: &mut dyn EngineApi, now: f64) {
    let handles = bend_handles(outline, side);
    if handles.len() < 3 {
        return;
    }
    let painter = ui.painter_at(canvas.rect);
    for (index, handle) in handles.iter().enumerate() {
        let position = canvas.photo_to_canvas(*handle);
        painter.circle_filled(position, theme::BEND_RADIUS, theme::HANDLE);
        painter.circle_stroke(position, theme::BEND_RADIUS + theme::CORNER_RING / 2.0, Stroke::new(theme::CORNER_RING, theme::SCAN_CYAN));
        let id = ui.make_persistent_id(("scancraft.bend", outline.page, side.index(), index));
        let hit = Rect::from_center_size(position, Vec2::splat(theme::CORNER_HIT_RADIUS * 2.0));
        let response = ui.interact(hit, id, Sense::click_and_drag());
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let mut moved = handles.clone();
            moved[index] = canvas.canvas_to_photo(pointer);
            dispatch(state, engine, Command::CropBendSide { page: outline.page, side, handles: [moved[0], moved[1], moved[2]] });
            state.selection = Some(Selection::Bend { page: outline.page, side, index });
            state.show_loupe(now, pointer);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::bend_label(side, index)));
        let page = outline.page;
        response.context_menu(|ui| {
            if ui.button("Straighten Side").clicked() {
                dispatch(state, engine, Command::CropStraightenSide { page, side });
            }
        });
    }
}

/// Sends a command to the engine and shows its error as a message (§4.8).
pub fn dispatch(state: &mut UiState, engine: &mut dyn EngineApi, command: Command) {
    if let Err(error) = engine.dispatch(command) {
        state.notice(error, true);
    }
}

/// Draws the spine of a split spread: a dashed Scan Cyan line with a round handle at each end
/// (§4.3).
pub fn draw_spine(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) {
    let Some(spine) = crop.spine else { return };
    let now = ui.input(|i| i.time);
    let painter = ui.painter_at(canvas.rect);
    let a = canvas.photo_to_canvas(spine[0]);
    let b = canvas.photo_to_canvas(spine[1]);
    for shape in Shape::dashed_line(&[a, b], Stroke::new(theme::OUTLINE_WIDTH, theme::SCAN_CYAN), 6.0, 4.0) {
        painter.add(shape);
    }
    for (index, point) in spine.iter().enumerate() {
        let position = canvas.photo_to_canvas(*point);
        painter.circle_filled(position, theme::SPINE_RADIUS, theme::SCAN_CYAN);
        painter.circle_stroke(position, theme::SPINE_RADIUS + 1.0, Stroke::new(2.0, theme::HANDLE));
        let id = ui.make_persistent_id(("scancraft.spine", crop.photo, index));
        let hit = Rect::from_center_size(position, Vec2::splat(theme::CORNER_HIT_RADIUS * 2.0));
        let response = ui.interact(hit, id, Sense::click_and_drag());
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let moved = canvas.canvas_to_photo(pointer);
            let (top, bottom) = if index == 0 { (moved, spine[1]) } else { (spine[0], moved) };
            dispatch(state, engine, Command::CropSetSpine { photo: crop.photo, top, bottom });
            state.selection = Some(Selection::Spine { photo: crop.photo, end: index });
            state.show_loupe(now, pointer);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, a11y::spine_label(index)));
    }
}
