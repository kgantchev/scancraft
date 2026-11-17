//! The whole app's flow, end to end: the real engine behind the real interface, in a window the
//! test drives. Seven photos reach a PDF in three actions (drop them, press Export PDF, save), and
//! the page the user is on, the pages in the sidebar and the sheet at the end all come from what
//! the engine reports.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen};
use scancraft_core::geom::{Point, Quad};
use scancraft_core::image::Rgb8;
use scancraft_core::traits::{PageDetector, ServiceError};
use scancraft_engine::{Engine, EngineOptions, Services};
use scancraft_ui_egui::{ScanCraftUi, UiAction};

/// A detector that finds the sheet `desk` draws, wherever it is asked.
struct Finder(Quad);

impl PageDetector for Finder {
    fn detect(&self, _: &Rgb8) -> Result<Option<Quad>, ServiceError> {
        Ok(Some(self.0))
    }
}

/// What the app owns: the interface and the engine behind it.
struct Shell {
    ui: ScanCraftUi,
    engine: Engine,
}

/// A dark desk with a lighter sheet on it; `seed` changes the desk so each file is different.
fn desk(dir: &std::path::Path, seed: u8) -> String {
    let image = image::RgbImage::from_fn(160, 120, |x, y| {
        let on_sheet = (24..136).contains(&x) && (16..104).contains(&y);
        if on_sheet { image::Rgb([235, 232, 225]) } else { image::Rgb([40 + seed, 38, 36 + (x % 7) as u8]) }
    });
    let path = dir.join(format!("desk{seed}.png"));
    image.save(&path).expect("the picture is written");
    path.to_string_lossy().into_owned()
}

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

fn shown_text(harness: &Harness<Shell>) -> String {
    let mut text = String::new();
    for node in harness.root().children_recursive() {
        if let Some(value) = node.accesskit_node().data().value() {
            text.push_str(value);
            text.push('\n');
        }
    }
    text
}

/// Steps the window, polling the engine as the app does, until `done` holds.
fn until(harness: &mut Harness<Shell>, what: &str, done: impl Fn(&Harness<Shell>) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    while !done(harness) {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn click(harness: &mut Harness<Shell>, label: &str) {
    let bounds = {
        let node = harness.query_all_by_label(label).last().unwrap_or_else(|| panic!("no widget labelled {label:?}"));
        node.accesskit_node().raw_bounds().unwrap_or_else(|| panic!("{label:?} has no bounds"))
    };
    let pos = egui::pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.step();
    harness.step();
}

fn shell(dir: &std::path::Path) -> Harness<'static, Shell> {
    let quad = Quad([Point::new(24.0, 16.0), Point::new(136.0, 16.0), Point::new(136.0, 104.0), Point::new(24.0, 104.0)]);
    let services = Services { detector: Some(Arc::new(Finder(quad))), recognizer: None, upright: None };
    let mut options = EngineOptions::new(dir.join("session"), services);
    options.settings = craft_settings::Location::Memory;
    options.sessions_root = Some(dir.to_path_buf());
    options.threads = 2;
    let engine = Engine::open(options).expect("the engine opens");
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_context_setup(|ctx| ctx.set_fonts(egui::FontDefinitions::default())).build_ui_state(
        |ui, shell: &mut Shell| {
            shell.engine.poll();
            shell.ui.ui(ui, &mut shell.engine);
        },
        Shell { ui: ScanCraftUi::new(), engine },
    )
}

#[test]
fn seven_photos_reach_a_pdf_in_three_actions() {
    let dir = std::env::temp_dir().join(format!("scancraft-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the folder");
    let mut harness = shell(&dir);
    harness.step();
    assert!(harness.query_all_by_label("Choose Files…").next().is_some(), "the empty window is not there to start with");
    assert!(harness.query_all_by_label("Export PDF").next().is_none(), "the toolbar is up with nothing to export");

    // Action one: drop the seven photos on the window.
    for seed in 0..7 {
        let path = desk(&dir, seed);
        harness.input_mut().dropped_files.push(Arc::new(Dropped(path.into())));
    }
    until(&mut harness, "the seven pages to join", |h| h.query_all_by_label("Page 7").next().is_some() && !h.state().engine.view().busy);
    let view = harness.state().engine.view();
    assert_eq!(view.tray.len(), 7);
    // No Add Page, no second confirmation: every page is in the document, and the first is open finished.
    assert!(matches!(view.screen, Screen::Enhance(_)), "{:?}", view.screen);
    assert!(harness.query_all_by_label("Add Page").next().is_none());
    assert!(shown_text(&harness).contains("7 pages"), "{}", shown_text(&harness));
    assert!(!harness.query_all_by_label("Export PDF").next().expect("Export PDF").accesskit_node().is_disabled());

    // Action two: Export PDF, which asks the shell for the Save panel.
    click(&mut harness, "Export PDF");
    let actions = harness.state_mut().ui.take_actions();
    let [UiAction::ExportPdf { options }] = actions.as_slice() else { panic!("Export PDF asked for {actions:?}") };

    // Action three: the Save panel answers with a path, and the shell sends it to the engine.
    let out = dir.join("Seven.pdf");
    harness
        .state_mut()
        .engine
        .dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: options.clone() })
        .expect("the export starts");
    until(&mut harness, "the sheet to say the PDF is written", |h| shown_text(h).contains("Seven.pdf"));
    let text = shown_text(&harness);
    assert!(text.contains("Seven.pdf") && text.contains("7 pages"), "{text}");
    let pdf = std::fs::read(&out).expect("the PDF is on disk");
    assert!(pdf.starts_with(b"%PDF-"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn crop_a_page_delete_it_and_take_the_delete_back() {
    let dir = std::env::temp_dir().join(format!("scancraft-flow-crop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the folder");
    let mut harness = shell(&dir);
    harness.step();
    let paths: Vec<String> = (0..3).map(|seed| desk(&dir, seed)).collect();
    harness.state_mut().engine.dispatch(Command::ImportAdd { paths }).expect("the import starts");
    until(&mut harness, "the pages", |h| h.query_all_by_label("Page 3").next().is_some() && !h.state().engine.view().busy);

    // Crop opens on the page, with its edges and its controls, and Done finishes it in place.
    click(&mut harness, "Crop");
    until(&mut harness, "Crop", |h| h.query_all_by_label("Reset to Detected Edges").next().is_some());
    assert!(matches!(harness.state().engine.view().screen, Screen::Crop(_)));
    let before: Vec<_> = harness.state().engine.view().tray.iter().map(|t| t.page).collect();
    click(&mut harness, "Done");
    until(&mut harness, "the finished page", |h| h.query_all_by_label("Show Original").next().is_some());
    let after: Vec<_> = harness.state().engine.view().tray.iter().map(|t| t.page).collect();
    assert_eq!(before, after, "Done changed the pages");

    // Delete says so, with an Undo, and the Undo brings the page back.
    click(&mut harness, "Delete");
    until(&mut harness, "the page to go", |h| shown_text(h).contains("2 pages"));
    assert!(shown_text(&harness).contains("Page 1 deleted"), "{}", shown_text(&harness));
    click(&mut harness, "Undo");
    until(&mut harness, "the page to come back", |h| shown_text(h).contains("3 pages"));
    let _ = std::fs::remove_dir_all(&dir);
}
