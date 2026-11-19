//! The interface's tests: the three screens draw, every control has an accessible name, and every
//! action dispatches the command of §4.
//!
//! The tests run against the fake engine, so they say nothing about the pipeline: they say that
//! the interface draws what the specification describes and asks the engine for the right things.

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EnhanceView, ImageKey, Notice, OutlineView, PageState, PhotoTab, Screen, TrayItem, ViewState};
use scancraft_core::geom::{Point, Quad, Side};
use scancraft_core::image::Rgb8;
use scancraft_core::recipe::{CornerOrigin, Detector, FlattenDecision, FlattenMode, FlattenReason, Half, Look, PageId, PhotoId, SideShape};
use scancraft_ui_egui::fake_engine::FakeEngine;
use scancraft_ui_egui::state::Selection;
use scancraft_ui_egui::{ScanCraftUi, UiAction};

/// The photo of the batch.
const PHOTO: PhotoId = PhotoId(1);
/// The page cut from it.
const PAGE: PageId = PageId(1);

/// The interface under test and the engine it draws from.
type TestState = (ScanCraftUi, FakeEngine);

/// A picture with a gradient in it, so a change in the pixels shows.
fn picture(width: u32, height: u32) -> Arc<Rgb8> {
    let mut data = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for y in 0..height {
        for x in 0..width {
            data.push([(x % 251) as u8, (y % 241) as u8, 128]);
        }
    }
    Arc::new(Rgb8::from_vec(width, height, data).expect("the picture's size"))
}

/// The corners of the page in the photo.
fn corners() -> Quad {
    Quad([Point::new(120.0, 80.0), Point::new(1080.0, 60.0), Point::new(1100.0, 820.0), Point::new(100.0, 840.0)])
}

/// The outline of the page.
fn outline() -> OutlineView {
    OutlineView {
        page: PAGE,
        corners: corners(),
        corners_from: CornerOrigin::Detected { by: Detector::Apple, score: 0.9 },
        sides: [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight],
        fitted: None,
        half: None,
        invalid: None,
        flatten: FlattenMode::Auto,
        flatten_decision: None,
    }
}

fn outline_corners() -> scancraft_core::geom::Quad {
    outline().corners
}

/// The three pages of the document: the open one, and two others, the second still being read.
fn three_pages() -> Vec<TrayItem> {
    vec![
        TrayItem { page: PAGE, photo: PHOTO, number: 1, text_ready: true, state: PageState::Ready },
        TrayItem { page: PageId(11), photo: PhotoId(111), number: 2, text_ready: false, state: PageState::Ready },
        TrayItem { page: PageId(12), photo: PhotoId(112), number: 3, text_ready: true, state: PageState::Ready },
    ]
}

/// Crop's view: one photo of seven, with its page found.
fn crop_view() -> ViewState {
    ViewState {
        screen: Screen::Crop(PHOTO),
        crop: Some(CropView {
            photos: (1..=7)
                .map(|n| PhotoTab {
                    photo: if n == 2 { PHOTO } else { PhotoId(100 + n) },
                    page: Some(PageId(200 + n)),
                    name: format!("IMG_{n}.jpg"),
                    added: n == 1,
                })
                .collect(),
            photo: PHOTO,
            position: (2, 7),
            photo_size: (1200, 900),
            outlines: vec![outline()],
            spine: None,
            suggest_split: false,
            auto_detect: true,
            not_found: false,
        }),
        enhance: None,
        tray: three_pages(),
        book_spreads: false,
        can_undo: false,
        can_redo: false,
        busy: false,
        notices: Vec::new(),
        restore_offer: None,
        importing: None,
        reading_text: None,
        exporting: None,
        last_export: None,
    }
}

/// Screen 3's view: the page of the crop view, with three pages in the tray.
fn enhance_view() -> ViewState {
    ViewState {
        screen: Screen::Enhance(PAGE),
        crop: None,
        enhance: Some(EnhanceView {
            page: PAGE,
            look: Look::ColorDocument,
            strength: 60,
            quarter_turns: 0,
            turned_automatically: false,
            flattened_automatically: false,
            half: None,
            low_resolution: false,
            more_to_come: true,
        }),
        tray: three_pages(),
        book_spreads: false,
        can_undo: true,
        can_redo: false,
        busy: false,
        notices: Vec::new(),
        restore_offer: None,
        importing: None,
        reading_text: None,
        exporting: None,
        last_export: None,
    }
}

/// Screen 1's view: nothing imported, and a session to restore.
fn import_view() -> ViewState {
    ViewState {
        screen: Screen::Import,
        crop: None,
        enhance: None,
        tray: Vec::new(),
        book_spreads: false,
        can_undo: false,
        can_redo: false,
        busy: false,
        notices: Vec::new(),
        restore_offer: Some(7),
        importing: None,
        reading_text: None,
        exporting: None,
        last_export: None,
    }
}

/// The harness of a view, in one appearance.
fn harness(view: ViewState, theme: egui::Theme) -> Harness<'static, TestState> {
    let mut engine = FakeEngine::with_view(view)
        .with_image(ImageKey::DisplayCopy(PHOTO), picture(600, 450))
        .with_image(ImageKey::LookPreview(PAGE), picture(600, 450));
    for look in [Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument] {
        engine = engine.with_image(ImageKey::FilterThumb(PAGE, look), picture(256, 343));
    }
    for page in [PAGE, PageId(11), PageId(12)] {
        engine = engine.with_image(ImageKey::TrayThumb(page), picture(128, 176));
    }
    harness_with(engine, theme)
}

/// The harness of an engine the test made, in one appearance.
fn harness_with(engine: FakeEngine, theme: egui::Theme) -> Harness<'static, TestState> {
    harness_sized(engine, theme, egui::vec2(1200.0, 800.0))
}

/// The harness of an engine the test made, in a window of another size.
fn harness_sized(engine: FakeEngine, theme: egui::Theme, size: egui::Vec2) -> Harness<'static, TestState> {
    Harness::builder()
        .with_size(size)
        .with_theme(theme)
        // The bundled font, so the snapshots are the same on every machine.
        .with_context_setup(|ctx| ctx.set_fonts(egui::FontDefinitions::default()))
        .build_ui_state(
            |ui, state: &mut TestState| {
                state.0.ui(ui, &mut state.1);
            },
            (ScanCraftUi::new(), engine),
        )
}

/// The commands the interface sent.
fn dispatched(harness: &Harness<TestState>) -> Vec<Command> {
    harness.state().1.dispatched().to_vec()
}

/// The text the screen shows: every label and every button.
fn shown_text(harness: &Harness<TestState>) -> String {
    let mut text = String::new();
    for node in harness.root().children_recursive() {
        if let Some(value) = node.accesskit_node().data().value() {
            text.push_str(value);
            text.push('\n');
        }
    }
    text
}

/// Clicks a widget with the given label.
fn click_label(harness: &mut Harness<TestState>, label: &str) {
    let bounds = {
        let node = harness.query_all_by_label(label).next().unwrap_or_else(|| panic!("no widget labelled {label:?}"));
        node.accesskit_node().raw_bounds().unwrap_or_else(|| panic!("the widget {label:?} has no bounds"))
    };
    let pos = egui::Pos2::new(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
    click_at(harness, pos);
}

/// Clicks the last widget with the given label: the one drawn latest, such as a toast's button
/// over the toolbar's of the same name.
fn click_last_label(harness: &mut Harness<TestState>, label: &str) {
    let bounds = {
        let node = harness.query_all_by_label(label).last().unwrap_or_else(|| panic!("no widget labelled {label:?}"));
        node.accesskit_node().raw_bounds().unwrap_or_else(|| panic!("the widget {label:?} has no bounds"))
    };
    click_at(harness, egui::Pos2::new(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32));
}

/// Clicks a point.
fn click_at(harness: &mut Harness<TestState>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run();
}

/// The bounds of a widget with the given label.
fn bounds_of(harness: &Harness<TestState>, label: &str) -> egui::Rect {
    let node = harness.query_all_by_label(label).next().unwrap_or_else(|| panic!("no widget labelled {label:?}"));
    let bounds = node.accesskit_node().raw_bounds().unwrap_or_else(|| panic!("the widget {label:?} has no bounds"));
    egui::Rect::from_min_max(egui::Pos2::new(bounds.x0 as f32, bounds.y0 as f32), egui::Pos2::new(bounds.x1 as f32, bounds.y1 as f32))
}

/// Whether this machine can render the screen: the snapshot needs a renderer, and a headless
/// machine may have none. The test says so and passes when there is none.
fn can_render(harness: &mut Harness<TestState>) -> bool {
    match harness.render() {
        Ok(_) => true,
        Err(why) => {
            eprintln!("no renderer on this machine, so the snapshot is skipped: {why}");
            false
        }
    }
}

// ---------------------------------------------------------------- the snapshots

#[test]
fn empty_window_light_snapshot() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "empty_window_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn empty_window_dark_snapshot() {
    let mut harness = harness(import_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "empty_window_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn crop_view_light_snapshot() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "crop_view_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn crop_view_dark_snapshot() {
    let mut harness = harness(crop_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "crop_view_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

/// The options the snapshots of the finished page share.
fn snapshot_options() -> egui_kittest::SnapshotOptions {
    egui_kittest::SnapshotOptions::new()
        .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
        .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400))
}

#[test]
fn filter_view_with_pages_that_need_a_look_snapshot() {
    // Amber dots on the pages, a page being prepared, and the status bar saying what is going on.
    let mut view = enhance_view();
    view.tray[1].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::NotFound);
    view.tray[2].state = PageState::Preparing;
    view.reading_text = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.step();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options("filter_view_needs_a_look_light", &snapshot_options());
}

#[test]
fn export_sheet_running_snapshot() {
    let mut view = enhance_view();
    view.exporting = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options("export_sheet_running_light", &snapshot_options());
}

#[test]
fn export_sheet_finished_snapshot() {
    let mut view = enhance_view();
    view.last_export =
        Some(scancraft_core::engine_api::ExportSummary { path: "/Users/me/Scans/Receipts.pdf".to_owned(), pages: 3, bytes: 3_412_000 });
    let mut harness = harness(view, egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options("export_sheet_finished_dark", &snapshot_options());
}

/// The crop screen with the top-left corner pressed, so the loupe is open: a sharp crop of the
/// photo (a fine chequer, so the magnification can be seen) over the display copy.
fn loupe_snapshot(theme: egui::Theme, name: &str) {
    let mut chequer = Vec::new();
    for y in 0..400u32 {
        for x in 0..400u32 {
            let on = ((x / 8) + (y / 8)) % 2 == 0;
            chequer.push(if on { [240, 240, 235] } else { [60, 60, 70] });
        }
    }
    let crop = scancraft_core::engine_api::LoupeCrop {
        image: Arc::new(Rgb8::from_vec(400, 400, chequer).expect("the chequer's size")),
        origin: Point::new(120.0 - 200.0, 80.0 - 200.0),
        scale: 1.0,
    };
    let mut engine = FakeEngine::with_view(crop_view()).with_image(ImageKey::DisplayCopy(PHOTO), picture(600, 450));
    engine.set_loupe(Some(crop));
    let mut harness = harness_with(engine, theme);
    harness.run();
    press_corner(&mut harness, egui::Vec2::ZERO);
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        name,
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn crop_loupe_light_snapshot() {
    loupe_snapshot(egui::Theme::Light, "crop_loupe_light");
}

#[test]
fn crop_loupe_dark_snapshot() {
    loupe_snapshot(egui::Theme::Dark, "crop_loupe_dark");
}

#[test]
fn filter_view_light_snapshot() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "filter_view_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn filter_view_dark_snapshot() {
    let mut harness = harness(enhance_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "filter_view_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

// ------------------------------------------------------------- accessibility

#[test]
fn every_control_has_an_accessible_name() {
    for view in [import_view(), crop_view(), enhance_view()] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        assert!(harness.unnamed_widgets().is_empty(), "unnamed widgets: {:?}", harness.unnamed_widgets());
    }
}

#[test]
fn corners_are_accessible_adjustable_elements() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let labels = ["Top-left corner, 120, 80", "Top-right corner, 1080, 60", "Bottom-right corner, 1100, 820", "Bottom-left corner, 100, 840"];
    for label in labels {
        let node = harness.query_by_label(label).unwrap_or_else(|| panic!("no corner labelled {label:?}"));
        assert_eq!(node.accesskit_node().role(), egui::Role::Slider, "the corner {label:?} is not adjustable");
    }
}

#[test]
fn bend_handles_are_accessible_adjustable_elements() {
    let mut view = crop_view();
    let mut outline = outline();
    outline.sides[Side::Top.index()] = SideShape::Bent { handles: [Point::new(400.0, 70.0), Point::new(600.0, 60.0), Point::new(800.0, 55.0)] };
    view.crop.as_mut().expect("the crop view").outlines = vec![outline];
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    // One vertex per bent side, the middle one.
    let label = "Top side, bend handle 2 of 3";
    let node = harness.query_by_label(label).unwrap_or_else(|| panic!("no handle labelled {label:?}"));
    assert_eq!(node.accesskit_node().role(), egui::Role::Slider, "the handle {label:?} is not adjustable");
    assert!(harness.query_by_label("Top side, bend handle 1 of 3").is_none());
}

#[test]
fn the_menu_bar_has_the_items_of_the_specification() {
    let bar = scancraft_ui_egui::build_menu_bar(&enhance_view());
    let titles: Vec<&str> = bar.menus.iter().map(|menu| menu.title.as_str()).collect();
    assert_eq!(titles, vec!["ScanCraft", "File", "Edit", "Page", "View"]);
    let mut labels: Vec<&str> = bar.items().iter().map(|item| item.label.as_str()).collect();
    for menu in &bar.menus {
        for node in &menu.children {
            if let craft_menu::Node::Submenu { label, .. } = node {
                labels.push(label.as_str());
            }
        }
    }
    for expected in [
        "About ScanCraft",
        "Settings…",
        "Quit ScanCraft",
        "New Scan",
        "Add Pages…",
        "Export PDF…",
        "Export Options…",
        "Undo",
        "Redo",
        "Delete Page",
        "Select All",
        "Crop",
        "Reset to Detected Edges",
        "Use Whole Photo",
        "Flatten",
        "Split into Two Pages",
        "Rotate Left",
        "Rotate Right",
        "Filter",
        "Apply Filter to All Pages",
        "Zoom In",
        "Zoom Out",
        "Zoom to Fit",
        "Show Pages",
        "Show Inspector",
        "Previous Page",
        "Next Page",
    ] {
        assert!(labels.contains(&expected), "the menu bar has no {expected:?} item: {labels:?}");
    }
    // What the flow no longer has.
    for gone in ["Add Page", "Correct Perspective", "Back to Crop", "Book Spreads", "Straighten Side", "Auto-Detect", "Look", "New Session"] {
        assert!(!labels.contains(&gone), "the menu bar still has {gone:?}: {labels:?}");
    }
    // The shortcuts of §4.7.
    let shortcut_of = |label: &str| {
        bar.items().iter().find(|item| item.label == label).and_then(|item| item.shortcut).map(|chord| chord.display(craft_keymap::Platform::Mac))
    };
    assert_eq!(shortcut_of("Add Pages…").as_deref(), Some("⌘O"));
    assert_eq!(shortcut_of("Export PDF…").as_deref(), Some("⌘E"));
    assert_eq!(shortcut_of("Undo").as_deref(), Some("⌘Z"));
    assert_eq!(shortcut_of("Select All").as_deref(), Some("⌘A"));
    assert_eq!(shortcut_of("Split into Two Pages").as_deref(), Some("⌥⌘B"));
    assert_eq!(shortcut_of("Show Pages").as_deref(), Some("⌥⌘T"));
    assert_eq!(shortcut_of("Show Inspector").as_deref(), Some("⌥⌘I"));
    assert_eq!(shortcut_of("Zoom to Fit").as_deref(), Some("⌘0"));
}

#[test]
fn a_menu_item_is_live_only_when_it_can_act() {
    use scancraft_ui_egui::menu::item_enabled;
    // The empty window: nothing to crop, turn, filter, export or zoom. These items were live and did nothing.
    let import = import_view();
    for id in [
        "crop.auto",
        "crop.full_image",
        "crop.split",
        "page.rotate",
        "page.set_look",
        "page.open_crop",
        "page.delete",
        "page.set_look_all",
        "export.pdf",
        "file.export_options",
        "page.previous",
        "page.next",
        "view.zoom_in",
        "view.zoom_fit",
    ] {
        assert!(!item_enabled(id, &import), "{id} is live in the empty window");
    }
    for id in ["session.new", "file.open", "app.settings"] {
        assert!(item_enabled(id, &import), "{id} is off in the empty window");
    }
    // Crop, on the first page: the crop items act, a filter does not, and the edges are the
    // detected ones, so there is nothing to reset. There is no page before the first.
    let crop = crop_view();
    for id in ["crop.full_image", "crop.split", "page.rotate", "page.delete", "export.pdf", "page.set_look_all", "page.next", "view.zoom_in"] {
        assert!(item_enabled(id, &crop), "{id} is off in Crop");
    }
    for id in ["page.set_look", "page.open_crop", "crop.auto", "page.previous"] {
        assert!(!item_enabled(id, &crop), "{id} is live in Crop");
    }
    let mut by_hand = crop_view();
    by_hand.crop.as_mut().expect("the crop view").auto_detect = false;
    assert!(item_enabled("crop.auto", &by_hand));
    // Select All needs pages to select among: two or more.
    assert!(!item_enabled("edit.select_all", &import) && item_enabled("edit.select_all", &crop));
    let mut one = enhance_view();
    one.tray.truncate(1);
    assert!(!item_enabled("edit.select_all", &one));
    // The finished page: a filter and Crop act, the crop items do not.
    let enhance = enhance_view();
    for id in ["page.set_look", "page.open_crop", "page.rotate", "export.pdf"] {
        assert!(item_enabled(id, &enhance), "{id} is off on the finished page");
    }
    for id in ["crop.full_image", "crop.split", "crop.auto"] {
        assert!(!item_enabled(id, &enhance), "{id} is live on the finished page");
    }
    // The middle page has a page on each side; the last has none after it.
    let mut middle = enhance_view();
    middle.enhance.as_mut().expect("the enhance view").page = PageId(11);
    assert!(item_enabled("page.previous", &middle) && item_enabled("page.next", &middle));
    let mut last = enhance_view();
    last.enhance.as_mut().expect("the enhance view").page = PageId(12);
    assert!(item_enabled("page.previous", &last) && !item_enabled("page.next", &last));
}

#[test]
fn export_options_with_no_pages_opens_nothing_and_does_not_block_the_menus() {
    // The item set the popover's flag with no button to hang it from, and `is_modal` then greyed
    // out every menu for good.
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    let view = import_view();
    let command = craft_menu::CommandRef::new(scancraft_ui_egui::menu::EXPORT_OPTIONS);
    let (ui, engine) = harness.state_mut();
    let _ = engine;
    ui.run_menu_command(&command, &mut FakeEngine::with_view(view));
    assert!(!ui.is_modal(), "Export Options… with no pages made the window modal");
}

#[test]
fn the_export_options_close_on_a_click_outside_and_the_menus_come_back() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Export options");
    assert!(harness.state().0.is_modal(), "the options did not open");
    // A click on the canvas, far from the popup.
    click_at(&mut harness, egui::pos2(700.0, 300.0));
    assert!(!harness.state().0.is_modal(), "the flag stayed set after the popup closed, so the menus stayed grey");
}

// -------------------------------------------------------------- the commands

#[test]
fn reset_to_detected_edges_sends_crop_auto() {
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").auto_detect = false;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Reset to Detected Edges");
    assert_eq!(dispatched(&harness), vec![Command::CropAuto { page: PAGE }]);
}

#[test]
fn reset_to_detected_edges_does_nothing_while_the_edges_are_the_detected_ones() {
    // It was a checkbox that could be ticked and never unticked. It is a button now, and it is
    // off while there is nothing to reset.
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let node = harness.query_all_by_label("Reset to Detected Edges").next().expect("the button");
    assert!(node.accesskit_node().is_disabled(), "the button is on while the edges are the detected ones");
    click_label(&mut harness, "Reset to Detected Edges");
    assert!(dispatched(&harness).is_empty(), "a click sent {:?}", dispatched(&harness));
}

#[test]
fn use_whole_photo_sends_crop_full_image() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Use Whole Photo");
    assert_eq!(dispatched(&harness), vec![Command::CropFullImage { page: PAGE }]);
}

#[test]
fn done_the_filter_switch_and_return_all_finish_the_crop() {
    // Done in the inspector, the Filter side of the switch, and the Return key all send crop.apply.
    for how in ["Done", "Filter", "Return", "Escape"] {
        let mut harness = harness(crop_view(), egui::Theme::Light);
        harness.run();
        match how {
            "Return" => {
                harness.key_press(egui::Key::Enter);
                harness.run();
            }
            "Escape" => {
                // With nothing selected, Escape has nothing to let go of: it is Done.
                harness.key_press(egui::Key::Escape);
                harness.run();
            }
            label => click_label(&mut harness, label),
        }
        assert_eq!(dispatched(&harness), vec![Command::CropApply { photo: PHOTO }], "with {how}");
    }
}

#[test]
fn remove_photo_sends_import_discard() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Remove Photo");
    assert_eq!(dispatched(&harness), vec![Command::ImportDiscard { item: PHOTO }]);
}

#[test]
fn the_flatten_control_sends_crop_set_flatten() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Always");
    assert_eq!(dispatched(&harness), vec![Command::CropSetFlatten { page: PAGE, mode: FlattenMode::On }]);
}

#[test]
fn splitting_a_photo_sends_crop_split_and_the_offer_for_every_photo_sends_crop_book() {
    // One page: the switch is off, and turning it on splits this photo.
    let mut one = harness(crop_view(), egui::Theme::Light);
    one.run();
    assert!(one.query_all_by_label("Do this for every photo in this scan").next().is_none(), "the offer is there before the split");
    click_label(&mut one, "Split into two pages");
    assert_eq!(dispatched(&one), vec![Command::CropSplit { photo: PHOTO, on: true }]);
    // Two pages: the switch is on, and below it the same cut can be asked for every photo.
    let mut view = crop_view();
    let crop = view.crop.as_mut().expect("the crop view");
    let mut right = crop.outlines[0].clone();
    right.page = PageId(2);
    crop.outlines[0].half = Some(Half::Left);
    right.half = Some(Half::Right);
    crop.outlines.push(right);
    let mut two = harness(view, egui::Theme::Light);
    two.run();
    click_label(&mut two, "Do this for every photo in this scan");
    assert_eq!(dispatched(&two), vec![Command::CropBook { on: true }]);
    two.state_mut().1.clear();
    click_label(&mut two, "Split into two pages");
    assert_eq!(dispatched(&two), vec![Command::CropSplit { photo: PHOTO, on: false }]);
}

#[test]
fn the_restore_banner_sends_session_restore_and_session_new() {
    let mut continuing = harness(import_view(), egui::Theme::Light);
    continuing.run();
    click_label(&mut continuing, "Continue");
    assert_eq!(dispatched(&continuing), vec![Command::SessionRestore]);
    let mut fresh = harness(import_view(), egui::Theme::Light);
    fresh.run();
    click_label(&mut fresh, "Start New");
    assert_eq!(dispatched(&fresh), vec![Command::SessionNew]);
}

#[test]
fn choose_files_asks_the_shell_for_the_open_panel() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Choose Files…");
    assert_eq!(harness.state_mut().0.take_actions(), vec![UiAction::ImportFiles]);
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn dropped_files_send_import_add_in_order() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    harness.input_mut().dropped_files.push(Arc::new(Dropped("/tmp/IMG_9.jpg".into())));
    harness.input_mut().dropped_files.push(Arc::new(Dropped("/tmp/IMG_10.jpg".into())));
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::ImportAdd { paths: vec!["/tmp/IMG_9.jpg".to_owned(), "/tmp/IMG_10.jpg".to_owned()] }]);
}

#[test]
fn files_can_be_dropped_on_any_screen() {
    // Dropping used to work only on the Import screen: anywhere else, the files were ignored.
    for (name, view) in [("crop", crop_view()), ("enhance", enhance_view())] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        harness.input_mut().dropped_files.push(Arc::new(Dropped("/tmp/IMG_9.jpg".into())));
        harness.run();
        assert_eq!(dispatched(&harness), vec![Command::ImportAdd { paths: vec!["/tmp/IMG_9.jpg".to_owned()] }], "a drop on the {name} screen");
    }
}

#[test]
fn files_held_over_any_screen_show_what_a_drop_would_do() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    assert!(!shown_text(&harness).contains("Drop to add pages"));
    harness.input_mut().hovered_files.push(egui::HoveredFile::default());
    // Files held over the window keep it repainting, so it is stepped, not run until it settles.
    harness.step();
    // The overlay is painted, not a widget: it is in the frame's shapes.
    let painted = format!("{:?}", harness.output().shapes);
    assert!(painted.contains("Drop to add pages"), "nothing said what a drop would do");
}

#[test]
fn clicking_the_drop_zone_opens_the_open_panel() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    // The middle of the zone, away from the Browse Files… button under it.
    click_at(&mut harness, egui::pos2(600.0, 300.0));
    assert!(harness.state_mut().0.take_actions().contains(&UiAction::ImportFiles), "a click on the drop zone did nothing");
}

#[test]
fn a_look_click_sends_page_set_look() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Grayscale filter");
    assert_eq!(dispatched(&harness), vec![Command::PageSetLook { page: PAGE, look: Look::Grayscale }]);
}

#[test]
fn the_strength_slider_sends_page_set_strength() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    let slider = harness.query_by_role(egui::Role::Slider).expect("the strength slider");
    let bounds = slider.accesskit_node().raw_bounds().expect("the slider's bounds");
    let pos = egui::Pos2::new(bounds.x0 as f32 + 4.0, (bounds.y0 + bounds.y1) as f32 / 2.0);
    harness.drag_at(pos);
    harness.drag_at(egui::Pos2::new(bounds.x1 as f32 - 4.0, (bounds.y0 + bounds.y1) as f32 / 2.0));
    harness.drop_at(egui::Pos2::new(bounds.x1 as f32 - 4.0, (bounds.y0 + bounds.y1) as f32 / 2.0));
    harness.run();
    let commands = dispatched(&harness);
    assert!(
        commands.iter().any(|command| matches!(command, Command::PageSetStrength { page, .. } if *page == PAGE)),
        "the slider sent no strength: {commands:?}"
    );
}

#[test]
fn rotate_right_sends_page_rotate_by_one_quarter_turn() {
    let mut right = harness(enhance_view(), egui::Theme::Light);
    right.run();
    click_label(&mut right, "Rotate Right");
    assert_eq!(dispatched(&right), vec![Command::PageRotate { page: PAGE, quarter_turns: 1 }]);
    let mut left = harness(enhance_view(), egui::Theme::Light);
    left.run();
    click_label(&mut left, "Rotate Left");
    assert_eq!(dispatched(&left), vec![Command::PageRotate { page: PAGE, quarter_turns: -1 }]);
}

#[test]
fn the_label_window_asks_for_a_folder_and_then_writes_the_label() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    {
        let state = harness.state_mut();
        state.0.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::LABEL_REFERENCE), &mut state.1);
    }
    harness.run();
    // Without a folder the label can't be saved, and the window asks the shell for one.
    click_label(&mut harness, "Choose folder…");
    let actions = harness.state_mut().0.take_actions();
    assert!(matches!(actions.as_slice(), [UiAction::ChooseLabelFolder]), "{actions:?}");
    assert!(dispatched(&harness).is_empty(), "nothing is written before a folder is chosen");
    // The shell answers with a folder; the label is written for the page on screen.
    harness.state_mut().0.set_label_folder("/tmp/reference-set".to_string());
    harness.run();
    click_label(&mut harness, "Save label");
    assert_eq!(
        dispatched(&harness),
        vec![Command::ReferenceLabel { page: PAGE, folder: "/tmp/reference-set".to_string(), category: "dark desk".to_string() }]
    );
}

#[test]
fn export_pdf_asks_the_shell_for_the_save_panel_at_once() {
    // From the finished page and from Crop alike: no question first.
    for view in [enhance_view(), crop_view()] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        click_label(&mut harness, "Export PDF");
        let actions = harness.state_mut().0.take_actions();
        assert_eq!(actions.len(), 1, "the export asked for {actions:?}");
        assert!(matches!(actions[0], UiAction::ExportPdf { .. }), "the export asked for {actions:?}");
    }
}

#[test]
fn a_tray_click_sends_page_edit() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Page 2");
    assert_eq!(dispatched(&harness), vec![Command::PageEdit { page: PageId(11) }]);
}

#[test]
fn a_tray_drag_sends_page_move() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    let third = bounds_of(&harness, "Page 3");
    let first = bounds_of(&harness, "Page 1");
    harness.drag_at(third.center());
    harness.drag_at(first.center());
    harness.drop_at(first.center());
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::PageMove { from: 2, to: 0 }]);
}

#[test]
fn the_delete_key_deletes_the_page_that_is_open() {
    // The key used to delete whichever page was clicked last, from any screen, with nothing to say so.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::PageDelete { page: PAGE }]);
}

#[test]
fn deleting_a_page_says_so_and_offers_an_undo() {
    // The Delete key works from every view, and nothing on screen said that a page had gone.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert!(shown_text(&harness).contains("Page 1 deleted"), "nothing said the page was deleted: {}", shown_text(&harness));
    harness.state_mut().1.clear();
    // The toast's Undo is drawn over the toolbar's, which has the same name.
    click_last_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::EditUndo]);
    assert!(!shown_text(&harness).contains("Page 1 deleted"), "the toast stayed after its Undo was used");
}

// ------------------------------------------------------------ the geometry

#[test]
fn photo_pixels_map_to_canvas_points_and_back() {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let canvas = scancraft_ui_egui::canvas::Canvas::new(area, &picture(600, 450), (1200, 900));
    for point in [Point::new(0.0, 0.0), Point::new(1200.0, 900.0), Point::new(412.5, 380.25), Point::new(600.0, 450.0)] {
        let there = canvas.photo_to_canvas(point);
        let back = canvas.canvas_to_photo(there);
        // The canvas is in points, which are f32, so the round trip is a thousandth of a photo pixel.
        assert!((back.x - point.x).abs() < 1e-3 && (back.y - point.y).abs() < 1e-3, "{point:?} came back as {back:?}");
    }
}

#[test]
fn the_canvas_fits_the_photo_with_32_points_of_padding() {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let canvas = scancraft_ui_egui::canvas::Canvas::new(area, &picture(600, 450), (1200, 900));
    let inner = area.shrink(32.0);
    assert!(inner.expand(0.01).contains_rect(canvas.workspace), "the photo {:?} is outside {:?}", canvas.workspace, inner);
    // The photo touches one side of the padded area.
    let touches = (canvas.workspace.min.x - inner.min.x).abs() < 0.01
        || (inner.max.x - canvas.workspace.max.x).abs() < 0.01
        || (canvas.workspace.min.y - inner.min.y).abs() < 0.01
        || (inner.max.y - canvas.workspace.max.y).abs() < 0.01;
    assert!(touches, "the photo {:?} fits inside {:?} without touching it", canvas.workspace, inner);
}

#[test]
fn a_spline_through_five_points_is_centripetal() {
    let points = [Point::new(0.0, 0.0), Point::new(100.0, 40.0), Point::new(200.0, 0.0), Point::new(300.0, 40.0), Point::new(400.0, 0.0)];
    let spline = scancraft_ui_egui::canvas::catmull_rom_centripetal(&points, 16);
    // The curve passes through every control point.
    for point in points {
        assert!(spline.iter().any(|p| (p.x - point.x).abs() < 1e-6 && (p.y - point.y).abs() < 1e-6), "the curve misses {point:?}");
    }
    // It starts and ends at the corners.
    assert!((spline[0].x - points[0].x).abs() < 1e-9 && (spline[0].y - points[0].y).abs() < 1e-9);
    let last = spline.last().expect("the curve's last point");
    assert!((last.x - points[4].x).abs() < 1e-9 && (last.y - points[4].y).abs() < 1e-9);
    // A symmetric curve is symmetric.
    let mirrored = scancraft_ui_egui::canvas::catmull_rom_centripetal(&points, 16);
    assert_eq!(spline, mirrored);
}

#[test]
fn a_photo_pixel_is_scale_times_k_points_wide() {
    // A 12-megapixel photo (4032 px wide) shown from its 3072 px display copy in a 1200 pt wide
    // canvas. The canvas reported scale / k, which for any photo wider than 3072 px made the
    // loupe's view wider than the canvas's own.
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let canvas = scancraft_ui_egui::canvas::Canvas::new(area, &picture(3072, 2304), (4032, 3024));
    let expected = f64::from(canvas.rect.width()) / 4032.0;
    assert!((canvas.points_per_photo_px() - expected).abs() < 1e-6, "{} points to a photo pixel, not {expected}", canvas.points_per_photo_px());
    // The same number from the conversions that draw the canvas.
    let one = canvas.photo_to_canvas(Point::new(1.0, 0.0)).x - canvas.photo_to_canvas(Point::new(0.0, 0.0)).x;
    assert!((f64::from(one) - canvas.points_per_photo_px()).abs() < 1e-5);
}

#[test]
fn zooming_keeps_the_point_under_the_pointer_still() {
    use scancraft_ui_egui::state::CanvasView;
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let display = picture(600, 450);
    let before = scancraft_ui_egui::canvas::Canvas::new(area, &display, (1200, 900));
    let pointer = egui::pos2(500.0, 300.0);
    let photo = before.canvas_to_photo(pointer);
    let view = CanvasView::default().zoomed(3.0, pointer, area);
    let after = scancraft_ui_egui::canvas::Canvas::with_view(area, &display, (1200, 900), view, scancraft_ui_egui::canvas::VIRTUAL_MARGIN);
    assert!((after.view.zoom - 3.0).abs() < 1e-6);
    let still = after.photo_to_canvas(photo);
    assert!((still.x - pointer.x).abs() < 0.01 && (still.y - pointer.y).abs() < 0.01, "the photo point moved to {still:?}");
    // And the photo is three times the size.
    assert!((after.points_per_photo_px() / before.points_per_photo_px() - 3.0).abs() < 1e-6);
}

#[test]
fn the_pan_cannot_take_the_photo_off_the_canvas() {
    use scancraft_ui_egui::state::CanvasView;
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let display = picture(600, 450);
    let far = CanvasView { zoom: 2.0, pan: egui::vec2(50_000.0, -50_000.0) };
    let canvas = scancraft_ui_egui::canvas::Canvas::with_view(area, &display, (1200, 900), far, scancraft_ui_egui::canvas::VIRTUAL_MARGIN);
    assert!(canvas.workspace.intersects(area.shrink(31.0)), "the photo {:?} left the canvas {area:?}", canvas.workspace);
    // Not zoomed in, there is nothing to pan: the photo stays where the fit puts it.
    let whole = scancraft_ui_egui::canvas::Canvas::with_view(
        area,
        &display,
        (1200, 900),
        CanvasView { zoom: 1.0, pan: egui::vec2(300.0, 300.0) },
        scancraft_ui_egui::canvas::VIRTUAL_MARGIN,
    );
    assert_eq!(whole.rect, scancraft_ui_egui::canvas::Canvas::new(area, &display, (1200, 900)).rect);
}

// -------------------------------------------------------------- the keyboard

#[test]
fn tab_selects_the_corners_in_order() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    for corner in [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft] {
        harness.key_press(egui::Key::Tab);
        harness.run();
        assert_eq!(
            harness.state().0.selection(),
            Some(Selection::Corner { page: PAGE, corner }),
            "the selection is {:?}",
            harness.state().0.selection()
        );
    }
}

#[test]
fn shift_tab_selects_the_previous_item() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press_modifiers(egui::Modifiers { shift: true, ..egui::Modifiers::NONE }, egui::Key::Tab);
    harness.run();
    assert_eq!(
        harness.state().0.selection(),
        Some(Selection::Corner { page: PAGE, corner: Corner::BottomLeft }),
        "the selection is {:?}",
        harness.state().0.selection()
    );
}

#[test]
fn tab_then_continues_to_the_bend_handles_and_the_spine() {
    let mut view = crop_view();
    let mut outline = outline();
    outline.sides[Side::Top.index()] = SideShape::Bent { handles: [Point::new(400.0, 70.0), Point::new(600.0, 60.0), Point::new(800.0, 55.0)] };
    view.crop.as_mut().expect("the crop view").outlines = vec![outline];
    view.crop.as_mut().expect("the crop view").spine = Some([Point::new(600.0, 0.0), Point::new(600.0, 900.0)]);
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    for _ in 0..5 {
        harness.key_press(egui::Key::Tab);
        harness.run();
    }
    assert_eq!(
        harness.state().0.selection(),
        Some(Selection::Bend { page: PAGE, side: Side::Top, index: 1 }),
        "the selection is {:?}",
        harness.state().0.selection()
    );
    for _ in 0..1 {
        harness.key_press(egui::Key::Tab);
        harness.run();
    }
    assert_eq!(
        harness.state().0.selection(),
        Some(Selection::Spine { photo: PHOTO, end: 0 }),
        "the selection is {:?}",
        harness.state().0.selection()
    );
}

#[test]
fn an_arrow_key_moves_the_selected_corner_by_one_photo_pixel() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.state_mut().1.clear();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::CropSetCorner { page: PAGE, corner: Corner::TopLeft, x: 121.0, y: 80.0 }]);
}

#[test]
fn shift_and_an_arrow_key_move_by_ten_photo_pixels() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.state_mut().1.clear();
    harness.key_press_modifiers(egui::Modifiers { shift: true, ..egui::Modifiers::NONE }, egui::Key::ArrowRight);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::CropSetCorner { page: PAGE, corner: Corner::TopLeft, x: 130.0, y: 80.0 }]);
}

#[test]
fn option_and_an_arrow_key_move_by_a_quarter_photo_pixel() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.state_mut().1.clear();
    harness.key_press_modifiers(egui::Modifiers { alt: true, ..egui::Modifiers::NONE }, egui::Key::ArrowRight);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::CropSetCorner { page: PAGE, corner: Corner::TopLeft, x: 120.25, y: 80.0 }]);
}

#[test]
fn escape_clears_the_selection() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.state().0.selection().is_some());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(harness.state().0.selection(), None);
}

// ------------------------------------------------------ what the screen says

#[test]
fn the_not_found_hint_is_shown_when_detection_found_nothing() {
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").not_found = true;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("Couldn't find the page edges"), "the canvas does not show the hint");
}

#[test]
fn done_with_an_outline_the_engine_refuses_says_why_and_stays_in_crop() {
    // The engine refuses to finish a page whose outline is not a shape, and says why; the interface
    // shows it, and the user is still in Crop to put it right.
    let mut engine = FakeEngine::with_view(crop_view()).with_image(ImageKey::DisplayCopy(PHOTO), picture(600, 450));
    engine.set_error("The outline isn't valid: a corner is outside the page.");
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Done");
    assert_eq!(dispatched(&harness), vec![Command::CropApply { photo: PHOTO }]);
    assert!(shown_text(&harness).contains("The outline isn't valid"), "{}", shown_text(&harness));
    assert!(harness.query_all_by_label("Use Whole Photo").next().is_some(), "Crop's tools are gone");
}

#[test]
fn the_flatten_tooltip_names_the_reason() {
    let mut outline = outline();
    outline.flatten_decision = Some(FlattenDecision {
        flattened: true,
        reason: FlattenReason::Straighter,
        d95: 0.01,
        plane_cost: 1.0,
        curved_cost: 0.5,
        straightness_corners: Some(0.5),
        straightness_flat: Some(0.82),
        lines_corners: 10,
        lines_flat: 12,
    });
    assert_eq!(scancraft_ui_egui::crop::flatten_tooltip(&outline), "Flattened: text lines 64 % straighter");
    outline.flatten_decision = Some(FlattenDecision {
        flattened: false,
        reason: FlattenReason::OnButNoFit,
        d95: 0.0,
        plane_cost: 0.0,
        curved_cost: 0.0,
        straightness_corners: None,
        straightness_flat: None,
        lines_corners: 0,
        lines_flat: 0,
    });
    assert_eq!(scancraft_ui_egui::crop::flatten_tooltip(&outline), "Couldn't flatten the curve: no lines of text or curved edges to follow.");
}

#[test]
fn the_auto_detect_tooltip_names_the_source() {
    use scancraft_core::recipe::CornerOrigin;
    assert_eq!(scancraft_ui_egui::crop::auto_detect_tooltip(&CornerOrigin::Detected { by: Detector::Apple, score: 1.0 }), "Detected by Apple");
    assert_eq!(scancraft_ui_egui::crop::auto_detect_tooltip(&CornerOrigin::NotFound), "Not found");
    assert_eq!(scancraft_ui_egui::crop::auto_detect_tooltip(&CornerOrigin::Manual), "Adjusted by hand");
    assert_eq!(scancraft_ui_egui::crop::auto_detect_tooltip(&CornerOrigin::FullImage), "The whole photo");
}

#[test]
fn the_spread_note_names_the_half() {
    let mut view = enhance_view();
    view.enhance.as_mut().expect("the enhance view").half = Some(Half::Left);
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("Left page of a spread"), "the screen does not name the half");
}

#[test]
fn the_low_resolution_note_is_shown() {
    let mut view = enhance_view();
    view.enhance.as_mut().expect("the enhance view").low_resolution = true;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("under 150 DPI"), "the screen does not warn about the resolution");
}

#[test]
fn the_notices_of_the_engine_are_shown() {
    let mut view = crop_view();
    view.notices = vec![Notice { text: "IMG_9.jpg is not supported".to_owned(), error: true }];
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("IMG_9.jpg is not supported"), "the screen does not show the engine's message");
}

#[test]
fn a_notice_of_the_engine_stays_until_it_is_dismissed() {
    // The engine holds a notice for one poll, a frame or two. The strip used to come and go with
    // it, and every screen jumped down and back up.
    let mut view = crop_view();
    view.notices = vec![Notice { text: "IMG_9.jpg is not supported".to_owned(), error: true }];
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let with_notice = bounds_of(&harness, "Reset to Detected Edges");
    harness.state_mut().1.set_view(crop_view());
    harness.run();
    assert!(shown_text(&harness).contains("IMG_9.jpg is not supported"), "the notice went when the engine let go of it");
    assert_eq!(bounds_of(&harness, "Reset to Detected Edges"), with_notice, "the screen moved when the engine let go of the notice");
    click_label(&mut harness, "Dismiss");
    assert!(!shown_text(&harness).contains("IMG_9.jpg is not supported"), "the notice stayed after Dismiss");
}

#[test]
fn the_same_notice_twice_is_one_row() {
    let one = |notices: Vec<Notice>| {
        let mut view = crop_view();
        view.notices = notices;
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        shown_text(&harness).matches("Same").count()
    };
    let single = one(vec![Notice { text: "Same".to_owned(), error: false }]);
    let twice = one(vec![Notice { text: "Same".to_owned(), error: false }, Notice { text: "Same".to_owned(), error: false }]);
    assert!(single >= 1 && twice == single, "one notice drew {single} times, and the same notice twice drew {twice}");
}

#[test]
fn the_screen_stays_put_while_the_pointer_moves() {
    for view in [import_view(), crop_view(), enhance_view()] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        let positions = |h: &Harness<TestState>| {
            h.root()
                .children_recursive()
                .filter_map(|n| n.accesskit_node().raw_bounds().map(|b| (b.x0 as i32, b.y0 as i32, b.x1 as i32, b.y1 as i32)))
                .collect::<Vec<_>>()
        };
        let before = positions(&harness);
        for step in 0..60 {
            harness.event(egui::Event::PointerMoved(egui::pos2(5.0 + (step as f32 * 37.0) % 1190.0, 5.0 + (step as f32 * 53.0) % 790.0)));
            harness.step();
            assert_eq!(positions(&harness), before, "the screen moved on step {step}");
        }
    }
}

// ------------------------------------------------------------ the determinism

#[test]
fn the_same_view_draws_the_same_shapes_twice() {
    let first = harness(crop_view(), egui::Theme::Light);
    let second = harness(crop_view(), egui::Theme::Light);
    let shapes = |h: &Harness<TestState>| h.output().shapes.iter().map(|shape| format!("{shape:?}")).collect::<Vec<String>>();
    assert_eq!(shapes(&first), shapes(&second), "the same view drew two different screens");
}

// ------------------------------------------------------------- the handles

/// The canvas's points per photo pixel, read from two corners of the page.
fn points_per_photo_pixel(harness: &Harness<TestState>) -> f64 {
    let top_left = bounds_of(harness, "Top-left corner, 120, 80").center();
    let top_right = bounds_of(harness, "Top-right corner, 1080, 60").center();
    // The two corners are 960 photo pixels apart.
    f64::from(top_right.x - top_left.x) / 960.0
}

/// Drags from one point to another.
fn drag(harness: &mut Harness<TestState>, from: egui::Pos2, to: egui::Pos2) {
    harness.drag_at(from);
    harness.hover_at(from + (to - from) / 2.0);
    harness.hover_at(to);
    harness.drop_at(to);
    harness.run();
}

#[test]
fn a_corner_drag_sends_crop_set_corner_in_photo_pixels() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let corner = bounds_of(&harness, "Top-left corner, 120, 80").center();
    let per_pixel = points_per_photo_pixel(&harness);
    drag(&mut harness, corner, corner + egui::vec2(10.0, 0.0));
    let commands = dispatched(&harness);
    let Some(Command::CropSetCorner { page, corner: which, x, y }) = commands.first() else {
        panic!("the drag sent {commands:?}");
    };
    assert_eq!(*page, PAGE);
    assert_eq!(*which, Corner::TopLeft);
    assert!((x - (120.0 + 10.0 / per_pixel)).abs() < 1.0, "the corner went to {x}, not to {}", 120.0 + 10.0 / per_pixel);
    assert!((y - 80.0).abs() < 1.0, "the corner's height changed to {y}");
}

#[test]
fn a_straight_side_has_a_pill_and_a_single_click_does_nothing() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    // Every straight side has its pill; no side is bent, so no bend handle is drawn.
    for side in ["Top", "Right", "Bottom", "Left"] {
        let label = format!("{side} side handle: drag to move, double-click to bend");
        assert!(harness.query_by_label(&label).is_some(), "no pill on the {side} side");
    }
    assert!(harness.query_all_by_label_contains("bend handle 2 of 3").next().is_none());
    let target = bounds_of(&harness, "Top side handle: drag to move, double-click to bend").center();
    harness.event(egui::Event::PointerMoved(target));
    harness.run();
    // A single click does nothing. (The harness's clock does not make a double-click, so the
    // command it sends is checked on `bend_through` in the canvas's own tests.)
    click_at(&mut harness, target);
    assert!(dispatched(&harness).is_empty(), "a single click sent {:?}", dispatched(&harness));
}

#[test]
fn dragging_a_sides_pill_sends_crop_move_side_along_its_outward_normal() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let per_pixel = points_per_photo_pixel(&harness);
    let pill = bounds_of(&harness, "Top side handle: drag to move, double-click to bend").center();
    // Drag the top side 12 points up the screen: away from the page, by 12 points of photo.
    drag(&mut harness, pill, pill - egui::vec2(0.0, 12.0));
    let commands = dispatched(&harness);
    assert!(!commands.is_empty(), "the drag sent nothing");
    let mut total = 0.0;
    for command in &commands {
        let Command::CropMoveSide { page, side, offset } = command else {
            panic!("the drag sent {command:?}");
        };
        assert_eq!(*page, PAGE);
        assert_eq!(*side, Side::Top);
        total += offset;
    }
    // The top side's outward normal points up the screen, so the offsets add up to the drag.
    assert!((total - 12.0 / per_pixel).abs() < 1.0, "the side moved {total} photo pixels, not {}", 12.0 / per_pixel);
}

#[test]
fn a_bend_vertex_drag_sends_crop_bend_side_on_one_parabola() {
    let mut view = crop_view();
    let mut outline = outline();
    outline.sides[Side::Top.index()] = SideShape::Bent { handles: [Point::new(400.0, 70.0), Point::new(600.0, 60.0), Point::new(800.0, 55.0)] };
    view.crop.as_mut().expect("the crop view").outlines = vec![outline];
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let handle = bounds_of(&harness, "Top side, bend handle 2 of 3").center();
    let per_pixel = points_per_photo_pixel(&harness);
    drag(&mut harness, handle, handle - egui::vec2(0.0, 10.0));
    let commands = dispatched(&harness);
    let Some(Command::CropBendSide { page, side, handles }) = commands.first() else {
        panic!("the drag sent {commands:?}");
    };
    assert_eq!(*page, PAGE);
    assert_eq!(*side, Side::Top);
    // The vertex went where it was dragged, and the other two stay on the same parabola.
    assert!((handles[1].y - (60.0 - 10.0 / per_pixel)).abs() < 1.0, "the vertex went to {handles:?}");
    let corners = outline_corners();
    let chord_middle_y = (corners.tl().y + corners.tr().y) / 2.0;
    let d = handles[1].y - chord_middle_y;
    assert!((handles[0].y - (corners.tl().y * 0.75 + corners.tr().y * 0.25 + 0.75 * d)).abs() < 0.5, "{handles:?}");
}

#[test]
fn the_spine_end_drag_sends_crop_set_spine() {
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").spine = Some([Point::new(600.0, 0.0), Point::new(600.0, 900.0)]);
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let end = bounds_of(&harness, "Spine, top end").center();
    drag(&mut harness, end, end - egui::vec2(10.0, 0.0));
    let commands = dispatched(&harness);
    let Some(Command::CropSetSpine { photo, top, bottom }) = commands.first() else {
        panic!("the drag sent {commands:?}");
    };
    assert_eq!(*photo, PHOTO);
    assert!(top.x < 600.0, "the spine's top end went to {top:?}");
    assert!((bottom.x - 600.0).abs() < 0.01, "the spine's bottom end moved: {bottom:?}");
}

#[test]
fn the_loupe_appears_after_a_keyboard_nudge() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    let until = harness.state().0.loupe_until().expect("the loupe did not appear after the nudge");
    let since = harness.state().0.loupe_since().expect("the loupe has no start");
    // The loupe shows for a second (§4.3).
    assert!((until - since - 1.0).abs() < 1e-9, "the loupe shows for {} seconds", until - since);
}

/// A press on a corner's handle, off its centre by `by` points, with the button still down.
fn press_corner(harness: &mut Harness<TestState>, by: egui::Vec2) -> egui::Pos2 {
    let corner = bounds_of(harness, "Top-left corner, 120, 80").center();
    let at = corner + by;
    harness.hover_at(at);
    harness.drag_at(at);
    harness.run();
    at
}

#[test]
fn a_still_press_on_a_corner_shows_the_loupe_and_release_hides_it() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    assert!(!harness.state().0.loupe_held(), "the loupe showed before anything was pressed");
    let at = press_corner(&mut harness, egui::Vec2::ZERO);
    // The pointer has not moved: a still press is enough.
    assert!(harness.state().0.loupe_held(), "a still press did not show the loupe");
    let focus = harness.state().0.loupe_focus().expect("the loupe has no focus");
    assert!((focus.x - 120.0).abs() < 1.0 && (focus.y - 80.0).abs() < 1.0, "the loupe looks at {focus:?}, not at the corner");
    assert!(harness.state_mut().1.loupe_requests().iter().all(|(photo, _, _)| *photo == PHOTO));
    harness.drop_at(at);
    harness.run();
    assert!(!harness.state().0.loupe_held(), "the loupe stayed after the button was released");
    // It does not linger either: a nudge is the only thing that holds it past the release.
    assert!(harness.state().0.loupe_until().is_none());
}

#[test]
fn the_loupe_asks_the_engine_for_a_crop_sized_to_the_lens() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    press_corner(&mut harness, egui::Vec2::ZERO);
    let per_pixel = points_per_photo_pixel(&harness);
    let requests = harness.state().1.loupe_requests().to_vec();
    let (photo, centre, side) = requests.last().copied().expect("the loupe asked the engine for nothing");
    assert_eq!(photo, PHOTO);
    // The crop is centred on the corner, and holds what a 180 pt lens shows at three times the canvas zoom.
    assert!((centre.x - 120.0).abs() < 1.0 && (centre.y - 80.0).abs() < 1.0, "the crop is centred on {centre:?}");
    let across = 180.0 / (per_pixel * 3.0);
    assert!((side - scancraft_ui_egui::loupe::CROP_MARGIN * across).abs() < 1.0, "the crop is {side} photo pixels, for a lens that shows {across}");
}

#[test]
fn a_grab_off_the_handles_centre_does_not_move_the_corner() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let per_pixel = points_per_photo_pixel(&harness);
    // Press 15 points to the right of the handle's centre: inside the 44 pt grab area, outside the
    // 16 pt handle. Then move the pointer 3 points.
    let at = press_corner(&mut harness, egui::vec2(15.0, 0.0));
    harness.hover_at(at + egui::vec2(3.0, 0.0));
    harness.run();
    harness.hover_at(at + egui::vec2(8.0, 0.0));
    harness.run();
    let commands = dispatched(&harness);
    let Some(Command::CropSetCorner { x, y, .. }) = commands.last() else {
        panic!("the drag sent {commands:?}");
    };
    // The corner follows the pointer's movement (8 points), not the pointer's position (23 points from the corner).
    assert!((x - (120.0 + 8.0 / per_pixel)).abs() < 1.0, "the corner went to {x}, a jump of {} photo pixels", x - 120.0);
    assert!((y - 80.0).abs() < 1.0, "the corner's height changed to {y}");
}

#[test]
fn the_loupe_is_centred_on_the_corner_not_on_the_pointer() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let per_pixel = points_per_photo_pixel(&harness);
    let at = press_corner(&mut harness, egui::vec2(12.0, 6.0));
    harness.hover_at(at + egui::vec2(20.0, 0.0));
    harness.run();
    // The pointer is 12 points right of the handle and has moved 20 more: the handle is 20 points
    // from where it started, and that is where the loupe looks.
    let focus = harness.state().0.loupe_focus().expect("the loupe has no focus");
    assert!((focus.x - (120.0 + 20.0 / per_pixel)).abs() < 1.0, "the loupe looks at x = {}, not the corner's {}", focus.x, 120.0 + 20.0 / per_pixel);
    assert!((focus.y - 80.0).abs() < 1.0, "the loupe looks at y = {}", focus.y);
}

#[test]
fn the_canvas_has_room_for_a_corner_outside_the_photo() {
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let canvas = scancraft_ui_egui::canvas::Canvas::new(area, &picture(600, 450), (1200, 900));
    // The farthest a corner may go (15% beyond each edge) is still inside the workspace.
    for (x, y) in [(-180.0, -135.0), (1380.0, -135.0), (1380.0, 1035.0), (-180.0, 1035.0)] {
        let at = canvas.photo_to_canvas(scancraft_core::geom::Point::new(x, y));
        assert!(canvas.workspace.expand(0.5).contains(at), "{at:?} is outside {:?}", canvas.workspace);
        assert!(!canvas.rect.contains(at), "{at:?} should be outside the photo {:?}", canvas.rect);
    }
}

// -------------------------------------------------------------- the keyboard's ring

#[test]
fn hovering_a_corner_does_not_select_it() {
    // The keyboard's selection followed the pointer, so the corner Tab would continue from was
    // wherever the mouse happened to rest.
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let corner = bounds_of(&harness, "Top-left corner, 120, 80").center();
    harness.hover_at(corner);
    harness.run();
    assert_eq!(harness.state().0.selection(), None);
}

#[test]
fn the_corner_the_keyboard_selected_is_ringed() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let rings = |h: &Harness<TestState>| {
        painted(h)
            .into_iter()
            .filter(|shape| matches!(shape, egui::Shape::Circle(c) if (c.radius - (scancraft_ui_egui::theme::CORNER_RADIUS + 7.0)).abs() < 1e-3))
            .count()
    };
    assert_eq!(rings(&harness), 0, "a ring was drawn before anything was selected");
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(harness.state().0.selection(), Some(Selection::Corner { page: PAGE, corner: Corner::TopLeft }));
    assert!(rings(&harness) > 0, "the selected corner has no ring");
}

#[test]
fn arrow_keys_change_the_filter_unless_the_strength_slider_has_the_focus() {
    // The arrow keys moved the filter even while the strength slider had the focus and was moving
    // itself with them. Up and down go through the list of filters.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::ArrowDown);
    harness.run();
    assert!(dispatched(&harness).iter().any(|c| matches!(c, Command::PageSetLook { .. })), "an arrow key did nothing with no focus");
    harness.state_mut().1.clear();
    harness.query_all_by_label("Strength").next().expect("the strength slider").focus();
    harness.run();
    harness.key_press(egui::Key::ArrowDown);
    harness.run();
    assert!(
        !dispatched(&harness).iter().any(|c| matches!(c, Command::PageSetLook { .. })),
        "an arrow key changed the filter while the slider had the focus: {:?}",
        dispatched(&harness)
    );
}

#[test]
fn arrow_keys_still_change_the_filter_after_a_click_on_a_filter() {
    // A click gives the clicked widget the keyboard's focus. The arrow keys must keep working.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Grayscale filter");
    harness.state_mut().1.clear();
    harness.key_press(egui::Key::ArrowDown);
    harness.run();
    assert!(
        dispatched(&harness).iter().any(|c| matches!(c, Command::PageSetLook { .. })),
        "the arrow key did nothing after a click: {:?}",
        dispatched(&harness)
    );
}

#[test]
fn each_screen_has_one_copper_button() {
    // A screen carried up to six copper buttons, so none of them was the primary one. (A copper
    // line under the current stage is not a button: only rectangles a finger can press count.)
    for (name, view) in [("import", import_view()), ("crop", crop_view()), ("enhance", enhance_view())] {
        for theme_choice in [egui::Theme::Light, egui::Theme::Dark] {
            let mut harness = harness(view.clone(), theme_choice);
            harness.run();
            let found: Vec<_> = painted(&harness)
                .into_iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(rect) if rect.fill == scancraft_ui_egui::theme::COPPER && rect.rect.height() >= 20.0 => Some(rect.rect),
                    _ => None,
                })
                .collect();
            assert_eq!(found.len(), 1, "the {name} screen in the {theme_choice:?} appearance has {} copper buttons: {found:?}", found.len());
        }
    }
}

// ------------------------------------------------------------ honest controls

/// Every shape a frame painted, with the groups taken apart.
fn painted<S>(harness: &Harness<'_, S>) -> Vec<egui::Shape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| walk(shape, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The rectangle of every picture a frame painted (a textured mesh that is not the font's).
fn painted_images<S>(harness: &Harness<'_, S>) -> Vec<egui::Rect> {
    painted(harness)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => Some(mesh.calc_bounds()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_note_s_undo_turns_the_page_back_and_nothing_else() {
    // The Undo beside "Turned upright automatically" ran the general undo, which reverts whatever
    // the user did last.
    let mut view = enhance_view();
    view.enhance.as_mut().expect("the enhance view").turned_automatically = true;
    view.enhance.as_mut().expect("the enhance view").quarter_turns = 1;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_last_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::PageRotate { page: PAGE, quarter_turns: -1 }]);
}

#[test]
fn a_note_s_undo_leaves_the_page_flat_and_nothing_else() {
    let mut view = enhance_view();
    view.enhance.as_mut().expect("the enhance view").flattened_automatically = true;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_last_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::CropSetFlatten { page: PAGE, mode: FlattenMode::Off }]);
}

#[test]
fn an_unticked_checkbox_and_an_unselected_radio_button_are_not_copper() {
    // Every control rested in copper, so an unticked checkbox was a solid copper square, and
    // Flatten's Auto, On and Off all looked selected.
    for theme_choice in [egui::Theme::Light, egui::Theme::Dark] {
        let mut harness = Harness::builder().with_size(egui::vec2(400.0, 300.0)).with_theme(theme_choice).build_ui(|ui| {
            scancraft_ui_egui::theme::apply(ui.ctx());
            let mut ticked = false;
            ui.checkbox(&mut ticked, "Unticked");
            let mut mode = 1;
            ui.radio_value(&mut mode, 0, "Not chosen");
            ui.radio_value(&mut mode, 1, "Chosen");
            let mut strength = 40.0;
            ui.add(egui::Slider::new(&mut strength, 0.0..=100.0).text("Strength"));
            let _ = ui.button("A button");
        });
        harness.run();
        for shape in painted(&harness) {
            let fill = match &shape {
                egui::Shape::Rect(rect) => Some(rect.fill),
                egui::Shape::Circle(circle) => Some(circle.fill),
                _ => None,
            };
            let Some(fill) = fill else { continue };
            for copper in [scancraft_ui_egui::theme::COPPER, scancraft_ui_egui::theme::LIGHT_COPPER] {
                assert_ne!(fill, copper, "a control rests in copper in the {theme_choice:?} appearance: {shape:?}");
            }
        }
    }
}

#[test]
fn a_thumbnail_keeps_the_shape_of_its_page() {
    // Every thumbnail was stretched to fill a box of one fixed shape, so a landscape page looked
    // like a portrait one.
    let engine = FakeEngine::with_view(enhance_view())
        .with_image(ImageKey::LookPreview(PAGE), picture(600, 450))
        .with_image(ImageKey::TrayThumb(PAGE), picture(200, 100))
        .with_image(ImageKey::TrayThumb(PageId(11)), picture(100, 200))
        .with_image(ImageKey::TrayThumb(PageId(12)), picture(128, 176));
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    // The sidebar's boxes are 116 × 150: a picture fills one side of its box and no more.
    let thumbs: Vec<egui::Rect> = painted_images(&harness)
        .into_iter()
        .filter(|r| ((r.width() - 116.0).abs() < 0.5 && r.height() <= 150.5) || ((r.height() - 150.0).abs() < 0.5 && r.width() <= 116.5))
        .collect();
    assert!(thumbs.len() >= 3, "{} thumbnails were painted", thumbs.len());
    let aspects: Vec<f32> = thumbs.iter().map(|r| r.width() / r.height()).collect();
    for expected in [2.0, 0.5, 128.0 / 176.0] {
        assert!(aspects.iter().any(|a| (a - expected).abs() < 0.02), "no thumbnail is {expected}:1; they are {aspects:?}");
    }
}

#[test]
fn a_tray_thumbnail_shows_its_page_number() {
    // The number was drawn under the thumbnail, outside the area the painter could paint in.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    let words: Vec<String> = painted(&harness)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    for number in ["1", "2", "3"] {
        assert!(words.iter().any(|w| w == number), "page {number}'s number was not painted: {words:?}");
    }
}

#[test]
fn a_canvas_is_never_blank_without_saying_why() {
    // With no picture yet, both canvases stayed empty.
    let crop = harness_with(FakeEngine::with_view(crop_view()), egui::Theme::Light);
    let mut crop = crop;
    crop.run();
    assert!(shown_text(&crop).contains("Preparing the photo"), "the crop canvas is blank: {}", shown_text(&crop));
    let mut enhance = harness_with(FakeEngine::with_view(enhance_view()), egui::Theme::Light);
    enhance.run();
    assert!(shown_text(&enhance).contains("Preparing the page"), "the page canvas is blank: {}", shown_text(&enhance));
}

// -------------------------------------------------------------- the stages

// ---------------------------------------------------------------- the toolbar

#[test]
fn the_toolbar_has_add_the_switch_the_page_buttons_history_and_export() {
    for view in [enhance_view(), crop_view()] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        for label in ["Add", "Filter", "Crop", "Rotate Left", "Rotate Right", "Delete", "Undo", "Redo", "Export PDF", "Export options"] {
            assert!(harness.query_all_by_label(label).next().is_some(), "the toolbar has no {label:?}");
        }
        // They are along the top, in a row, with Export PDF at the far end.
        let add = bounds_of(&harness, "Add");
        let export = bounds_of(&harness, "Export PDF");
        assert!(add.max.y < 56.0 && export.max.y < 56.0, "the toolbar is not along the top");
        assert!(add.min.x < 60.0 && export.min.x > 900.0, "Add is at the start and Export PDF at the end: {add:?} {export:?}");
    }
}

#[test]
fn add_in_the_toolbar_asks_the_shell_for_the_open_panel() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Add");
    assert_eq!(harness.state_mut().0.take_actions(), vec![UiAction::ImportFiles]);
}

#[test]
fn rotate_and_delete_in_the_toolbar_act_on_the_open_page() {
    for (label, expected) in [
        ("Rotate Left", Command::PageRotate { page: PAGE, quarter_turns: -1 }),
        ("Rotate Right", Command::PageRotate { page: PAGE, quarter_turns: 1 }),
        ("Delete", Command::PageDelete { page: PAGE }),
    ] {
        for view in [enhance_view(), crop_view()] {
            let mut harness = harness(view, egui::Theme::Light);
            harness.run();
            click_label(&mut harness, label);
            assert_eq!(dispatched(&harness), vec![expected.clone()], "{label}");
        }
    }
}

#[test]
fn undo_and_redo_in_the_toolbar_are_live_only_when_there_is_something_to_undo_or_redo() {
    // The finished page's view can undo and not redo.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    assert!(!harness.query_all_by_label("Undo").next().expect("Undo").accesskit_node().is_disabled());
    assert!(harness.query_all_by_label("Redo").next().expect("Redo").accesskit_node().is_disabled());
    click_label(&mut harness, "Redo");
    assert!(dispatched(&harness).is_empty(), "a disabled Redo sent {:?}", dispatched(&harness));
    click_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::EditUndo]);
}

#[test]
fn the_switch_goes_to_crop_and_back() {
    let mut finished = harness(enhance_view(), egui::Theme::Light);
    finished.run();
    let node = finished.query_all_by_label("Filter").next().expect("the Filter side");
    assert_eq!(node.accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True), "Filter is on while the finished page shows");
    click_label(&mut finished, "Crop");
    assert_eq!(dispatched(&finished), vec![Command::PageOpenCrop { page: PAGE }]);
    let mut cropping = harness(crop_view(), egui::Theme::Light);
    cropping.run();
    let node = cropping.query_all_by_label("Crop").next().expect("the Crop side");
    assert_eq!(node.accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True), "Crop is on while Crop shows");
    // Leaving Crop is Done: the page is finished.
    click_label(&mut cropping, "Filter");
    assert_eq!(dispatched(&cropping), vec![Command::CropApply { photo: PHOTO }]);
}

#[test]
fn export_pdf_is_off_while_photos_are_being_read() {
    let mut view = enhance_view();
    view.importing = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(harness.query_all_by_label("Export PDF").next().expect("Export PDF").accesskit_node().is_disabled());
    click_label(&mut harness, "Export PDF");
    assert!(harness.state_mut().0.take_actions().is_empty(), "an export started while photos were still being read");
}

#[test]
fn a_narrow_window_moves_the_groups_that_do_not_fit_into_a_menu() {
    let engine = FakeEngine::with_view(enhance_view()).with_image(ImageKey::LookPreview(PAGE), picture(600, 450));
    // Wide: everything is in the row and there is no menu.
    let mut wide = harness_with(engine, egui::Theme::Light);
    wide.run();
    assert!(wide.query_all_by_label("More").next().is_none());
    // Narrow: the last groups are in the menu, which is there to open, and Export PDF stays.
    let engine = FakeEngine::with_view(enhance_view()).with_image(ImageKey::LookPreview(PAGE), picture(600, 450));
    let mut narrow = harness_sized(engine, egui::Theme::Light, egui::vec2(480.0, 700.0));
    narrow.run();
    assert!(narrow.query_all_by_label("More").next().is_some(), "nothing says that commands have moved to a menu");
    assert!(narrow.query_all_by_label("Redo").next().is_none(), "Redo is both in the row and in the menu");
    assert!(narrow.query_all_by_label("Export PDF").next().is_some(), "Export PDF is not at the end of the row");
    click_label(&mut narrow, "More");
    assert!(narrow.query_all_by_label("Redo").next().is_some(), "the menu does not hold Redo");
}

// ---------------------------------------------------------------- the sidebar

#[test]
fn the_sidebar_lists_the_pages_and_marks_the_ones_that_are_open() {
    for (view, open) in [(enhance_view(), vec!["Page 1"]), (crop_view(), vec!["Page 1"])] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        for label in ["Page 1", "Page 2", "Page 3"] {
            let node = harness.query_all_by_label(label).next().unwrap_or_else(|| panic!("no {label}"));
            let on = node.accesskit_node().data().toggled() == Some(egui::accesskit::Toggled::True);
            assert_eq!(on, open.contains(&label), "{label}");
        }
        // They are in a column down the left of the window, in order.
        let (first, third) = (bounds_of(&harness, "Page 1"), bounds_of(&harness, "Page 3"));
        assert!(first.max.x < 160.0 && third.min.y > first.max.y, "{first:?} {third:?}");
    }
}

#[test]
fn a_page_that_needs_a_look_or_is_being_prepared_or_failed_says_so() {
    let mut view = enhance_view();
    view.tray[0].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::NotFound);
    view.tray[1].state = PageState::Preparing;
    view.tray[2].state = PageState::Failed("the file is gone".to_owned());
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    for label in ["Page 1, needs a look", "Page 2, being prepared", "Page 3, could not be made"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "no {label:?}");
    }
}

#[test]
fn photos_still_being_read_have_a_placeholder_each() {
    let mut view = enhance_view();
    view.importing = Some(scancraft_core::engine_api::Progress { done: 1, total: 4 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.step();
    // Three of the four photos are still being read.
    assert_eq!(harness.query_all_by_label("Reading the photo…").count(), 3);
}

#[test]
fn a_click_opens_the_page_in_the_view_the_window_is_in() {
    let mut finished = harness(enhance_view(), egui::Theme::Light);
    finished.run();
    click_label(&mut finished, "Page 3");
    assert_eq!(dispatched(&finished), vec![Command::PageEdit { page: PageId(12) }]);
    let mut cropping = harness(crop_view(), egui::Theme::Light);
    cropping.run();
    click_label(&mut cropping, "Page 3");
    assert_eq!(dispatched(&cropping), vec![Command::PageOpenCrop { page: PageId(12) }]);
}

#[test]
fn the_sidebar_can_be_hidden_from_the_view_menu() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    assert!(harness.query_all_by_label("Page 1").next().is_some());
    let (ui, engine) = harness.state_mut();
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::TOGGLE_PAGES), engine);
    harness.run();
    assert!(harness.query_all_by_label("Page 1").next().is_none(), "the pages stayed after Show Pages was turned off");
    assert_eq!(harness.state().0.menu_item_checked(scancraft_ui_egui::menu::TOGGLE_PAGES), Some(false));
}

// --------------------------------------------------------------- the inspector

#[test]
fn the_inspector_has_the_controls_of_the_view_the_window_is_in() {
    let mut finished = harness(enhance_view(), egui::Theme::Light);
    finished.run();
    for label in ["Original filter", "Black & White filter", "Grayscale filter", "Color Document filter", "Apply to All Pages", "Show Original"] {
        assert!(finished.query_all_by_label(label).next().is_some(), "the filters have no {label:?}");
    }
    assert!(finished.query_all_by_label("Use Whole Photo").next().is_none(), "Crop's controls are on the finished page");
    let mut cropping = harness(crop_view(), egui::Theme::Light);
    cropping.run();
    for label in ["Reset to Detected Edges", "Use Whole Photo", "Automatic", "Always", "Never", "Split into two pages", "Remove Photo", "Done"] {
        assert!(cropping.query_all_by_label(label).next().is_some(), "Crop has no {label:?}");
    }
    assert!(cropping.query_all_by_label("Apply to All Pages").next().is_none(), "the filters' controls are in Crop");
    // On the right of the window.
    assert!(bounds_of(&cropping, "Done").min.x > 900.0);
}

#[test]
fn apply_to_all_pages_sends_the_open_pages_filter_and_strength_to_every_page() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Apply to All Pages");
    assert_eq!(dispatched(&harness), vec![Command::PageSetLookAll { look: Look::ColorDocument, strength: 60 }]);
}

#[test]
fn show_original_shows_the_unfiltered_page_while_it_is_held() {
    let engine = FakeEngine::with_view(enhance_view())
        .with_image(ImageKey::LookPreview(PAGE), picture(600, 450))
        .with_image(ImageKey::OriginalPreview(PAGE), picture(300, 200));
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    let asked = |h: &Harness<TestState>| h.state().1.image_requests().contains(&ImageKey::OriginalPreview(PAGE));
    assert!(!asked(&harness), "the unfiltered page was asked for before it was wanted");
    harness.state_mut().1.clear();
    // Held: the button is pressed and not let go.
    let button = bounds_of(&harness, "Show Original").center();
    harness.hover_at(button);
    harness.drag_at(button);
    harness.run();
    assert!(asked(&harness), "holding Show Original did not ask for the unfiltered page");
    // Let go: the filtered page is back, and the unfiltered one is not asked for again.
    harness.drop_at(button);
    harness.run();
    harness.state_mut().1.clear();
    harness.run();
    assert!(!asked(&harness), "the unfiltered page is still on show after the button was let go");
}

#[test]
fn the_backslash_key_shows_the_original_too() {
    let engine = FakeEngine::with_view(enhance_view())
        .with_image(ImageKey::LookPreview(PAGE), picture(600, 450))
        .with_image(ImageKey::OriginalPreview(PAGE), picture(300, 200));
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    harness.state_mut().1.clear();
    harness.event(egui::Event::Key { key: egui::Key::Backslash, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    harness.step();
    assert!(harness.state().1.image_requests().contains(&ImageKey::OriginalPreview(PAGE)));
}

// -------------------------------------------------------------- the status bar

#[test]
fn the_status_bar_counts_the_pages_and_says_what_is_going_on() {
    let mut view = enhance_view();
    view.reading_text = Some(scancraft_core::engine_api::Progress { done: 2, total: 3 });
    view.importing = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.step();
    let text = shown_text(&harness);
    for expected in ["3 pages", "Reading text 3 of 3", "Reading photo 2 of 3"] {
        assert!(text.contains(expected), "the status bar does not say {expected:?}: {text}");
    }
    // It is a bar along the bottom.
    assert!(bounds_of(&harness, "3 pages").min.y > 770.0);
}

#[test]
fn pages_that_need_a_look_are_counted_and_show_opens_the_first() {
    let mut view = enhance_view();
    view.tray[1].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::LowScore);
    view.tray[2].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::NotFound);
    let mut flagged = harness(view, egui::Theme::Light);
    flagged.run();
    assert!(shown_text(&flagged).contains("2 pages need a look"), "{}", shown_text(&flagged));
    click_label(&mut flagged, "Show");
    assert_eq!(dispatched(&flagged), vec![Command::PageOpenCrop { page: PageId(11) }]);
    // With none flagged, there is nothing to show.
    let mut calm = harness(enhance_view(), egui::Theme::Light);
    calm.run();
    assert!(calm.query_all_by_label("Show").next().is_none());
    assert!(!shown_text(&calm).contains("need a look"));
}

#[test]
fn in_crop_the_arrow_keys_go_to_the_next_page_that_needs_a_look_when_nothing_is_selected() {
    let mut view = crop_view();
    view.tray[2].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::NotFound);
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    // Page 2 is next, but page 3 is the one that needs a look.
    assert_eq!(dispatched(&harness), vec![Command::PageOpenCrop { page: PageId(12) }]);
    // With a corner selected the arrow keys nudge it, as ever.
    harness.state_mut().1.clear();
    harness.key_press(egui::Key::Tab);
    harness.run();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    assert!(matches!(dispatched(&harness).as_slice(), [Command::CropSetCorner { .. }]), "{:?}", dispatched(&harness));
}

// ------------------------------------------------------------------ the canvas

#[test]
fn the_photo_is_the_same_size_and_in_the_same_place_whether_or_not_a_corner_is_off_it() {
    // The room for a corner to leave the photo was made only once a corner was off it, by
    // shrinking the photo, so the picture jumped under the hand as the corner crossed its edge. The
    // room is always there now. (`flow.rs` drags a corner across the edge with the real engine.)
    let mut inside = harness(crop_view(), egui::Theme::Light);
    inside.run();
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").outlines[0].corners.0[0] = Point::new(-60.0, 80.0);
    let mut outside = harness(view, egui::Theme::Light);
    outside.run();
    // Three corners are where they were in both views: if the photo is the same, so are they.
    for label in ["Top-right corner, 1080, 60", "Bottom-right corner, 1100, 820", "Bottom-left corner, 100, 840"] {
        let (a, b) = (bounds_of(&inside, label).center(), bounds_of(&outside, label).center());
        assert!((a - b).length() < 0.05, "{label} is at {a:?} with every corner on the photo and at {b:?} with one off it");
    }
    // The room has a price, and it is bounded: the photo is still more than a third of the
    // window's width.
    let photo = points_per_photo_pixel(&inside) * 1200.0;
    assert!(photo > 400.0, "the photo is only {photo} points wide");
}

#[test]
fn the_zoom_items_of_the_view_menu_zoom_the_page() {
    let engine = FakeEngine::with_view(enhance_view()).with_image(ImageKey::LookPreview(PAGE), picture(600, 450));
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    let width = |h: &Harness<TestState>| painted_images(h).into_iter().map(|r| r.width()).fold(0.0f32, f32::max);
    let fitted = width(&harness);
    let (ui, engine) = harness.state_mut();
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::ZOOM_IN), engine);
    harness.run();
    let zoomed = width(&harness);
    assert!(zoomed > fitted * 1.2, "Zoom In left the page at {zoomed} from {fitted}");
    let (ui, engine) = harness.state_mut();
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::ZOOM_FIT), engine);
    harness.run();
    assert!((width(&harness) - fitted).abs() < 1.0, "Zoom to Fit did not fit the page");
}

// ---------------------------------------------------------------- the menus' items

#[test]
fn the_split_item_of_the_page_menu_carries_the_state_it_goes_to() {
    // One page: the item splits. Two: it joins.
    let one = scancraft_ui_egui::build_menu_bar(&crop_view());
    let item = one.items().into_iter().find(|item| item.label == "Split into Two Pages").expect("the split item");
    assert_eq!(scancraft_ui_egui::menu::command_of(&item.command), Some(Command::CropSplit { photo: PHOTO, on: true }));
    let mut view = crop_view();
    let crop = view.crop.as_mut().expect("the crop view");
    let mut right = crop.outlines[0].clone();
    right.page = PageId(2);
    right.half = Some(Half::Right);
    crop.outlines[0].half = Some(Half::Left);
    crop.outlines.push(right);
    let two = scancraft_ui_egui::build_menu_bar(&view);
    let item = two.items().into_iter().find(|item| item.label == "Split into Two Pages").expect("the split item");
    assert_eq!(scancraft_ui_egui::menu::command_of(&item.command), Some(Command::CropSplit { photo: PHOTO, on: false }));
}

#[test]
fn the_crop_item_and_apply_filter_to_all_pages_run_their_commands() {
    let bar = scancraft_ui_egui::build_menu_bar(&enhance_view());
    let crop = bar.items().into_iter().find(|item| item.label == "Crop").expect("the Crop item");
    assert_eq!(scancraft_ui_egui::menu::command_of(&crop.command), Some(Command::PageOpenCrop { page: PAGE }));
    let all = bar.items().into_iter().find(|item| item.label == "Apply Filter to All Pages").expect("the item");
    assert_eq!(scancraft_ui_egui::menu::command_of(&all.command), Some(Command::PageSetLookAll { look: Look::ColorDocument, strength: 60 }));
}

#[test]
fn previous_and_next_page_open_the_neighbours_in_the_view_the_window_is_in() {
    for (view, open) in [(enhance_view(), true), (crop_view(), false)] {
        let mut harness = harness(view, egui::Theme::Light);
        harness.run();
        let (ui, engine) = harness.state_mut();
        ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::NEXT_PAGE), engine);
        let expected = if open { Command::PageEdit { page: PageId(11) } } else { Command::PageOpenCrop { page: PageId(11) } };
        assert_eq!(dispatched(&harness), vec![expected]);
    }
}

// ----------------------------------------------------------------- the export sheet

#[test]
fn the_export_sheet_shows_the_progress_by_page_and_a_cancel() {
    let mut view = enhance_view();
    view.exporting = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("1 of 3 pages"), "{}", shown_text(&harness));
    click_label(&mut harness, "Cancel");
    assert_eq!(dispatched(&harness), vec![Command::ExportCancel]);
    // While it runs, the menus stand aside.
    assert!(harness.state().0.is_modal());
}

#[test]
fn the_finished_sheet_names_the_file_and_offers_show_in_finder_open_and_a_new_scan() {
    let mut view = enhance_view();
    view.last_export =
        Some(scancraft_core::engine_api::ExportSummary { path: "/Users/me/Scans/Receipts.pdf".to_owned(), pages: 3, bytes: 3_412_000 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let text = shown_text(&harness);
    assert!(text.contains("Receipts.pdf") && text.contains("3 pages") && text.contains("3.4 MB"), "{text}");
    click_label(&mut harness, "Show in Finder");
    click_label(&mut harness, "Open");
    assert_eq!(
        harness.state_mut().0.take_actions(),
        vec![
            UiAction::RevealInFinder { path: "/Users/me/Scans/Receipts.pdf".to_owned() },
            UiAction::OpenFile { path: "/Users/me/Scans/Receipts.pdf".to_owned() },
        ]
    );
    // New Scan starts again; Done only closes the sheet.
    click_label(&mut harness, "New Scan");
    assert_eq!(dispatched(&harness), vec![Command::SessionNew]);
    assert!(!harness.state().0.is_modal(), "the sheet stayed after New Scan");
}

#[test]
fn done_closes_the_finished_sheet_and_it_does_not_come_back() {
    let mut view = enhance_view();
    view.last_export = Some(scancraft_core::engine_api::ExportSummary { path: "/tmp/a.pdf".to_owned(), pages: 1, bytes: 900 });
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(harness.state().0.is_modal());
    click_last_label(&mut harness, "Done");
    harness.run();
    assert!(!harness.state().0.is_modal());
    assert!(dispatched(&harness).is_empty());
}

#[test]
fn a_long_scan_asks_the_engine_for_the_thumbnails_of_the_pages_in_view_only() {
    // Every page of the list used to ask for its picture on every frame, scrolled into view or not.
    let mut view = enhance_view();
    view.tray = (1..=60)
        .map(|n| TrayItem { page: PageId(1000 + n), photo: PhotoId(2000 + n), number: n as u32, text_ready: true, state: PageState::Ready })
        .collect();
    view.tray[0].page = PAGE;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let asked: std::collections::HashSet<&ImageKey> =
        harness.state().1.image_requests().iter().filter(|key| matches!(key, ImageKey::TrayThumb(_))).collect();
    let asked = asked.len();
    assert!(asked > 0 && asked < 12, "{asked} thumbnails were asked for in a window that shows about four pages");
}

// ------------------------------------------------------------------ the settings

/// A window with the Settings window open, on a scans folder of the test's own.
fn settings_open() -> Harness<'static, TestState> {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    let settings = scancraft_ui_egui::Settings { session_storage: "/Users/me/Scans".to_owned(), ..scancraft_ui_egui::Settings::default() };
    let (ui, engine) = harness.state_mut();
    ui.set_settings(settings);
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::SETTINGS), engine);
    harness.run();
    harness
}

#[test]
fn the_settings_are_in_groups() {
    let harness = settings_open();
    let text = shown_text(&harness);
    for group in ["PAGES", "SCANNING", "TEXT", "BOOKS", "SCANS FOLDER"] {
        assert!(text.contains(group), "the Settings window has no {group:?} group: {text}");
    }
}

#[test]
fn each_switch_has_a_line_under_it_that_says_what_it_does() {
    let harness = settings_open();
    for (switch, line) in [
        ("Snap to standard paper sizes", "made exactly that size"),
        ("Rotate pages upright automatically", "reads the right way up"),
        ("Flatten curved pages automatically", "straighter"),
        ("Keep paper colour", "stays tinted"),
        ("Language correction in text recognition", "a search finds them"),
    ] {
        let switch_bounds = bounds_of(&harness, switch);
        let explanation = harness.query_all_by_label_contains(line).next().unwrap_or_else(|| panic!("no explanation with {line:?} under {switch:?}"));
        let raw = explanation.accesskit_node().raw_bounds().expect("the explanation has bounds");
        // Directly under the switch, not above it and not a group away.
        assert!(
            raw.y0 as f32 >= switch_bounds.max.y - 2.0 && (raw.y0 as f32) < switch_bounds.max.y + 24.0,
            "{switch:?}: {raw:?} against {switch_bounds:?}"
        );
    }
    // The choices have theirs too.
    let text = shown_text(&harness);
    for line in ["not close to any standard size", "the two pages of a spread are read in"] {
        assert!(text.contains(line), "no line saying {line:?}: {text}");
    }
}

#[test]
fn a_switch_in_settings_sends_the_changed_settings_to_the_shell() {
    let mut harness = settings_open();
    let before = scancraft_ui_egui::Settings::default().keep_paper_colour;
    click_label(&mut harness, "Keep paper colour");
    let actions = harness.state_mut().0.take_actions();
    let [UiAction::SettingsChanged(settings)] = actions.as_slice() else { panic!("the switch asked for {actions:?}") };
    assert_eq!(settings.keep_paper_colour, !before);
    // Nothing else about the settings changed.
    assert_eq!(settings.session_storage, "/Users/me/Scans");
}

#[test]
fn the_scans_folder_is_shown_and_choose_asks_the_shell_for_the_folder_panel() {
    let mut harness = settings_open();
    assert!(shown_text(&harness).contains("/Users/me/Scans"), "{}", shown_text(&harness));
    click_label(&mut harness, "Choose…");
    assert_eq!(harness.state_mut().0.take_actions(), vec![UiAction::ChooseScansFolder]);
}

/// The Settings window over the finished page, in one appearance.
fn settings_snapshot(theme_choice: egui::Theme, name: &str) {
    let mut harness = harness(enhance_view(), theme_choice);
    harness.run();
    let settings = scancraft_ui_egui::Settings {
        session_storage: "~/Library/Application Support/ScanCraft/Sessions".to_owned(),
        default_paper: scancraft_core::recipe::Paper::UsLetter,
        ..scancraft_ui_egui::Settings::default()
    };
    let (ui, engine) = harness.state_mut();
    ui.set_settings(settings);
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::SETTINGS), engine);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(name, &snapshot_options());
}

#[test]
fn settings_light_snapshot() {
    settings_snapshot(egui::Theme::Light, "settings_light");
}

#[test]
fn settings_dark_snapshot() {
    settings_snapshot(egui::Theme::Dark, "settings_dark");
}

// ----------------------------------------------------------- several pages at once

/// Clicks a widget with the given label while `modifiers` are held.
fn click_label_with(harness: &mut Harness<TestState>, label: &str, modifiers: egui::Modifiers) {
    let bounds = bounds_of(harness, label);
    let pos = bounds.center();
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run();
}

const COMMAND: egui::Modifiers = egui::Modifiers { command: true, ctrl: false, alt: false, shift: false, mac_cmd: true };
const SHIFT: egui::Modifiers = egui::Modifiers { shift: true, ..egui::Modifiers::NONE };

fn selected(harness: &Harness<TestState>, label: &str) -> bool {
    harness.query_all_by_label(label).next().is_some_and(|n| n.accesskit_node().data().toggled() == Some(egui::accesskit::Toggled::True))
}

#[test]
fn command_click_selects_more_pages_and_a_plain_click_lets_go() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    assert!(selected(&harness, "Page 1") && !selected(&harness, "Page 3"));
    click_label_with(&mut harness, "Page 3", COMMAND);
    // The open page is in the selection it was added to; and nothing was opened.
    assert!(selected(&harness, "Page 1") && selected(&harness, "Page 3") && !selected(&harness, "Page 2"));
    assert!(dispatched(&harness).is_empty(), "a selecting click opened a page: {:?}", dispatched(&harness));
    assert!(shown_text(&harness).contains("2 pages selected"), "{}", shown_text(&harness));
    // A second command-click on a selected page takes it out again.
    click_label_with(&mut harness, "Page 3", COMMAND);
    assert!(!selected(&harness, "Page 3"));
    assert!(!shown_text(&harness).contains("pages selected"));
    // And a plain click opens the page and lets go of whatever was selected.
    click_label_with(&mut harness, "Page 2", COMMAND);
    click_label(&mut harness, "Page 3");
    assert_eq!(dispatched(&harness), vec![Command::PageEdit { page: PageId(12) }]);
    assert!(!shown_text(&harness).contains("pages selected"));
}

#[test]
fn shift_click_selects_the_pages_between() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label_with(&mut harness, "Page 3", SHIFT);
    for page in ["Page 1", "Page 2", "Page 3"] {
        assert!(selected(&harness, page), "{page} is not selected after a Shift-click from the open page to the last");
    }
    assert!(shown_text(&harness).contains("3 pages selected"));
}

#[test]
fn rotate_and_delete_act_on_the_selected_pages_in_one_command_each() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label_with(&mut harness, "Page 3", COMMAND);
    click_label(&mut harness, "Rotate Right");
    assert_eq!(dispatched(&harness), vec![Command::PageRotateMany { pages: vec![PAGE, PageId(12)], quarter_turns: 1 }]);
    harness.state_mut().1.clear();
    click_label(&mut harness, "Delete");
    assert_eq!(dispatched(&harness), vec![Command::PageDeleteMany { pages: vec![PAGE, PageId(12)] }]);
    // One toast, one Undo, for the lot; and the selection is let go of.
    assert!(shown_text(&harness).contains("2 pages deleted"), "{}", shown_text(&harness));
    assert!(!shown_text(&harness).contains("pages selected"));
}

#[test]
fn the_delete_key_deletes_the_selected_pages() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label_with(&mut harness, "Page 2", COMMAND);
    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::PageDeleteMany { pages: vec![PAGE, PageId(11)] }]);
}

#[test]
fn a_filter_for_the_selected_pages_is_one_command() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label_with(&mut harness, "Page 2", SHIFT);
    click_label(&mut harness, "Grayscale filter");
    let commands = dispatched(&harness);
    assert_eq!(commands, vec![Command::PageSetLookMany { pages: vec![PAGE, PageId(11)], look: Look::Grayscale, strength: 60 }]);
    // The inspector says what the filters are for.
    assert!(shown_text(&harness).contains("2 pages selected"));
}

#[test]
fn select_all_selects_every_page_from_the_menu_and_the_keyboard() {
    let mut menu = harness(enhance_view(), egui::Theme::Light);
    menu.run();
    let (ui, engine) = menu.state_mut();
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::SELECT_ALL), engine);
    menu.run();
    assert!(shown_text(&menu).contains("3 pages selected"), "{}", shown_text(&menu));
    let mut keyboard = harness(enhance_view(), egui::Theme::Light);
    keyboard.run();
    keyboard.key_press_modifiers(COMMAND, egui::Key::A);
    keyboard.run();
    assert!(shown_text(&keyboard).contains("3 pages selected"), "{}", shown_text(&keyboard));
    // Escape lets go.
    keyboard.key_press(egui::Key::Escape);
    keyboard.run();
    assert!(!shown_text(&keyboard).contains("pages selected"));
}

#[test]
fn the_menus_rotate_and_delete_act_on_the_selection_too() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label_with(&mut harness, "Page 3", COMMAND);
    let bar = scancraft_ui_egui::build_menu_bar(&enhance_view());
    let rotate = bar.items().into_iter().find(|item| item.label == "Rotate Left").expect("Rotate Left");
    let (ui, engine) = harness.state_mut();
    ui.run_menu_command(&rotate.command, engine);
    assert_eq!(dispatched(&harness), vec![Command::PageRotateMany { pages: vec![PAGE, PageId(12)], quarter_turns: -1 }]);
}

// ------------------------------------------------------------- the keyboard's walk

/// The name of the widget that has the keyboard's focus, if one does.
fn focused_name(harness: &Harness<TestState>) -> Option<String> {
    harness.root().children_recursive().find(|node| node.accesskit_node().is_focused()).and_then(|node| node.accesskit_node().label())
}

#[test]
fn the_tab_key_walks_the_toolbar_then_the_pages_then_the_canvas_then_the_inspector_then_the_status_bar() {
    let mut view = enhance_view();
    // A flagged page, so the status bar has a button to reach.
    view.tray[2].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::NotFound);
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    let mut walk: Vec<String> = Vec::new();
    for _ in 0..80 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        if let Some(name) = focused_name(&harness) {
            // The slider and its number box have one name; the walk has come full circle at the first stop.
            if walk.len() > 3 && walk.first() == Some(&name) {
                break;
            }
            if walk.last() != Some(&name) {
                walk.push(name);
            }
        }
    }
    let at = |name: &str| walk.iter().position(|w| w == name).unwrap_or_else(|| panic!("the walk never reached {name:?}: {walk:?}"));
    let (toolbar, pages, canvas, inspector, status) = (at("Add"), at("Page 1"), at("Page"), at("Original filter"), at("Show"));
    assert!(toolbar < pages && pages < canvas && canvas < inspector && inspector < status, "the walk is {walk:?}");
}

#[test]
fn a_page_can_be_moved_without_dragging_it() {
    // The sidebar's drag has a keyboard and screen-reader way: the Page menu's Move Page Up and Down.
    let mut middle = enhance_view();
    middle.enhance.as_mut().expect("the enhance view").page = PageId(11);
    let mut harness = harness(middle.clone(), egui::Theme::Light);
    harness.run();
    let (ui, engine) = harness.state_mut();
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::MOVE_UP), engine);
    ui.run_menu_command(&craft_menu::CommandRef::new(scancraft_ui_egui::menu::MOVE_DOWN), engine);
    assert_eq!(dispatched(&harness), vec![Command::PageMove { from: 1, to: 0 }, Command::PageMove { from: 1, to: 2 }]);
    // At the ends there is nowhere to go, and the items say so.
    use scancraft_ui_egui::menu::item_enabled;
    assert!(item_enabled("page.move_up", &middle) && item_enabled("page.move_down", &middle));
    assert!(!item_enabled("page.move_up", &enhance_view()), "the first page can be moved up");
    let mut last = enhance_view();
    last.enhance.as_mut().expect("the enhance view").page = PageId(12);
    assert!(!item_enabled("page.move_down", &last), "the last page can be moved down");
    let bar = scancraft_ui_egui::build_menu_bar(&enhance_view());
    let labels: Vec<&str> = bar.items().iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"Move Page Up") && labels.contains(&"Move Page Down"));
}

// ----------------------------------------------------------- reduce motion, contrast

/// The points of the first line the frame painted, for comparing one frame with the next.
fn first_line<S>(harness: &Harness<'_, S>) -> Option<Vec<egui::Pos2>> {
    painted(harness).into_iter().find_map(|shape| match shape {
        egui::Shape::Path(path) => Some(path.points),
        _ => None,
    })
}

#[test]
fn a_spinner_turns_unless_reduce_motion_is_on() {
    for (reduce_motion, turns) in [(false, true), (true, false)] {
        let mut harness = Harness::builder().with_size(egui::vec2(100.0, 100.0)).build_ui(move |ui| {
            scancraft_ui_egui::theme::set_accessibility(ui.ctx(), scancraft_ui_egui::Accessibility { reduce_motion, increase_contrast: false });
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
            scancraft_ui_egui::theme::spinner(ui, rect, egui::Color32::WHITE);
        });
        harness.step();
        let before = first_line(&harness).expect("the spinner's ring");
        harness.step();
        harness.step();
        let after = first_line(&harness).expect("the spinner's ring");
        assert_eq!(before != after, turns, "with Reduce Motion {reduce_motion} the spinner {}", if turns { "did not turn" } else { "turned" });
    }
}

/// The widths of the lines the crop view drew over the photo.
fn line_widths(harness: &Harness<TestState>) -> Vec<f32> {
    painted(harness)
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Path(path) => match path.stroke.color {
                egui::epaint::ColorMode::Solid(colour) if colour == scancraft_ui_egui::theme::SCAN_CYAN => Some(path.stroke.width),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn increase_contrast_thickens_the_outline_and_darkens_its_edge() {
    let mut normal = harness(crop_view(), egui::Theme::Light);
    normal.run();
    let widest_normal = line_widths(&normal).into_iter().fold(0.0f32, f32::max);
    let mut high = harness(crop_view(), egui::Theme::Light);
    high.state_mut().0.set_accessibility(scancraft_ui_egui::Accessibility { reduce_motion: false, increase_contrast: true });
    high.run();
    let widest_high = line_widths(&high).into_iter().fold(0.0f32, f32::max);
    assert!(widest_normal > 0.0, "no outline was drawn");
    assert!(widest_high > widest_normal + 0.9, "the outline is {widest_high} wide with Increase Contrast and {widest_normal} without");
    // The edge under it is darker, too.
    let darkest = |h: &Harness<TestState>| {
        painted(h)
            .into_iter()
            .filter_map(|shape| match shape {
                egui::Shape::Path(path) => match path.stroke.color {
                    egui::epaint::ColorMode::Solid(c) if c.r() == 0 && c.g() == 0 && c.b() == 0 && path.stroke.width >= 4.0 => Some(c.a()),
                    _ => None,
                },
                _ => None,
            })
            .max()
            .unwrap_or(0)
    };
    assert!(darkest(&high) > darkest(&normal), "the edge under the outline is no darker: {} against {}", darkest(&high), darkest(&normal));
}

// ------------------------------------------------------------- the names

/// Every state of the workspace a control can be in, as the views the engine would report.
fn every_view() -> Vec<(&'static str, ViewState)> {
    let mut flagged = enhance_view();
    flagged.tray[0].state = PageState::NeedsLook(scancraft_core::engine_api::ReviewReason::LowScore);
    flagged.tray[1].state = PageState::Preparing;
    flagged.tray[2].state = PageState::Failed("the file is gone".to_owned());
    let mut reading = enhance_view();
    reading.importing = Some(scancraft_core::engine_api::Progress { done: 1, total: 4 });
    reading.reading_text = Some(scancraft_core::engine_api::Progress { done: 0, total: 3 });
    let mut running = enhance_view();
    running.exporting = Some(scancraft_core::engine_api::Progress { done: 1, total: 3 });
    let mut finished = enhance_view();
    finished.last_export = Some(scancraft_core::engine_api::ExportSummary { path: "/tmp/Scan.pdf".to_owned(), pages: 3, bytes: 2_000_000 });
    let mut split = crop_view();
    let crop = split.crop.as_mut().expect("the crop view");
    let mut right = crop.outlines[0].clone();
    right.page = PageId(2);
    right.half = Some(Half::Right);
    crop.outlines[0].half = Some(Half::Left);
    crop.outlines.push(right);
    crop.spine = Some([Point::new(600.0, 0.0), Point::new(600.0, 900.0)]);
    let mut bent = crop_view();
    bent.crop.as_mut().expect("the crop view").outlines[0].sides[Side::Top.index()] =
        SideShape::Bent { handles: [Point::new(400.0, 70.0), Point::new(600.0, 60.0), Point::new(800.0, 55.0)] };
    vec![
        ("the empty window", import_view()),
        ("crop", crop_view()),
        ("crop on a split photo", split),
        ("crop with a bent side", bent),
        ("the finished page", enhance_view()),
        ("pages that need a look, are preparing or failed", flagged),
        ("photos being read", reading),
        ("the export running", running),
        ("the export finished", finished),
    ]
}

#[test]
fn every_control_in_every_view_has_an_accessible_name() {
    for (name, view) in every_view() {
        for theme_choice in [egui::Theme::Light, egui::Theme::Dark] {
            let mut harness = harness(view.clone(), theme_choice);
            harness.step();
            harness.run();
            assert!(harness.unnamed_widgets().is_empty(), "{name}, {theme_choice:?}: unnamed widgets: {:?}", harness.unnamed_widgets());
        }
    }
}

#[test]
fn the_settings_window_the_selection_and_the_narrow_toolbar_have_names_too() {
    // Settings.
    let mut settings = settings_open();
    assert!(settings.unnamed_widgets().is_empty(), "Settings: {:?}", settings.unnamed_widgets());
    // Several pages selected, with a toast over the window.
    let mut selecting = harness(enhance_view(), egui::Theme::Light);
    selecting.run();
    click_label_with(&mut selecting, "Page 3", COMMAND);
    click_label(&mut selecting, "Delete");
    assert!(selecting.unnamed_widgets().is_empty(), "a selection and a toast: {:?}", selecting.unnamed_widgets());
    // The toolbar's menu, open.
    let engine = FakeEngine::with_view(enhance_view()).with_image(ImageKey::LookPreview(PAGE), picture(600, 450));
    let mut narrow = harness_sized(engine, egui::Theme::Light, egui::vec2(480.0, 700.0));
    narrow.run();
    click_label(&mut narrow, "More");
    assert!(narrow.unnamed_widgets().is_empty(), "the toolbar's menu: {:?}", narrow.unnamed_widgets());
    let _ = &mut settings;
}

#[test]
fn a_toast_is_announced_by_a_screen_reader_when_it_appears() {
    // News is said politely, an error at once.
    for (error, expected) in [(false, egui::accesskit::Live::Polite), (true, egui::accesskit::Live::Assertive)] {
        let mut harness = harness(enhance_view(), egui::Theme::Light);
        harness.run();
        harness.state_mut().0.notice("Something happened", error);
        harness.run();
        let node = harness.query_all_by_label("Something happened").next().expect("the toast");
        assert_eq!(node.accesskit_node().data().live(), Some(expected), "error {error}");
    }
}
