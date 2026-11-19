//! Thumbnails at their own proportions.
//!
//! A thumbnail is drawn inside a box of a fixed size, so the rows of a list line up, but the
//! picture in it keeps its shape: a wide page is wide in its box, and a narrow one is narrow.
//! Stretching every picture to fill the box made a landscape page look like a portrait one.

use egui::{Color32, Painter, Pos2, Rect, TextureHandle, Vec2};

/// The largest rectangle with the proportions of a `size` image that fits inside `bounds`,
/// centred in it. An image with no size, or a box with none, gives an empty rectangle at the
/// middle of the box.
pub fn fit_inside(bounds: Rect, size: [usize; 2]) -> Rect {
    let (width, height) = (size[0] as f32, size[1] as f32);
    if width <= 0.0 || height <= 0.0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return Rect::from_center_size(bounds.center(), Vec2::ZERO);
    }
    let scale = (bounds.width() / width).min(bounds.height() / height);
    Rect::from_center_size(bounds.center(), Vec2::new(width * scale, height * scale))
}

/// Draws `texture` at its own proportions, centred in `bounds`, and returns where it went.
pub fn paint_fitted(painter: &Painter, bounds: Rect, texture: &TextureHandle) -> Rect {
    let rect = fit_inside(bounds, texture.size());
    painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(64.0, 88.0))
    }

    #[test]
    fn a_wide_picture_is_wide_in_a_tall_box() {
        let rect = fit_inside(boxed(), [200, 100]);
        assert!((rect.width() - 64.0).abs() < 1e-4 && (rect.height() - 32.0).abs() < 1e-4, "{rect:?}");
        assert_eq!(rect.center(), boxed().center());
    }

    #[test]
    fn a_narrow_picture_is_narrow_in_a_wide_box() {
        let rect = fit_inside(Rect::from_min_size(Pos2::ZERO, Vec2::new(112.0, 150.0)), [100, 400]);
        assert!((rect.height() - 150.0).abs() < 1e-4 && (rect.width() - 37.5).abs() < 1e-4, "{rect:?}");
    }

    #[test]
    fn a_picture_the_shape_of_the_box_fills_it() {
        let rect = fit_inside(boxed(), [128, 176]);
        assert!((rect.width() - 64.0).abs() < 1e-3 && (rect.height() - 88.0).abs() < 1e-3, "{rect:?}");
    }

    #[test]
    fn an_empty_picture_draws_nothing() {
        assert_eq!(fit_inside(boxed(), [0, 100]).size(), Vec2::ZERO);
        assert_eq!(fit_inside(boxed(), [100, 0]).size(), Vec2::ZERO);
    }
}
