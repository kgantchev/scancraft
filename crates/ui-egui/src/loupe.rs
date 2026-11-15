//! The magnifier loupe of Screen 2 (§4.3).
//!
//! It appears while a node, an edge handle, a bend handle or a spine end is pressed or dragged,
//! and for a second after a keyboard nudge. It is a 140 pt circle showing the photo at three times
//! the canvas zoom, sampled from the full-resolution photo.

use egui::{Color32, Mesh, Painter, Pos2, Shape, Stroke, Ui, Vec2};
use scancraft_core::engine_api::{CropView, EngineApi, LoupeCrop, OutlineView};
use scancraft_core::geom::{Point, Side};
use scancraft_core::recipe::SideShape;

use crate::canvas::Canvas;
use crate::state::{Selection, UiState};
use crate::theme;

/// The loupe's diameter (§4.3).
pub const DIAMETER: f32 = 140.0;
/// The loupe's radius.
pub const RADIUS: f32 = DIAMETER / 2.0;
/// How far the loupe's centre sits from the pointer (§4.3).
pub const OFFSET: f32 = 110.0;
/// How close to the window's top the pointer has to be for the loupe to sit below it (§4.3).
pub const FLIP_DISTANCE: f32 = 130.0;
/// The magnification over the canvas zoom (§4.3).
pub const MAGNIFICATION: f64 = 3.0;
/// The largest crop the engine is asked for (§6.5).
pub const MAX_CROP: f64 = 1024.0;
/// The gap in the middle of the crosshair (§4.3).
pub const CROSSHAIR_GAP: f32 = 3.0;
/// The number of segments the loupe's circle is drawn with (§6.5).
pub const SEGMENTS: usize = 64;

/// Where the loupe is and how much it magnifies.
#[derive(Debug, Clone, Copy)]
pub struct Loupe {
    /// The circle's centre, in window points.
    pub centre: Pos2,
    /// Points per full-resolution photo pixel inside the loupe.
    pub points_per_photo_px: f64,
}

/// The centre of the loupe for a pointer at `pointer`, inside the window `window` (§4.3).
pub fn loupe_centre(pointer: Pos2, window: egui::Rect) -> Pos2 {
    let below = pointer.y - window.min.y < FLIP_DISTANCE;
    let y = if below { pointer.y + OFFSET } else { pointer.y - OFFSET };
    let x = pointer.x.clamp(window.min.x + RADIUS, window.max.x - RADIUS);
    let y = y.clamp(window.min.y + RADIUS, window.max.y - RADIUS);
    Pos2::new(x, y)
}

/// The side of the crop the engine is asked for: 1.5 times what the loupe shows, at most 1024
/// photo pixels (§6.5).
pub fn crop_side(points_per_photo_px: f64) -> f64 {
    if points_per_photo_px <= 0.0 {
        return MAX_CROP;
    }
    let across = f64::from(DIAMETER) / points_per_photo_px;
    (1.5 * across).min(MAX_CROP)
}

/// Draws the loupe, when it is showing.
pub fn draw(ui: &Ui, canvas: &Canvas, crop: &CropView, state: &mut UiState, engine: &mut dyn EngineApi) {
    let now = ui.input(|i| i.time);
    if !state.loupe_visible(now) {
        return;
    }
    let Some(pointer) = state.loupe_centre else {
        return;
    };
    let window = ui.ctx().viewport_rect();
    let centre = loupe_centre(pointer, window);
    let points_per_photo_px = canvas.points_per_photo_px() / MAGNIFICATION;
    if points_per_photo_px <= 0.0 {
        return;
    }
    let centre_photo = canvas.canvas_to_photo(pointer);
    let side = crop_side(points_per_photo_px);
    let Some(loupe_crop) = engine.loupe(crop.photo, centre_photo, side) else {
        return;
    };
    let loupe = Loupe { centre, points_per_photo_px };
    let Some(handle) = state.loupe_texture(ui, &loupe_crop) else {
        return;
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("scancraft.loupe")));
    draw_circle(&painter, &loupe, &loupe_crop, centre_photo, handle.id());
    draw_crosshair(&painter, &loupe);
    draw_sides(&painter, &loupe, &loupe_crop, centre_photo, crop, state);
}

/// Where a photo position falls in the crop's texture, as the 0–1 texture coordinates a mesh
/// takes. The crop's own pixel position is divided by the crop's size: used as pixels, the
/// coordinates all lay far outside the texture, and the loupe showed one clamped colour.
fn crop_uv(crop: &LoupeCrop, photo: Point) -> Pos2 {
    let cx = (photo.x - crop.origin.x) / crop.scale;
    let cy = (photo.y - crop.origin.y) / crop.scale;
    let (w, h) = (f64::from(crop.image.width().max(1)), f64::from(crop.image.height().max(1)));
    Pos2::new((cx / w) as f32, (cy / h) as f32)
}

/// Draws the loupe's circle: the crop's pixels, magnified, in a fan of 64 triangles.
fn draw_circle(painter: &Painter, loupe: &Loupe, crop: &LoupeCrop, centre_photo: Point, texture: egui::TextureId) {
    let mut mesh = Mesh::with_texture(texture);
    let uv_of = |photo: Point| crop_uv(crop, photo);
    mesh.vertices.push(egui::epaint::Vertex { pos: loupe.centre, uv: uv_of(centre_photo), color: Color32::WHITE });
    for i in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
        let offset = Vec2::new(angle.cos(), angle.sin()) * RADIUS;
        let pos = loupe.centre + offset;
        let photo = Point::new(
            centre_photo.x + f64::from(offset.x) / loupe.points_per_photo_px,
            centre_photo.y + f64::from(offset.y) / loupe.points_per_photo_px,
        );
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: uv_of(photo), color: Color32::WHITE });
    }
    for i in 0..SEGMENTS {
        mesh.indices.push(0);
        mesh.indices.push(i as u32 + 1);
        mesh.indices.push(i as u32 + 2);
    }
    if mesh.indices.is_empty() {
        return;
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the crosshair: 1 pt lines with a 6 pt gap, centred on the exact position.
fn draw_crosshair(painter: &Painter, loupe: &Loupe) {
    let c = loupe.centre;
    let stroke = Stroke::new(1.0, Color32::WHITE);
    for (from, to) in [
        (Pos2::new(c.x - RADIUS, c.y), Pos2::new(c.x - CROSSHAIR_GAP, c.y)),
        (Pos2::new(c.x + CROSSHAIR_GAP, c.y), Pos2::new(c.x + RADIUS, c.y)),
        (Pos2::new(c.x, c.y - RADIUS), Pos2::new(c.x, c.y - CROSSHAIR_GAP)),
        (Pos2::new(c.x, c.y + CROSSHAIR_GAP), Pos2::new(c.x, c.y + RADIUS)),
    ] {
        painter.add(Shape::line_segment([from, to], stroke));
    }
}

/// Draws the two sides that meet at the corner, or the curve or the spine through a handle, in
/// the same 2 pt Scan Cyan over the 1 pt black halo (§4.3).
fn draw_sides(painter: &Painter, loupe: &Loupe, _crop: &LoupeCrop, centre_photo: Point, view: &CropView, state: &UiState) {
    let Some(selection) = state.selection else {
        return;
    };
    let mut paths: Vec<Vec<Point>> = Vec::new();
    match selection {
        Selection::Corner { page, corner } => {
            let Some(outline) = view.outlines.iter().find(|o| o.page == page) else {
                return;
            };
            let index = corner.index();
            let sides = [Side::Top, Side::Right, Side::Bottom, Side::Left];
            for side in sides {
                let (a, b) = side.corners();
                if a == index || b == index {
                    paths.push(crate::canvas::side_points(outline, side));
                }
            }
        }
        Selection::Bend { page, side, .. } => {
            let Some(outline) = view.outlines.iter().find(|o| o.page == page) else {
                return;
            };
            paths.push(crate::canvas::side_points(outline, side));
        }
        Selection::Spine { photo, .. } => {
            if view.photo != photo {
                return;
            }
            if let Some(spine) = view.spine {
                paths.push(vec![spine[0], spine[1]]);
            }
        }
    }
    for path in paths {
        let points: Vec<Pos2> = path.iter().map(|p| loupe_point(loupe, centre_photo, *p)).collect();
        if points.len() < 2 {
            continue;
        }
        painter.add(Shape::line(points.clone(), Stroke::new(theme::HALO_WIDTH, theme::HALO)));
        painter.add(Shape::line(points, Stroke::new(theme::OUTLINE_WIDTH, theme::SCAN_CYAN)));
    }
}

/// A photo position in the loupe's points.
fn loupe_point(loupe: &Loupe, centre_photo: Point, photo: Point) -> Pos2 {
    loupe.centre
        + Vec2::new(((photo.x - centre_photo.x) * loupe.points_per_photo_px) as f32, ((photo.y - centre_photo.y) * loupe.points_per_photo_px) as f32)
}

/// Whether a side is drawn dashed: the spine of a spread (§4.3).
pub fn is_spine(outline: &OutlineView, side: Side) -> bool {
    matches!(outline.sides[side.index()], SideShape::Spine)
}

#[cfg(test)]
mod tests {
    use super::crop_uv;
    use scancraft_core::engine_api::LoupeCrop;
    use scancraft_core::geom::Point;
    use scancraft_core::image::Plane;
    use std::sync::Arc;

    #[test]
    fn photo_positions_map_to_zero_to_one_texture_coordinates() {
        // A 200 × 100 crop whose top-left pixel is photo (10, 20), two photo pixels to a crop pixel.
        let crop = LoupeCrop { image: Arc::new(Plane::new(200, 100, [0u8; 3]).unwrap()), origin: Point::new(10.0, 20.0), scale: 2.0 };
        let at = |x: f64, y: f64| crop_uv(&crop, Point::new(x, y));
        assert_eq!(at(10.0, 20.0), egui::pos2(0.0, 0.0));
        assert_eq!(at(210.0, 120.0), egui::pos2(0.5, 0.5));
        assert_eq!(at(410.0, 220.0), egui::pos2(1.0, 1.0));
    }
}
