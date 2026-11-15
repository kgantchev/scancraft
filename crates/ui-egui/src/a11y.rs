//! The accessible names of the interface (§4.9).
//!
//! Every control carries an AccessKit role and a label, so VoiceOver can read and operate it.
//! The corners, the bend handles and the spine's ends are adjustable elements: they carry a value
//! as well as a name.

use scancraft_core::command::Corner;
use scancraft_core::geom::{Point, Side};

/// The name of a corner: "Top-left", "Top-right", "Bottom-right" or "Bottom-left".
pub fn corner_name(corner: Corner) -> &'static str {
    match corner {
        Corner::TopLeft => "Top-left",
        Corner::TopRight => "Top-right",
        Corner::BottomRight => "Bottom-right",
        Corner::BottomLeft => "Bottom-left",
    }
}

/// The name of a side: "Top", "Right", "Bottom" or "Left".
pub fn side_name(side: Side) -> &'static str {
    match side {
        Side::Top => "Top",
        Side::Right => "Right",
        Side::Bottom => "Bottom",
        Side::Left => "Left",
    }
}

/// The name of a corner and its position in photo pixels: "Top-left corner, 412, 380".
pub fn corner_label(corner: Corner, p: Point) -> String {
    format!("{} corner, {}, {}", corner_name(corner), p.x.round() as i64, p.y.round() as i64)
}

/// The name of a bend handle: "Top side, bend handle 2 of 3".
pub fn bend_label(side: Side, index: usize) -> String {
    format!("{} side, bend handle {} of 3", side_name(side), index + 1)
}

/// The name of a spine end: "Spine, top end" or "Spine, bottom end".
pub fn spine_label(end: usize) -> String {
    if end == 0 { "Spine, top end".to_owned() } else { "Spine, bottom end".to_owned() }
}

/// The role of an adjustable element: a slider, the closest AccessKit role to a corner or a
/// handle that can be moved.
pub fn adjustable_role() -> egui::Role {
    egui::Role::Slider
}
