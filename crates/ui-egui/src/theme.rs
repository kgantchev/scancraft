//! ScanCraft's colours (§4.1): the three brand colours, the fixed canvas background and the
//! palette they make.
//!
//! The colours are sRGB bytes, and this fork of egui keeps `Color32` in gamma space, so the hex
//! values of the specification map straight across.

use craft_ui::theme::WidgetPalette;
use egui::{Color32, Context, CornerRadius, Stroke, Ui, Vec2};

/// Copper `#A84128`: the primary buttons.
pub const COPPER: Color32 = Color32::from_rgb(0xA8, 0x41, 0x28);
/// Light Copper `#D87840`: the hover and pressed states.
pub const LIGHT_COPPER: Color32 = Color32::from_rgb(0xD8, 0x78, 0x40);
/// Scan Cyan `#02FDFE`: the page outline and the handles, on the dark canvas.
pub const SCAN_CYAN: Color32 = Color32::from_rgb(0x02, 0xFD, 0xFE);
/// The canvas background `#1C1C1E`, the same in both appearances.
pub const CANVAS_BACKGROUND: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x1E);
/// The 45 % black scrim outside the page outline.
pub const SCRIM: Color32 = Color32::from_black_alpha(115);
/// The 50 % black halo under the outline's line.
pub const HALO: Color32 = Color32::from_black_alpha(128);
/// The 8 % cyan tint the drop zone takes while files hover over the window (§4.2).
pub const CYAN_TINT: Color32 = Color32::from_rgba_unmultiplied_const(0x02, 0xFD, 0xFE, 20);
/// The red an invalid outline turns (§4.3).
pub const INVALID: Color32 = Color32::from_rgb(0xD9, 0x2B, 0x2B);
/// The white of the corner nodes and the bend handles (§4.3).
pub const HANDLE: Color32 = Color32::WHITE;

/// The width of the page outline's line (§4.3).
pub const OUTLINE_WIDTH: f32 = 2.0;
/// The width of the halo under it, wider than the line so the dark edge shows on both sides of it
/// (§4.3).
pub const HALO_WIDTH: f32 = 4.0;
/// The radius of a corner node's white circle (§4.3).
pub const CORNER_RADIUS: f32 = 8.0;
/// The width of a corner node's cyan ring (§4.3).
pub const CORNER_RING: f32 = 2.0;
/// The radius of a corner node's hit area (§4.3).
pub const CORNER_HIT_RADIUS: f32 = 22.0;
/// The radius of a bend handle's white dot (§4.3).
pub const BEND_RADIUS: f32 = 5.0;
/// The size of an edge handle, a 6 × 22 pt pill (§4.3).
pub const EDGE_HANDLE: Vec2 = Vec2::new(6.0, 22.0);
/// The radius of the spine's end handles (§4.3).
pub const SPINE_RADIUS: f32 = 10.0;

/// The neutral fill of a control at rest, under the pointer and pressed. Controls are neutral in
/// both appearances, so the one copper button of a view is the one that moves on.
fn neutral_fills(dark: bool) -> (Color32, Color32, Color32) {
    if dark {
        (Color32::from_rgb(0x3A, 0x3A, 0x3D), Color32::from_rgb(0x48, 0x48, 0x4C), Color32::from_rgb(0x55, 0x55, 0x5A))
    } else {
        (Color32::from_rgb(0xE2, 0xE2, 0xE5), Color32::from_rgb(0xD2, 0xD2, 0xD7), Color32::from_rgb(0xC2, 0xC2, 0xC8))
    }
}

/// The colour that marks a selection or the keyboard focus on the window's own background: Copper
/// on the light one, where Scan Cyan can hardly be seen, and Light Copper on the dark one. Scan
/// Cyan stays on the dark canvas.
pub fn accent(dark: bool) -> Color32 {
    if dark { LIGHT_COPPER } else { COPPER }
}

/// Pushes ScanCraft's palette into the context's style: neutral controls, a copper selection and a
/// copper focus ring.
///
/// Every control rests in a neutral grey, so an unticked checkbox or an unselected radio button
/// does not look selected, and copper is left to the one primary button of a view
/// ([`nav_button`]). The palette sets every widget state's fill, and the non-interactive state is
/// also the window's own background, so that one is put back the way egui had it, and so is every
/// state's text colour.
pub fn apply(ctx: &Context) {
    let theme = ctx.theme();
    let dark = theme == egui::Theme::Dark;
    let panel = if dark { Color32::from_rgb(0x25, 0x25, 0x27) } else { Color32::from_rgb(0xF2, 0xF2, 0xF2) };
    let (rest, hover, pressed) = neutral_fills(dark);
    let palette = WidgetPalette { noninteractive: panel, inactive: rest, hovered: hover, active: pressed, open: rest };
    let defaults = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let accent = accent(dark);
    ctx.style_mut_of(theme, |style| {
        let window = style.visuals.widgets.noninteractive;
        palette.apply_to(&mut style.visuals.widgets, CornerRadius::same(6), Stroke::new(1.0, Color32::WHITE));
        style.visuals.widgets.noninteractive = window;
        let widgets = &mut style.visuals.widgets;
        widgets.inactive.fg_stroke = defaults.widgets.inactive.fg_stroke;
        widgets.hovered.fg_stroke = defaults.widgets.hovered.fg_stroke;
        widgets.active.fg_stroke = defaults.widgets.active.fg_stroke;
        widgets.open.fg_stroke = defaults.widgets.open.fg_stroke;
        widgets.inactive.bg_stroke = Stroke::NONE;
        // The pressed and the focused state: the ring the keyboard's focus shows.
        widgets.active.bg_stroke = Stroke::new(2.0, accent);
        widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
        // A selection is a tint of copper with copper ink: legible on both backgrounds.
        style.visuals.selection.stroke = Stroke::new(1.5, accent);
        style.visuals.selection.bg_fill = accent.gamma_multiply(if dark { 0.3 } else { 0.18 });
        style.visuals.hyperlink_color = accent;
    });
}

/// Paints the canvas background over `rect`: a fixed dark neutral in both appearances (§4.1).
pub fn canvas_background(ui: &Ui, rect: egui::Rect) {
    ui.painter().rect_filled(rect, 0.0, CANVAS_BACKGROUND);
}

/// The style of the icon-only buttons: Copper, with the focus ring in Scan Cyan.
pub fn icon_button_style() -> craft_ui::buttons::IconButtonStyle {
    let fill = Stroke::new(1.0, Color32::TRANSPARENT);
    craft_ui::buttons::IconButtonStyle {
        normal: craft_ui::buttons::ButtonVisuals { fill: COPPER, stroke: fill, icon: Color32::WHITE },
        hovered: craft_ui::buttons::ButtonVisuals { fill: LIGHT_COPPER, stroke: fill, icon: Color32::WHITE },
        pressed: Some(craft_ui::buttons::ButtonVisuals { fill: LIGHT_COPPER, stroke: fill, icon: Color32::WHITE }),
        selected: craft_ui::buttons::ButtonVisuals { fill: LIGHT_COPPER, stroke: fill, icon: Color32::WHITE },
        corner_radius: CornerRadius::same(6),
        focus_stroke: Stroke::new(2.0, SCAN_CYAN),
    }
}

/// Which way an arrow points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    /// To the left.
    Left,
    /// To the right.
    Right,
    /// Downwards.
    Down,
}

/// A button with an arrow drawn in it.
///
/// The arrows are drawn, not typed: the font of the system has the glyphs, but the font of the
/// test harness may not, and a missing glyph is a panic.
pub fn arrow_button(ui: &mut Ui, label: &str, pointing: Arrow) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 24.0), egui::Sense::click());
    let centre = rect.center();
    let (w, h) = (5.0f32, 7.0f32);
    let points = match pointing {
        Arrow::Left => {
            vec![egui::Pos2::new(centre.x + w, centre.y - h), egui::Pos2::new(centre.x + w, centre.y + h), egui::Pos2::new(centre.x - w, centre.y)]
        }
        Arrow::Right => {
            vec![egui::Pos2::new(centre.x - w, centre.y - h), egui::Pos2::new(centre.x - w, centre.y + h), egui::Pos2::new(centre.x + w, centre.y)]
        }
        Arrow::Down => {
            vec![egui::Pos2::new(centre.x - w, centre.y - h), egui::Pos2::new(centre.x + w, centre.y - h), egui::Pos2::new(centre.x, centre.y + h)]
        }
    };
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::convex_polygon(points, ui.visuals().text_color(), egui::Stroke::NONE));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// The fill of a tool button: neutral, so the copper buttons stay the ones that move on.
pub fn tool_fill(ui: &Ui) -> Color32 {
    neutral_fills(ui.visuals().dark_mode).0
}

/// A button that changes the picture or the page: neutral and full width, never copper.
pub fn tool_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let fill = tool_fill(ui);
    let ink = ui.visuals().text_color();
    let button =
        egui::Button::new(egui::RichText::new(label).color(ink)).fill(fill).stroke(Stroke::NONE).min_size(Vec2::new(ui.available_width(), 28.0));
    ui.add_enabled(enabled, button)
}

/// A small drawn icon at the head of a toast: a red disc with an exclamation mark for an error, a
/// neutral disc with a tick for anything else. The two differ in shape as well as in colour.
pub fn toast_icon(ui: &mut Ui, error: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    if error {
        painter.circle_filled(c, 8.0, INVALID);
        painter.line_segment([c + Vec2::new(0.0, -4.0), c + Vec2::new(0.0, 1.0)], Stroke::new(2.0, Color32::WHITE));
        painter.circle_filled(c + Vec2::new(0.0, 4.0), 1.2, Color32::WHITE);
    } else {
        painter.circle_filled(c, 8.0, accent(ui.visuals().dark_mode));
        let stroke = Stroke::new(2.0, Color32::WHITE);
        painter.line_segment([c + Vec2::new(-3.5, 0.0), c + Vec2::new(-1.0, 3.0)], stroke);
        painter.line_segment([c + Vec2::new(-1.0, 3.0), c + Vec2::new(4.0, -3.5)], stroke);
    }
}

/// A small close button, a drawn cross with `label` as its accessible name.
pub fn close_button(ui: &mut Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    let colour = if response.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    let c = rect.center();
    let stroke = Stroke::new(1.5, colour);
    let painter = ui.painter_at(rect);
    painter.line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], stroke);
    painter.line_segment([c + Vec2::new(-4.0, 4.0), c + Vec2::new(4.0, -4.0)], stroke);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The colour of text on the window's own background, in either appearance.
pub fn ink(ui: &Ui) -> Color32 {
    ui.visuals().text_color()
}

/// The heading of a group of tools.
pub fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(text.to_uppercase()).small().strong().color(ui.visuals().weak_text_color()));
    ui.add_space(2.0);
}

/// Which way a navigation button points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// To the previous step.
    Back,
    /// To the next step.
    Forward,
}

/// A button that moves to another step: large, with an arrow, copper when it goes forward and
/// outlined when it goes back. Tools never look like this.
pub fn nav_button(ui: &mut Ui, label: &str, direction: Direction, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(15.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, Color32::WHITE);
    let size = Vec2::new(galley.size().x + 28.0 + 26.0, 38.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = enabled && response.hovered();
    let forward = direction == Direction::Forward;
    let (fill, text, stroke) = if forward {
        (if hovered { LIGHT_COPPER } else { COPPER }, Color32::WHITE, Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, ui.visuals().text_color(), Stroke::new(1.5, if hovered { LIGHT_COPPER } else { ui.visuals().weak_text_color() }))
    };
    let alpha = if enabled { 1.0 } else { 0.4 };
    let painter = ui.painter_at(rect.expand(2.0));
    painter.rect(rect, 8.0, fill.gamma_multiply(alpha), stroke, egui::StrokeKind::Inside);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(15.0), text.gamma_multiply(alpha));
    let (text_x, arrow_x) = if forward { (rect.min.x + 14.0, rect.max.x - 15.0) } else { (rect.min.x + 30.0, rect.min.x + 15.0) };
    painter.galley(egui::Pos2::new(text_x, rect.center().y - galley.size().y / 2.0), galley, text);
    let (w, h) = (4.0f32, 6.0f32);
    let c = egui::Pos2::new(arrow_x, rect.center().y);
    let points = if forward {
        vec![egui::Pos2::new(c.x - w, c.y - h), egui::Pos2::new(c.x - w, c.y + h), egui::Pos2::new(c.x + w, c.y)]
    } else {
        vec![egui::Pos2::new(c.x + w, c.y - h), egui::Pos2::new(c.x + w, c.y + h), egui::Pos2::new(c.x - w, c.y)]
    };
    painter.add(egui::Shape::convex_polygon(points, text.gamma_multiply(alpha), Stroke::NONE));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response }
}
