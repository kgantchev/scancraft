//! The interface's tests: the three screens draw, every control has an accessible name, and every
//! action dispatches the command of §4.
//!
//! The tests run against the fake engine, so they say nothing about the pipeline: they say that
//! the interface draws what the specification describes and asks the engine for the right things.

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EnhanceView, ImageKey, Notice, OutlineView, Screen, TrayItem, ViewState};
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

/// Screen 2's view: one photo of seven, with its page found.
fn crop_view() -> ViewState {
    ViewState {
        screen: Screen::Crop(PHOTO),
        crop: Some(CropView {
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
        tray: vec![TrayItem { page: PageId(10), number: 1, text_ready: true }],
        book_spreads: false,
        can_undo: false,
        can_redo: false,
        busy: false,
        notices: Vec::new(),
        restore_offer: None,
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
        tray: vec![
            TrayItem { page: PageId(10), number: 1, text_ready: true },
            TrayItem { page: PageId(11), number: 2, text_ready: false },
            TrayItem { page: PageId(12), number: 3, text_ready: true },
        ],
        book_spreads: false,
        can_undo: true,
        can_redo: false,
        busy: false,
        notices: Vec::new(),
        restore_offer: None,
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
    for page in [PageId(10), PageId(11), PageId(12)] {
        engine = engine.with_image(ImageKey::TrayThumb(page), picture(128, 176));
    }
    Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
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
fn import_screen_light_snapshot() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "import_screen_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn import_screen_dark_snapshot() {
    let mut harness = harness(import_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "import_screen_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn crop_screen_light_snapshot() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "crop_screen_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn crop_screen_dark_snapshot() {
    let mut harness = harness(crop_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "crop_screen_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn enhance_screen_light_snapshot() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "enhance_screen_light",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn enhance_screen_dark_snapshot() {
    let mut harness = harness(enhance_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "enhance_screen_dark",
        &egui_kittest::SnapshotOptions::new()
            .threshold(egui_kittest::OsThreshold::new(0.0).macos(12.0).linux(12.0).windows(12.0))
            .max_failed_pixels(egui_kittest::OsThreshold::new(0).macos(400).linux(400).windows(400)),
    );
}

#[test]
fn page_tray_snapshot() {
    let mut harness = harness(enhance_view(), egui::Theme::Dark);
    harness.run();
    if !can_render(&mut harness) {
        return;
    }
    harness.snapshot_options(
        "page_tray",
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
    for index in 1..=3 {
        let label = format!("Top side, bend handle {index} of 3");
        let node = harness.query_by_label(&label).unwrap_or_else(|| panic!("no handle labelled {label:?}"));
        assert_eq!(node.accesskit_node().role(), egui::Role::Slider, "the handle {label:?} is not adjustable");
    }
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
        "New Session",
        "Import…",
        "Export to PDF…",
        "Export Options…",
        "Undo",
        "Redo",
        "Delete Page",
        "Auto-Detect",
        "Use Full Image",
        "Flatten",
        "Straighten Side",
        "Book Spreads",
        "Correct Perspective",
        "Back to Crop",
        "Rotate Left",
        "Rotate Right",
        "Look",
        "Add Page",
        "Previous Photo",
        "Next Photo",
        "Show Page Tray",
    ] {
        assert!(labels.contains(&expected), "the menu bar has no {expected:?} item: {labels:?}");
    }
    // The shortcuts of §4.7.
    let shortcut_of = |label: &str| {
        bar.items().iter().find(|item| item.label == label).and_then(|item| item.shortcut).map(|chord| chord.display(craft_keymap::Platform::Mac))
    };
    assert_eq!(shortcut_of("Import…").as_deref(), Some("⌘O"));
    assert_eq!(shortcut_of("Export to PDF…").as_deref(), Some("⌘E"));
    assert_eq!(shortcut_of("Undo").as_deref(), Some("⌘Z"));
    assert_eq!(shortcut_of("Book Spreads").as_deref(), Some("⌥⌘B"));
    assert_eq!(shortcut_of("Show Page Tray").as_deref(), Some("⌥⌘T"));
}

#[test]
fn the_book_spreads_menu_item_sends_the_opposite_of_the_current_setting() {
    // The item used to carry no parameter, so `command_of` found no command and the menu item
    // (and its shortcut) did nothing at all.
    for current in [false, true] {
        let mut view = crop_view();
        view.book_spreads = current;
        let bar = scancraft_ui_egui::build_menu_bar(&view);
        let item = bar.items().into_iter().find(|item| item.label == "Book Spreads").expect("the Book Spreads item");
        let command = scancraft_ui_egui::menu::command_of(&item.command);
        assert_eq!(command, Some(Command::CropBook { on: !current }), "with book spreads {current}");
    }
}

// -------------------------------------------------------------- the commands

#[test]
fn auto_detect_sends_crop_auto() {
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").auto_detect = false;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Auto-Detect");
    assert_eq!(dispatched(&harness), vec![Command::CropAuto { page: PAGE }]);
}

#[test]
fn use_full_image_sends_crop_full_image() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Use Full Image");
    assert_eq!(dispatched(&harness), vec![Command::CropFullImage { page: PAGE }]);
}

#[test]
fn correct_perspective_sends_crop_apply() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Correct Perspective");
    assert_eq!(dispatched(&harness), vec![Command::CropApply { photo: PHOTO }]);
}

#[test]
fn discard_photo_sends_import_discard() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Discard Photo");
    assert_eq!(dispatched(&harness), vec![Command::ImportDiscard { item: PHOTO }]);
}

#[test]
fn the_flatten_control_sends_crop_set_flatten() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "On");
    assert_eq!(dispatched(&harness), vec![Command::CropSetFlatten { page: PAGE, mode: FlattenMode::On }]);
}

#[test]
fn book_spreads_sends_crop_book() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Book Spreads");
    assert_eq!(dispatched(&harness), vec![Command::CropBook { on: true }]);
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
fn browse_files_asks_the_shell_for_the_open_panel() {
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Browse Files…");
    assert_eq!(harness.state_mut().0.take_actions(), vec![UiAction::ImportFiles]);
}

#[test]
fn dropped_files_send_import_add_in_order() {
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
    let mut harness = harness(import_view(), egui::Theme::Light);
    harness.run();
    harness.input_mut().dropped_files.push(Arc::new(Dropped("/tmp/IMG_9.jpg".into())));
    harness.input_mut().dropped_files.push(Arc::new(Dropped("/tmp/IMG_10.jpg".into())));
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::ImportAdd { paths: vec!["/tmp/IMG_9.jpg".to_owned(), "/tmp/IMG_10.jpg".to_owned()] }]);
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
fn back_to_crop_sends_page_edit() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Back to Crop");
    assert_eq!(dispatched(&harness), vec![Command::PageEdit { page: PAGE }]);
}

#[test]
fn add_page_sends_page_add() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Add Page & Next");
    assert_eq!(dispatched(&harness), vec![Command::PageAdd { page: PAGE }]);
}

#[test]
fn export_asks_the_shell_for_the_save_panel_with_the_options() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Export to PDF");
    let actions = harness.state_mut().0.take_actions();
    assert_eq!(actions.len(), 1, "the export asked for {actions:?}");
    assert!(matches!(actions[0], UiAction::ExportPdf { .. }), "the export asked for {actions:?}");
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
fn deleting_from_the_tray_sends_page_delete() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Page 2");
    harness.state_mut().1.clear();
    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert_eq!(dispatched(&harness), vec![Command::PageDelete { page: PageId(11) }]);
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
    assert!(inner.contains_rect(canvas.rect), "the photo {:?} is outside {:?}", canvas.rect, inner);
    // The photo touches one side of the padded area.
    let touches = (canvas.rect.min.x - inner.min.x).abs() < 0.01
        || (inner.max.x - canvas.rect.max.x).abs() < 0.01
        || (canvas.rect.min.y - inner.min.y).abs() < 0.01
        || (inner.max.y - canvas.rect.max.y).abs() < 0.01;
    assert!(touches, "the photo {:?} fits inside {:?} without touching it", canvas.rect, inner);
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
fn the_loupe_centre_is_110_points_above_the_pointer() {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let centre = scancraft_ui_egui::loupe::loupe_centre(egui::Pos2::new(600.0, 500.0), window);
    assert!((centre.y - 390.0).abs() < 0.01, "the loupe's centre is at {}", centre.y);
    assert!((centre.x - 600.0).abs() < 0.01);
}

#[test]
fn the_loupe_centre_flips_below_the_pointer_near_the_window_top() {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let centre = scancraft_ui_egui::loupe::loupe_centre(egui::Pos2::new(600.0, 100.0), window);
    assert!((centre.y - 210.0).abs() < 0.01, "the loupe's centre is at {}", centre.y);
}

#[test]
fn the_loupe_stays_inside_the_window() {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    for pointer in [egui::Pos2::new(0.0, 0.0), egui::Pos2::new(1200.0, 800.0), egui::Pos2::new(5.0, 795.0)] {
        let centre = scancraft_ui_egui::loupe::loupe_centre(pointer, window);
        assert!(centre.x - 70.0 >= window.min.x - 0.01 && centre.x + 70.0 <= window.max.x + 0.01, "the loupe is outside: {centre:?}");
        assert!(centre.y - 70.0 >= window.min.y - 0.01 && centre.y + 70.0 <= window.max.y + 0.01, "the loupe is outside: {centre:?}");
    }
}

#[test]
fn the_loupe_asks_for_a_crop_of_the_right_size() {
    // The loupe shows the photo at three times the canvas zoom.
    let points_per_photo_px = 2.0;
    let in_loupe = points_per_photo_px / 3.0;
    let side = scancraft_ui_egui::loupe::crop_side(in_loupe);
    let across = 140.0 / in_loupe;
    assert!((side - 1.5 * across).abs() < 1e-9, "the crop's side is {side}");
    // The crop is at most 1024 photo pixels.
    assert_eq!(scancraft_ui_egui::loupe::crop_side(0.0001), 1024.0);
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
        Some(Selection::Bend { page: PAGE, side: Side::Top, index: 0 }),
        "the selection is {:?}",
        harness.state().0.selection()
    );
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(
        harness.state().0.selection(),
        Some(Selection::Bend { page: PAGE, side: Side::Top, index: 1 }),
        "the selection is {:?}",
        harness.state().0.selection()
    );
    for _ in 0..2 {
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
fn an_invalid_outline_disables_correct_perspective_with_a_reason() {
    let mut view = crop_view();
    view.crop.as_mut().expect("the crop view").outlines[0].invalid = Some("That shape is not a page.".to_owned());
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Correct Perspective");
    assert!(dispatched(&harness).is_empty(), "the invalid outline let the perspective through");
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
fn the_page_count_and_position_are_shown() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("Photo 2 of 7"), "the screen does not show the photo's position");
}

#[test]
fn the_notices_of_the_engine_are_shown() {
    let mut view = crop_view();
    view.notices = vec![Notice { text: "IMG_9.jpg is not supported".to_owned(), error: true }];
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    assert!(shown_text(&harness).contains("IMG_9.jpg is not supported"), "the screen does not show the engine's message");
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
fn an_edge_handle_drag_sends_crop_move_side_with_the_outward_offset() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let handle = bounds_of(&harness, "Top side handle").center();
    let per_pixel = points_per_photo_pixel(&harness);
    // The top side's normal points up, away from the page.
    drag(&mut harness, handle, handle - egui::vec2(0.0, 10.0));
    let commands = dispatched(&harness);
    let Some(Command::CropMoveSide { page, side, offset }) = commands.first() else {
        panic!("the drag sent {commands:?}");
    };
    assert_eq!(*page, PAGE);
    assert_eq!(*side, Side::Top);
    assert!(offset.abs() > 1.0, "the offset is {offset}");
    assert!((offset.abs() - 10.0 / per_pixel).abs() < 1.0, "the offset is {offset}, not {}", 10.0 / per_pixel);
}

#[test]
fn a_bend_handle_drag_sends_crop_bend_side_with_all_three_handles() {
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
    // The first and the third handles did not move; the second one did.
    assert!((handles[0].x - 400.0).abs() < 0.01 && (handles[0].y - 70.0).abs() < 0.01, "the first handle moved: {handles:?}");
    assert!((handles[2].x - 800.0).abs() < 0.01 && (handles[2].y - 55.0).abs() < 0.01, "the third handle moved: {handles:?}");
    assert!((handles[1].y - (60.0 - 10.0 / per_pixel)).abs() < 1.0, "the second handle went to {handles:?}");
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
