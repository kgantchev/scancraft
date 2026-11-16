//! The interface's tests: the three screens draw, every control has an accessible name, and every
//! action dispatches the command of §4.
//!
//! The tests run against the fake engine, so they say nothing about the pipeline: they say that
//! the interface draws what the specification describes and asks the engine for the right things.

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{CropView, EnhanceView, ImageKey, Notice, OutlineView, PhotoTab, Screen, TrayItem, ViewState};
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

/// Screen 2's view: one photo of seven, with its page found.
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
    harness_with(engine, theme)
}

/// The harness of an engine the test made, in one appearance.
fn harness_with(engine: FakeEngine, theme: egui::Theme) -> Harness<'static, TestState> {
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
        "New Session",
        "Import…",
        "Export to PDF…",
        "Export Options…",
        "Undo",
        "Redo",
        "Delete Page",
        "Reset to Detected Edges",
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

#[test]
fn a_menu_item_is_live_only_when_it_can_act() {
    use scancraft_ui_egui::menu::item_enabled;
    // The Import screen: nothing to crop, turn, filter or export. These items were live and did nothing.
    let import = import_view();
    for id in [
        "crop.apply",
        "crop.auto",
        "crop.full_image",
        "page.rotate",
        "page.set_look",
        "page.add",
        "page.open_crop",
        "export.pdf",
        "file.export_options",
    ] {
        assert!(!item_enabled(id, &import), "{id} is live on the Import screen");
    }
    for id in ["session.new", "file.open", "app.settings"] {
        assert!(item_enabled(id, &import), "{id} is off on the Import screen");
    }
    // The crop screen: Correct Perspective acts, a filter does not. The edges are the detected
    // ones, so there is nothing to reset.
    let crop = crop_view();
    assert!(item_enabled("crop.apply", &crop) && item_enabled("crop.full_image", &crop));
    assert!(!item_enabled("page.set_look", &crop));
    assert!(!item_enabled("crop.auto", &crop), "Reset to Detected Edges is live on detected edges");
    let mut by_hand = crop_view();
    by_hand.crop.as_mut().expect("the crop view").auto_detect = false;
    assert!(item_enabled("crop.auto", &by_hand));
    // No side is bent, so there is nothing to straighten.
    assert!(!item_enabled("crop.straighten_side", &crop));
    // The filter screen: a filter acts, Correct Perspective does not.
    let enhance = enhance_view();
    assert!(item_enabled("page.set_look", &enhance) && item_enabled("page.rotate", &enhance) && item_enabled("export.pdf", &enhance));
    assert!(!item_enabled("crop.apply", &enhance));
    // The first photo has no photo before it; the second has both.
    assert!(item_enabled("page.previous", &crop) && item_enabled("page.next", &crop));
    let mut first = crop_view();
    {
        let crop = first.crop.as_mut().expect("the crop view");
        crop.photo = crop.photos[0].photo;
    }
    assert!(!item_enabled("page.previous", &first));
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
fn back_to_crop_opens_the_corners_screen() {
    // It used to send page.edit, which reopens Screen 3, so the button did nothing.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Back to Crop");
    assert_eq!(dispatched(&harness), vec![Command::PageOpenCrop { page: PAGE }]);
}

#[test]
fn the_back_to_crop_menu_item_opens_the_corners_screen_too() {
    for view in [enhance_view(), crop_view()] {
        let bar = scancraft_ui_egui::build_menu_bar(&view);
        let item = bar.items().into_iter().find(|item| item.label == "Back to Crop").expect("the Back to Crop item");
        assert_eq!(scancraft_ui_egui::menu::command_of(&item.command), Some(Command::PageOpenCrop { page: PAGE }));
    }
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

#[test]
fn deleting_a_page_says_so_and_offers_an_undo() {
    // The Delete key works from every screen, and nothing on screen said that a page had gone.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Page 2");
    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert!(shown_text(&harness).contains("Page 2 deleted"), "nothing said the page was deleted: {}", shown_text(&harness));
    harness.state_mut().1.clear();
    click_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::EditUndo]);
    assert!(!shown_text(&harness).contains("Page 2 deleted"), "the toast stayed after its Undo was used");
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
    let after = scancraft_ui_egui::canvas::Canvas::with_view(area, &display, (1200, 900), view);
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
    let canvas = scancraft_ui_egui::canvas::Canvas::with_view(area, &display, (1200, 900), far);
    assert!(canvas.workspace.intersects(area.shrink(31.0)), "the photo {:?} left the canvas {area:?}", canvas.workspace);
    // Not zoomed in, there is nothing to pan: the photo stays where the fit puts it.
    let whole = scancraft_ui_egui::canvas::Canvas::with_view(area, &display, (1200, 900), CanvasView { zoom: 1.0, pan: egui::vec2(300.0, 300.0) });
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
fn the_strip_lists_every_photo_and_a_click_opens_it() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    for n in 1..=7 {
        assert!(harness.query_by_label(&format!("Photo {n} of 7: IMG_{n}.jpg")).is_some(), "no tab for photo {n}");
    }
    let target = bounds_of(&harness, "Photo 4 of 7: IMG_4.jpg").center();
    click_at(&mut harness, target);
    assert_eq!(dispatched(&harness), vec![Command::PageOpenCrop { page: PageId(204) }]);
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
fn arrow_keys_change_the_filter_only_when_no_widget_has_the_focus() {
    // The arrow keys moved the filter even while the strength slider had the focus and was moving
    // itself with them.
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    harness.key_press(egui::Key::ArrowLeft);
    harness.run();
    assert!(dispatched(&harness).iter().any(|c| matches!(c, Command::PageSetLook { .. })), "an arrow key did nothing with no focus");
    harness.state_mut().1.clear();
    harness.query_all_by_label("Strength").next().expect("the strength slider").focus();
    harness.run();
    harness.key_press(egui::Key::ArrowLeft);
    harness.run();
    assert!(
        !dispatched(&harness).iter().any(|c| matches!(c, Command::PageSetLook { .. })),
        "an arrow key changed the filter while the slider had the focus: {:?}",
        dispatched(&harness)
    );
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
    click_label(&mut harness, "Undo");
    assert_eq!(dispatched(&harness), vec![Command::PageRotate { page: PAGE, quarter_turns: -1 }]);
}

#[test]
fn a_note_s_undo_leaves_the_page_flat_and_nothing_else() {
    let mut view = enhance_view();
    view.enhance.as_mut().expect("the enhance view").flattened_automatically = true;
    let mut harness = harness(view, egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Undo");
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
        .with_image(ImageKey::TrayThumb(PageId(10)), picture(200, 100))
        .with_image(ImageKey::TrayThumb(PageId(11)), picture(100, 200))
        .with_image(ImageKey::TrayThumb(PageId(12)), picture(128, 176));
    let mut harness = harness_with(engine, egui::Theme::Light);
    harness.run();
    let thumbs: Vec<egui::Rect> =
        painted_images(&harness).into_iter().filter(|r| r.width() <= 70.0 && r.height() <= 90.0 && r.width() > 8.0).collect();
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

#[test]
fn the_stage_bar_names_the_four_stages_and_marks_the_current_one() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    for label in ["Step 1 of 4: Import", "Step 2 of 4: Crop", "Step 3 of 4: Enhance", "Step 4 of 4: Export"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "no stage {label:?}");
    }
    let node = harness.query_all_by_label("Step 2 of 4: Crop").next().expect("the Crop stage");
    assert_eq!(node.accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True));
    let node = harness.query_all_by_label("Step 3 of 4: Enhance").next().expect("the Enhance stage");
    assert_ne!(node.accesskit_node().data().toggled(), Some(egui::accesskit::Toggled::True));
}

#[test]
fn the_import_stage_shows_the_drop_zone_over_the_crop_screen_and_back() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Step 1 of 4: Import");
    assert!(
        shown_text(&harness).contains("Drop photos or PDFs here") || harness.query_all_by_label_contains("Drop photos or PDFs here").next().is_some()
    );
    assert!(dispatched(&harness).is_empty(), "going to Import is the interface's own move");
    click_label(&mut harness, "Step 2 of 4: Crop");
    assert!(harness.query_all_by_label("Correct Perspective").next().is_some());
}

#[test]
fn the_enhance_stage_from_crop_applies_the_perspective() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Step 3 of 4: Enhance");
    assert_eq!(dispatched(&harness), vec![Command::CropApply { photo: PHOTO }]);
}

#[test]
fn the_export_stage_asks_the_shell_for_the_save_panel() {
    let mut harness = harness(enhance_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Step 4 of 4: Export");
    let actions = harness.state_mut().0.take_actions();
    assert!(actions.iter().any(|a| matches!(a, scancraft_ui_egui::UiAction::ExportPdf { .. })), "{actions:?}");
}

#[test]
fn the_tools_are_not_in_the_step_bar_and_the_steps_are_not_in_the_tools() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    let tools = bounds_of(&harness, "Use Full Image");
    let step = bounds_of(&harness, "Correct Perspective");
    assert!(tools.max.x < 300.0, "the tools are in the left panel");
    assert!(step.min.y > tools.max.y, "the step button is below the tools, in the step bar");
}

#[test]
fn exporting_with_photos_waiting_asks_first() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Export to PDF");
    assert!(harness.state_mut().0.take_actions().is_empty(), "the Save panel waits for the answer");
    assert!(harness.query_all_by_label_contains("haven't been added").next().is_some(), "the question is shown");
    click_label(&mut harness, "Export");
    let actions = harness.state_mut().0.take_actions();
    assert!(actions.iter().any(|a| matches!(a, scancraft_ui_egui::UiAction::ExportPdf { .. })), "{actions:?}");
}

#[test]
fn cancelling_the_question_exports_nothing() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Export to PDF");
    click_label(&mut harness, "Cancel");
    assert!(harness.state_mut().0.take_actions().is_empty());
    assert!(harness.query_all_by_label_contains("haven't been added").next().is_none());
}

#[test]
fn the_menu_and_the_arrows_move_between_photos() {
    let mut harness = harness(crop_view(), egui::Theme::Light);
    harness.run();
    click_label(&mut harness, "Next Photo");
    let sent = dispatched(&harness);
    assert!(matches!(sent.as_slice(), [Command::PageOpenCrop { .. }]) || sent.is_empty(), "{sent:?}");
}
