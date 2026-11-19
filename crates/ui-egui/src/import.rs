//! The empty window (§4.2): what the app shows until there are pages. The whole window is the
//! drop target, with one sentence, a Choose Files… button and the formats the app reads; a card
//! offers the last scan that was never exported.

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ViewState};

use crate::canvas::dispatch;
use crate::state::UiState;
use crate::theme;

/// How far the drop zone's border is from the window's edge.
pub const ZONE_MARGIN: f32 = 24.0;
/// The corner radius of the drop zone (§4.2).
pub const CORNER_RADIUS: f32 = 12.0;
/// The width of the drop zone's dashes (§4.2).
pub const DASH_WIDTH: f32 = 2.0;
/// The length of a dash and the gap after it (§4.2).
pub const DASH_LENGTH: f32 = 8.0;
pub const GAP_LENGTH: f32 = 6.0;

/// The formats the app reads (§4.2).
pub const FORMATS: &str = "JPEG, PNG, HEIC/HEIF, PDF, TIFF, WebP";

/// Draws the empty window.
pub fn import_screen(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    let zone = zone_rect(area);
    draw_drop_zone(ui, state, zone);
    // The sentence, the button and the formats, in the middle of the zone.
    let content = Rect::from_center_size(zone.center() - Vec2::new(0.0, 24.0), Vec2::new(zone.width().min(520.0), 150.0));
    ui.scope_builder(UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Center)), |ui| {
        let title = state.text.t("Drop photos or PDFs anywhere in this window");
        ui.label(egui::RichText::new(title).size(20.0));
        ui.add_space(14.0);
        let label = state.text.t("Choose Files…");
        if theme::primary_button(ui, &label, true, 32.0).clicked() {
            state.actions.push(crate::state::UiAction::ImportFiles);
        }
        ui.add_space(12.0);
        let formats = state.text.t(FORMATS);
        ui.label(egui::RichText::new(formats).color(ui.visuals().weak_text_color()));
    });
    if let Some(pages) = view.restore_offer {
        restore_card(ui, state, pages, zone, engine);
    }
}

/// The drop zone's rectangle: the window less a margin, so the border is plain at its edge.
pub fn zone_rect(area: Rect) -> Rect {
    area.shrink(ZONE_MARGIN)
}

/// Draws the dashed rounded rectangle, and tints it while files are held over the window. A click
/// on it opens the Open panel, as the button does.
fn draw_drop_zone(ui: &mut Ui, state: &mut UiState, rect: Rect) {
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    let accent = theme::accent(ui.visuals().dark_mode);
    let colour = if hovering { accent } else { ui.visuals().weak_text_color() };
    let path = rounded_rect_path(rect, CORNER_RADIUS);
    let painter = ui.painter_at(rect.expand(4.0));
    if hovering {
        painter.rect_filled(rect, CORNER_RADIUS, accent.gamma_multiply(0.1));
    }
    for shape in Shape::dashed_line(&path, Stroke::new(DASH_WIDTH, colour), DASH_LENGTH, GAP_LENGTH) {
        painter.add(shape);
    }
    // The zone is where the files land, and a click on it opens the Open panel, the same as the
    // button in it: it is part of the interface a screen reader reads.
    let label = state.text.t("Drop photos or PDFs here");
    let formats = state.text.t(FORMATS);
    let id = ui.make_persistent_id("scancraft.drop-zone");
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}. {formats}")));
    if response.clicked() {
        state.actions.push(crate::state::UiAction::ImportFiles);
    }
}

/// The card that offers the last scan that was never exported: "Continue your last scan (7 pages)?"
fn restore_card(ui: &mut Ui, state: &mut UiState, pages: u32, zone: Rect, engine: &mut dyn EngineApi) {
    let card = Rect::from_center_size(Pos2::new(zone.center().x, zone.max.y - 70.0), Vec2::new(zone.width().min(460.0), 76.0));
    ui.scope_builder(UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Center)), |ui| {
        egui::Frame::group(ui.style()).fill(ui.visuals().faint_bg_color).show(ui, |ui| {
            ui.set_width(card.width() - 24.0);
            let question = state.text.t("Continue your last scan ({n} pages)?").replace("{n}", &pages.to_string());
            ui.label(question);
            ui.horizontal(|ui| {
                let cont = state.text.t("Continue");
                if ui.button(cont).clicked() {
                    dispatch(state, engine, Command::SessionRestore);
                }
                let fresh = state.text.t("Start New");
                if ui.button(fresh).clicked() {
                    dispatch(state, engine, Command::SessionNew);
                }
            });
        });
    });
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
