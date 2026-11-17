//! The pages sidebar (§4.5): every page of the scan, in the order of the PDF, down the left of the
//! window, as Preview lists the pages of a document.
//!
//! A thumbnail keeps the page's own shape and shows its number. A spinner marks a page still being
//! prepared, an amber dot one whose edges need a look, and a red mark one that could not be made.
//! Photos still being read have a placeholder at the end of the list. A click opens the page in the
//! view the window is in (the finished page, or Crop); a drag moves it, with a line where it will
//! land; the context menu offers Crop, Rotate and Delete.

use egui::{Color32, Key, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, ImageKey, PageState, TrayItem, ViewState};
use scancraft_core::recipe::PageId;

use crate::canvas::dispatch;
use crate::state::UiState;
use crate::theme;

/// The width of the sidebar.
pub const WIDTH: f32 = 148.0;
/// The box a thumbnail is fitted into.
pub const THUMB_BOX: Vec2 = Vec2::new(116.0, 150.0);
/// The gap between two thumbnails.
pub const GAP: f32 = 10.0;
/// At most this many placeholders are drawn for photos still being read; the status bar has the count.
const MAX_PLACEHOLDERS: u32 = 8;

/// The view the window is in, which decides what a click on a page opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The finished page, with its filters.
    Filter,
    /// The original photo and the page's outline.
    Crop,
}

/// The mode the view is in: Crop when Screen 2 is open, the finished page otherwise.
pub fn mode_of(view: &ViewState) -> Mode {
    if view.crop.is_some() { Mode::Crop } else { Mode::Filter }
}

/// The pages the window is showing: the finished page that is open, or the pages cut from the
/// photo that Crop is open on (a spread has two).
pub fn open_pages(view: &ViewState) -> Vec<PageId> {
    if let Some(crop) = view.crop.as_ref() {
        return view.tray.iter().filter(|item| item.photo == crop.photo).map(|item| item.page).collect();
    }
    view.enhance.as_ref().map(|enhance| vec![enhance.page]).unwrap_or_default()
}

/// The page that Rotate, Delete and the menus act on: the one open, or the first of a spread.
pub fn current_page(view: &ViewState) -> Option<PageId> {
    open_pages(view).first().copied().or_else(|| view.crop.as_ref().and_then(|crop| crop.outlines.first().map(|o| o.page)))
}

/// The page before or after the pages that are open, in the order of the document.
pub fn neighbour_page(view: &ViewState, forward: bool) -> Option<PageId> {
    let open = open_pages(view);
    let positions: Vec<usize> = view.tray.iter().enumerate().filter(|(_, item)| open.contains(&item.page)).map(|(i, _)| i).collect();
    let (first, last) = (positions.first().copied()?, positions.last().copied()?);
    let at = if forward { last.checked_add(1)? } else { first.checked_sub(1)? };
    view.tray.get(at).map(|item| item.page)
}

/// Whether a page's edges need a look.
pub fn needs_look(item: &TrayItem) -> bool {
    matches!(item.state, PageState::NeedsLook(_))
}

/// The pages that need a look, in the order of the document.
pub fn flagged_pages(view: &ViewState) -> Vec<PageId> {
    view.tray.iter().filter(|item| needs_look(item)).map(|item| item.page).collect()
}

/// The page that needs a look next: the first flagged page after the ones that are open, or before
/// them, and wrapping round; `None` when no page is flagged.
pub fn next_flagged(view: &ViewState, forward: bool) -> Option<PageId> {
    let open = open_pages(view);
    let flagged: Vec<usize> =
        view.tray.iter().enumerate().filter(|(_, item)| needs_look(item) && !open.contains(&item.page)).map(|(i, _)| i).collect();
    if flagged.is_empty() {
        return None;
    }
    let anchor = view.tray.iter().position(|item| open.contains(&item.page));
    let at = match (anchor, forward) {
        (Some(a), true) => flagged.iter().copied().find(|i| *i > a).or(flagged.first().copied()),
        (Some(a), false) => flagged.iter().rev().copied().find(|i| *i < a).or(flagged.last().copied()),
        (None, _) => flagged.first().copied(),
    }?;
    view.tray.get(at).map(|item| item.page)
}

/// Opens a page in the view the window is in.
pub fn open_page(state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi, page: PageId) {
    match mode_of(view) {
        Mode::Crop => dispatch(state, engine, Command::PageOpenCrop { page }),
        Mode::Filter => dispatch(state, engine, Command::PageEdit { page }),
    };
}

/// Draws the sidebar.
pub fn sidebar(ui: &mut Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let open = open_pages(view);
    let mut rects: Vec<Rect> = Vec::with_capacity(view.tray.len());
    let mut responses = Vec::with_capacity(view.tray.len());
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("scancraft.sidebar").show(ui, |ui| {
        ui.add_space(GAP);
        for item in &view.tray {
            let (cell, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), THUMB_BOX.y + GAP), Sense::hover());
            let thumb_box = Rect::from_center_size(Pos2::new(cell.center().x, cell.min.y + THUMB_BOX.y / 2.0), THUMB_BOX);
            rects.push(cell);
            draw_thumbnail(ui, state, item, thumb_box, open.contains(&item.page), engine);
            let id = ui.make_persistent_id(("scancraft.page", item.page));
            let response = ui.interact(cell, id, Sense::click_and_drag());
            let label = page_label(state, item);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open.contains(&item.page), label.clone()));
            responses.push(response);
        }
        placeholders(ui, state, view);
    });
    for (index, item) in view.tray.iter().enumerate() {
        if let Some(response) = responses.get(index) {
            handle_page(ui, state, view, item, response, &rects, index, engine);
        }
    }
}

/// The accessible name of a page: its number, and what is wrong with it.
fn page_label(state: &UiState, item: &TrayItem) -> String {
    let base = state.text.t("Page {n}").replace("{n}", &item.number.to_string());
    match &item.state {
        PageState::Ready => base,
        PageState::Preparing => format!("{base}, {}", state.text.t("being prepared")),
        PageState::NeedsLook(_) => format!("{base}, {}", state.text.t("needs a look")),
        PageState::Failed(_) => format!("{base}, {}", state.text.t("could not be made")),
    }
}

/// Draws one thumbnail with its number and its state.
fn draw_thumbnail(ui: &mut Ui, state: &mut UiState, item: &TrayItem, thumb_box: Rect, selected: bool, engine: &mut dyn EngineApi) {
    // A page scrolled out of the list is not drawn and its picture is not asked for: a long scan
    // would otherwise have the engine making a thumbnail of every page on every frame.
    if !ui.is_rect_visible(thumb_box) {
        return;
    }
    let texture = state.texture(ui, engine, ImageKey::TrayThumb(item.page), TextureOptions::LINEAR);
    let painter = ui.painter_at(thumb_box.expand(6.0));
    let shown = match texture {
        Some(texture) => crate::thumb::paint_fitted(&painter, thumb_box, &texture),
        None => {
            // No picture yet: a dark card of a page's usual shape, so the list does not jump.
            let card = crate::thumb::fit_inside(thumb_box, [3, 4]);
            painter.rect_filled(card, 3.0, theme::CANVAS_BACKGROUND);
            card
        }
    };
    if selected {
        let accent = theme::accent(ui.visuals().dark_mode);
        painter.rect_stroke(shown.expand(2.0), 3.0, egui::Stroke::new(3.0, accent), egui::StrokeKind::Middle);
    }
    // The number goes inside the picture, on a dark chip.
    let galley = painter.layout_no_wrap(item.number.to_string(), egui::FontId::proportional(11.0), Color32::WHITE);
    let chip = Rect::from_min_size(
        Pos2::new(shown.min.x + 4.0, shown.max.y - galley.size().y - 8.0),
        Vec2::new(galley.size().x + 8.0, galley.size().y + 4.0),
    );
    painter.rect_filled(chip, 4.0, Color32::from_black_alpha(170));
    painter.galley(chip.min + Vec2::new(4.0, 2.0), galley, Color32::WHITE);
    let badge = Pos2::new(shown.max.x - 11.0, shown.min.y + 11.0);
    match item.state {
        PageState::Ready => {}
        PageState::Preparing => {
            painter.circle_filled(badge, 10.0, Color32::from_black_alpha(180));
            theme::spinner(ui, Rect::from_center_size(badge, Vec2::splat(15.0)), Color32::WHITE);
        }
        PageState::NeedsLook(_) => {
            // Amber, with a dark rim so it shows on a white page.
            painter.circle_filled(badge, 8.5, Color32::from_black_alpha(190));
            painter.circle_filled(badge, 6.0, theme::NEEDS_LOOK);
        }
        PageState::Failed(_) => {
            painter.circle_filled(badge, 9.0, theme::INVALID);
            painter.line_segment([badge + Vec2::new(0.0, -4.0), badge + Vec2::new(0.0, 1.0)], egui::Stroke::new(2.0, Color32::WHITE));
            painter.circle_filled(badge + Vec2::new(0.0, 4.5), 1.2, Color32::WHITE);
        }
    }
}

/// A card for each photo still being read, at the end of the list.
fn placeholders(ui: &mut Ui, state: &UiState, view: &ViewState) {
    let Some(progress) = view.importing else { return };
    let waiting = progress.total.saturating_sub(progress.done);
    let label = state.text.t("Reading the photo…");
    for _ in 0..waiting.min(MAX_PLACEHOLDERS) {
        let (cell, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), THUMB_BOX.y + GAP), Sense::hover());
        let card = crate::thumb::fit_inside(Rect::from_center_size(Pos2::new(cell.center().x, cell.min.y + THUMB_BOX.y / 2.0), THUMB_BOX), [3, 4]);
        let painter = ui.painter_at(cell);
        painter.rect_filled(card, 3.0, theme::CANVAS_BACKGROUND);
        theme::spinner(ui, Rect::from_center_size(card.center(), Vec2::splat(22.0)), Color32::from_white_alpha(190));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label.clone()));
    }
}

/// A click opens the page, a drag moves it, and the context menu offers Crop, Rotate and Delete.
#[allow(clippy::too_many_arguments)]
fn handle_page(
    ui: &Ui,
    state: &mut UiState,
    view: &ViewState,
    item: &TrayItem,
    response: &egui::Response,
    rects: &[Rect],
    index: usize,
    engine: &mut dyn EngineApi,
) {
    if response.drag_started() {
        state.tray_drag = Some((index as u32, index as u32));
    }
    if response.dragged() {
        // Where the page would land: the gap whose line is nearest the pointer. The line is drawn
        // there; the page moves when the button is let go.
        let at = gap_under(rects, response.interact_pointer_pos());
        if let Some((from, _)) = state.tray_drag {
            state.tray_drag = Some((from, at));
        }
        if let Some(rect) = gap_rect(rects, at) {
            let accent = theme::accent(ui.visuals().dark_mode);
            ui.painter().line_segment([rect.left_center(), rect.right_center()], egui::Stroke::new(3.0, accent));
        }
    }
    if response.drag_stopped() {
        if let Some((from, at)) = state.tray_drag.take() {
            // Removing the page shifts every later gap up by one.
            let to = if at > from { at - 1 } else { at };
            if to != from {
                dispatch(state, engine, Command::PageMove { from, to });
            }
        }
    } else if response.clicked() {
        open_page(state, view, engine, item.page);
    }
    let page = item.page;
    let number = item.number;
    response.context_menu(|ui| {
        if ui.button(state.text.t("Crop")).clicked() {
            dispatch(state, engine, Command::PageOpenCrop { page });
            ui.close();
        }
        if ui.button(state.text.t("Rotate Left")).clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: -1 });
            ui.close();
        }
        if ui.button(state.text.t("Rotate Right")).clicked() {
            dispatch(state, engine, Command::PageRotate { page, quarter_turns: 1 });
            ui.close();
        }
        if needs_look(item) && ui.button(state.text.t("This Page Is Fine")).clicked() {
            dispatch(state, engine, Command::PageMarkReviewed { page });
            ui.close();
        }
        if ui.button(state.text.t("Delete")).clicked() {
            delete_page(state, engine, page, number);
            ui.close();
        }
    });
}

/// The index of the gap nearest the pointer: 0 is above the first page, `rects.len()` below the last.
pub fn gap_under(rects: &[Rect], pointer: Option<Pos2>) -> u32 {
    let Some(pointer) = pointer else { return 0 };
    rects.iter().take_while(|rect| rect.center().y < pointer.y).count() as u32
}

/// Where the line of a gap is drawn: along the top of the page below it, or the bottom of the last.
fn gap_rect(rects: &[Rect], at: u32) -> Option<Rect> {
    let at = at as usize;
    let (x0, x1) = (rects.first()?.min.x + 4.0, rects.first()?.max.x - 4.0);
    let y = match rects.get(at) {
        Some(below) => below.min.y - GAP / 2.0,
        None => rects.last()?.max.y - GAP / 2.0,
    };
    Some(Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, y)))
}

/// The Delete key removes the open page, with an Undo on screen (§4.5). The caller holds it back
/// while a window of the interface is open.
pub fn delete_key(ui: &Ui, state: &mut UiState, view: &ViewState, engine: &mut dyn EngineApi) {
    let pressed = ui.input(|i| i.key_pressed(Key::Backspace) || i.key_pressed(Key::Delete));
    if !pressed {
        return;
    }
    let Some(page) = current_page(view) else { return };
    let Some(number) = view.tray.iter().find(|item| item.page == page).map(|item| item.number) else { return };
    delete_page(state, engine, page, number);
}

/// Deletes a page and says so, with an Undo: nothing else on the screen says that a page has gone.
pub fn delete_page(state: &mut UiState, engine: &mut dyn EngineApi, page: PageId, number: u32) {
    if dispatch(state, engine, Command::PageDelete { page }) {
        let text = state.text.t("Page {n} deleted").replace("{n}", &number.to_string());
        state.toast_with_undo(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(n: usize) -> Vec<Rect> {
        (0..n).map(|i| Rect::from_min_size(Pos2::new(0.0, i as f32 * 100.0), Vec2::new(120.0, 90.0))).collect()
    }

    #[test]
    fn the_gap_is_the_one_the_pointer_is_nearest() {
        let rects = rects(3);
        // Above the middle of the first page: before it. Below the middle of it: after it.
        assert_eq!(gap_under(&rects, Some(Pos2::new(10.0, 10.0))), 0);
        assert_eq!(gap_under(&rects, Some(Pos2::new(10.0, 60.0))), 1);
        assert_eq!(gap_under(&rects, Some(Pos2::new(10.0, 160.0))), 2);
        assert_eq!(gap_under(&rects, Some(Pos2::new(10.0, 900.0))), 3);
        assert_eq!(gap_under(&rects, None), 0);
    }

    #[test]
    fn the_line_of_a_gap_is_between_two_pages_and_below_the_last() {
        let rects = rects(2);
        let top = gap_rect(&rects, 1).expect("a line");
        assert!(top.min.y > rects[0].max.y - 1.0 && top.min.y < rects[1].min.y + 1.0, "{top:?}");
        let end = gap_rect(&rects, 2).expect("a line");
        assert!(end.min.y >= rects[1].max.y - GAP, "{end:?}");
        assert!(gap_rect(&[], 0).is_none());
    }
}
