//! Screen 1, the Import Hub (§4.2): the drop zone, the Browse Files… button and the restore
//! banner.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ViewState};

use crate::canvas::dispatch;
use crate::state::UiState;
use crate::theme;

/// The share of the content area the drop zone fills (§4.2).
pub const DROP_ZONE_SHARE: f32 = 0.8;
/// The corner radius of the drop zone (§4.2).
pub const CORNER_RADIUS: f32 = 12.0;
/// The width of the drop zone's dashes (§4.2).
pub const DASH_WIDTH: f32 = 2.0;
/// The length of a dash and the gap after it (§4.2).
pub const DASH_LENGTH: f32 = 8.0;
pub const GAP_LENGTH: f32 = 6.0;

/// The formats the app reads (§4.2).
pub const FORMATS: &str = "JPEG, PNG, HEIC/HEIF, PDF, TIFF, WebP";

/// Draws Screen 1.
pub fn import_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    if let Some(pages) = view.restore_offer {
        restore_banner(ui, state, pages, engine);
    }
    let area = ui.available_rect_before_wrap();
    let drop_zone = drop_zone_rect(area);
    draw_drop_zone(ui, state, drop_zone);
    ui.allocate_space(Vec2::new(0.0, 12.0));
    browse_button(ui, state, drop_zone);
}

/// The drop zone's rectangle: 80 % of the content area, centred (§4.2).
pub fn drop_zone_rect(area: Rect) -> Rect {
    let size = area.size() * DROP_ZONE_SHARE;
    Rect::from_min_size(area.min + (area.size() - size) / 2.0, size)
}

/// Draws the dashed rounded rectangle, its label and the formats underneath.
fn draw_drop_zone(ui: &mut Ui, state: &mut UiState, rect: Rect) {
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    let accent = theme::accent(ui.visuals().dark_mode);
    let colour = if hovering { accent } else { ui.visuals().weak_text_color() };
    let path = rounded_rect_path(rect, CORNER_RADIUS);
    let painter = ui.painter_at(rect);
    if hovering {
        painter.rect_filled(rect, CORNER_RADIUS, accent.gamma_multiply(0.1));
    }
    for shape in Shape::dashed_line(&path, Stroke::new(DASH_WIDTH, colour), DASH_LENGTH, GAP_LENGTH) {
        painter.add(shape);
    }
    let label = state.text.t("Drop photos or PDFs here");
    let formats = state.text.t(FORMATS);
    let title = FontId::proportional(20.0);
    let body = FontId::proportional(14.0);
    let title_rect = painter.text(rect.center() - Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, label.clone(), title, ui.visuals().text_color());
    painter.text(rect.center() + Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, formats.clone(), body, ui.visuals().weak_text_color());
    // The zone is where the files land, and a click on it opens the Open panel, the same as the
    // button under it: it is part of the interface a screen reader reads.
    let id = ui.make_persistent_id("scancraft.drop-zone");
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}. {formats}")));
    if response.clicked() {
        state.actions.push(crate::state::UiAction::ImportFiles);
    }
    let _ = title_rect;
}

/// The Browse Files… button: the app shell opens the Open panel (§4.2).
fn browse_button(ui: &mut Ui, state: &mut UiState, drop_zone: Rect) {
    let label = state.text.t("Browse Files…");
    let width = 120.0;
    let rect = Rect::from_min_size(Pos2::new(drop_zone.center().x - width / 2.0, drop_zone.max.y + 12.0), Vec2::new(width, 28.0));
    let response = ui.interact(rect, ui.make_persistent_id("scancraft.browse"), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, theme::COPPER);
    painter.text(rect.center(), Align2::CENTER_CENTER, label.clone(), egui::FontId::proportional(14.0), Color32::WHITE);
    if response.clicked() {
        state.actions.push(crate::state::UiAction::ImportFiles);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
}

/// The restore banner: "Continue your last scan (7 pages)?" (§4.2).
fn restore_banner(ui: &mut Ui, state: &mut UiState, pages: u32, engine: &mut dyn EngineApi) {
    let question = state.text.t(&format!("Continue your last scan ({pages} pages)?"));
    ui.horizontal(|ui| {
        ui.label(question);
        let cont = state.text.t("Continue");
        if ui.button(cont).clicked() {
            dispatch(state, engine, Command::SessionRestore);
        }
        let fresh = state.text.t("Start New");
        if ui.button(fresh).clicked() {
            dispatch(state, engine, Command::SessionNew);
        }
    });
    ui.add_space(8.0);
}

/// The path of a rounded rectangle, as a polyline: the drop zone's dashes follow it (§4.2).
pub fn rounded_rect_path(rect: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let mut points = Vec::new();
    if radius <= 0.0 {
        points.push(rect.left_top());
        points.push(rect.right_top());
        points.push(rect.right_bottom());
        points.push(rect.left_bottom());
        points.push(rect.left_top());
        return points;
    }
    let steps = 8;
    let half = std::f32::consts::FRAC_PI_2;
    // Each corner's arc, in the order the border runs: the top right, the bottom right, the
    // bottom left and the top left.
    let corners = [
        (Pos2::new(rect.right() - radius, rect.top() + radius), -half, 0.0),
        (Pos2::new(rect.right() - radius, rect.bottom() - radius), 0.0, half),
        (Pos2::new(rect.left() + radius, rect.bottom() - radius), half, std::f32::consts::PI),
        (Pos2::new(rect.left() + radius, rect.top() + radius), std::f32::consts::PI, std::f32::consts::PI * 1.5),
    ];
    for (centre, from, to) in corners {
        for i in 0..=steps {
            let angle = from + (to - from) * i as f32 / steps as f32;
            points.push(centre + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(points[0]);
    points
}

/// The colour of the drop zone's border.
pub fn drop_zone_colour(ui: &Ui, hovering: bool) -> Color32 {
    if hovering { theme::accent(ui.visuals().dark_mode) } else { ui.visuals().weak_text_color() }
}
