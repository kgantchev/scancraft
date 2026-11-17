//! The toolbar's icons, drawn with painter shapes.
//!
//! They are drawn, not typed or loaded: the font of the system has glyphs for some of them, but
//! the font of the test harness may not, and a missing glyph is a panic; and an image would be an
//! asset to attribute. Each takes the rectangle of the button it is in and the colour to draw in,
//! which is what `craft_ui::buttons::IconButton` hands its painter.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// The stroke every icon is drawn with.
fn pen(colour: Color32) -> Stroke {
    Stroke::new(1.6, colour)
}

/// The square the icon is drawn in: 60 % of the button, centred.
fn square(rect: Rect) -> Rect {
    let side = rect.width().min(rect.height()) * 0.6;
    Rect::from_center_size(rect.center(), Vec2::splat(side))
}

/// A plus: Add.
pub fn plus(painter: &Painter, rect: Rect, colour: Color32) {
    let r = square(rect);
    let c = r.center();
    let half = r.width() / 2.0;
    painter.line_segment([c - Vec2::new(half, 0.0), c + Vec2::new(half, 0.0)], pen(colour));
    painter.line_segment([c - Vec2::new(0.0, half), c + Vec2::new(0.0, half)], pen(colour));
}

/// A three-quarter circle with an arrowhead at its end, for a turn. `clockwise` turns the arrow
/// the way the hands of a clock go.
fn turn(painter: &Painter, rect: Rect, colour: Color32, clockwise: bool) {
    let r = square(rect);
    let c = r.center();
    let radius = r.width() / 2.0;
    // The gap is at the top; the arc runs from just right of it, round the bottom, to just left.
    let (start, sweep) = if clockwise { (-60.0f32, 300.0f32) } else { (-120.0f32, -300.0f32) };
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let angle = (start + sweep * i as f32 / 24.0).to_radians();
            c + Vec2::new(angle.cos(), angle.sin()) * radius
        })
        .collect();
    let end = points.last().copied().unwrap_or(c);
    painter.add(Shape::line(points, pen(colour)));
    // The head: a small triangle on the end of the arc, pointing along it.
    let end_angle = (start + sweep).to_radians();
    let tangent = if clockwise { Vec2::new(-end_angle.sin(), end_angle.cos()) } else { Vec2::new(end_angle.sin(), -end_angle.cos()) };
    let normal = Vec2::new(-tangent.y, tangent.x);
    let tip = end + tangent * 3.0;
    painter.add(Shape::convex_polygon(vec![tip, end + normal * 3.4, end - normal * 3.4], colour, Stroke::NONE));
}

/// A turn to the left.
pub fn rotate_left(painter: &Painter, rect: Rect, colour: Color32) {
    turn(painter, rect, colour, false);
}

/// A turn to the right.
pub fn rotate_right(painter: &Painter, rect: Rect, colour: Color32) {
    turn(painter, rect, colour, true);
}

/// A bin: Delete.
pub fn trash(painter: &Painter, rect: Rect, colour: Color32) {
    let r = square(rect);
    let lid_y = r.top() + r.height() * 0.2;
    painter.line_segment([Pos2::new(r.left(), lid_y), Pos2::new(r.right(), lid_y)], pen(colour));
    let handle = Rect::from_center_size(Pos2::new(r.center().x, r.top() + r.height() * 0.08), Vec2::new(r.width() * 0.34, r.height() * 0.12));
    painter.rect_stroke(handle, 1.0, pen(colour), egui::StrokeKind::Middle);
    let body = Rect::from_min_max(Pos2::new(r.left() + r.width() * 0.12, lid_y), Pos2::new(r.right() - r.width() * 0.12, r.bottom()));
    painter.rect_stroke(body, 2.0, pen(colour), egui::StrokeKind::Middle);
    for x in [0.38, 0.62] {
        let x = body.left() + body.width() * x;
        painter.line_segment([Pos2::new(x, body.top() + 3.0), Pos2::new(x, body.bottom() - 3.0)], Stroke::new(1.2, colour));
    }
}

/// An arrow that bends back to the left: Undo. `mirrored` bends it the other way: Redo.
fn bend_back(painter: &Painter, rect: Rect, colour: Color32, mirrored: bool) {
    let r = square(rect);
    let flip = |p: Pos2| if mirrored { Pos2::new(2.0 * rect.center().x - p.x, p.y) } else { p };
    let c = r.center();
    let radius = r.width() * 0.42;
    // A hook: the arc goes right, down and round, and ends pointing left.
    let points: Vec<Pos2> = (0..=18)
        .map(|i| {
            let angle = (-90.0f32 + 200.0 * i as f32 / 18.0).to_radians();
            flip(Pos2::new(c.x + 1.0 + angle.cos() * radius, c.y + 3.0 + angle.sin() * radius))
        })
        .collect();
    painter.add(Shape::line(points, pen(colour)));
    let tip = flip(Pos2::new(r.left() + 1.0, r.top() + r.height() * 0.32));
    let (up, down) =
        (flip(Pos2::new(r.left() + 7.0, r.top() + r.height() * 0.32 - 4.5)), flip(Pos2::new(r.left() + 7.0, r.top() + r.height() * 0.32 + 4.5)));
    painter.add(Shape::convex_polygon(vec![tip, up, down], colour, Stroke::NONE));
}

/// Undo.
pub fn undo(painter: &Painter, rect: Rect, colour: Color32) {
    bend_back(painter, rect, colour, false);
}

/// Redo.
pub fn redo(painter: &Painter, rect: Rect, colour: Color32) {
    bend_back(painter, rect, colour, true);
}

/// A small downward chevron: the export options.
pub fn chevron_down(painter: &Painter, rect: Rect, colour: Color32) {
    let c = rect.center();
    painter.line_segment([c + Vec2::new(-4.5, -2.0), c + Vec2::new(0.0, 2.5)], pen(colour));
    painter.line_segment([c + Vec2::new(0.0, 2.5), c + Vec2::new(4.5, -2.0)], pen(colour));
}

/// Three dots: the toolbar's overflow.
pub fn more(painter: &Painter, rect: Rect, colour: Color32) {
    let c = rect.center();
    for dx in [-5.0, 0.0, 5.0] {
        painter.circle_filled(c + Vec2::new(dx, 0.0), 1.6, colour);
    }
}

/// A panel with a column at its right: the inspector's switch.
pub fn inspector(painter: &Painter, rect: Rect, colour: Color32) {
    let r = square(rect);
    painter.rect_stroke(r, 2.0, pen(colour), egui::StrokeKind::Middle);
    let x = r.right() - r.width() * 0.34;
    painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], pen(colour));
}
