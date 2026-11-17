//! The engine through its public face: commands in, view states and files out.

mod common;

use common::*;
use scancraft_core::command::{Command, Corner};
use scancraft_core::engine_api::{EngineApi, ImageKey, Screen};
use scancraft_core::geom::{Point, Side};
use scancraft_core::recipe::{CornerOrigin, ExportOptions, FlattenMode, Look, PageId, PhotoId, SideShape};
use scancraft_engine::Services;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn import_desk(engine: &mut scancraft_engine::Engine, dir: &TempDir, seed: u8) {
    let path = write(dir, &format!("desk{seed}.png"), &desk_png(seed));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(engine);
    open_crop_on_first_page(engine);
}

/// The view with the redo flag aside: undoing leaves a step to redo, which the earlier view didn't have.
fn without_redo(mut view: scancraft_core::engine_api::ViewState) -> scancraft_core::engine_api::ViewState {
    view.can_redo = false;
    view
}

fn engine_with_photo(dir: &TempDir) -> scancraft_engine::Engine {
    let mut engine = open(dir, services(desk_quad()));
    import_desk(&mut engine, dir, 0);
    engine
}

// ---- importing and detecting ----

#[test]
fn a_photo_joins_the_document_as_a_finished_page_when_it_has_been_read() {
    let dir = TempDir::new("joins");
    let mut engine = open(&dir, services(desk_quad()));
    let path = write(&dir, "desk0.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    // Nothing is in the document while the photo is being read, and the view says how far along it is.
    assert!(engine.view().tray.is_empty());
    assert_eq!(engine.view().importing, Some(scancraft_core::engine_api::Progress { done: 0, total: 1 }));
    settle(&mut engine);
    let view = engine.view();
    assert!(view.importing.is_none());
    // The page is in the document, with no Add Page: cut from the outline the detector found, in
    // the last look used, and the first page opens finished.
    assert_eq!(view.tray.len(), 1);
    assert_eq!(view.tray[0].state, scancraft_core::engine_api::PageState::Ready);
    let Screen::Enhance(page) = view.screen else { panic!("the finished page should be open: {:?}", view.screen) };
    assert_eq!(page, view.tray[0].page);
    let page = engine.session().page(page).expect("the page").clone();
    assert!(matches!(page.corners_from, CornerOrigin::Detected { .. }));
    assert_eq!(page.look, Look::ColorDocument);
    assert!(page.proportions.is_some() && page.page_size_mm.is_some(), "the geometry step ran");
    assert!(engine.session().pending().is_empty());
}

#[test]
fn opening_crop_on_a_page_shows_the_outline_a_detector_found() {
    let dir = TempDir::new("detect");
    let engine = engine_with_photo(&dir);
    let view = engine.view();
    let crop = view.crop.expect("Screen 2");
    assert!(matches!(view.screen, Screen::Crop(p) if p == crop.photo));
    assert_eq!(crop.position, (1, 1));
    assert_eq!(crop.photo_size, (160, 120));
    assert_eq!(crop.outlines.len(), 1);
    let outline = &crop.outlines[0];
    assert!(matches!(outline.corners_from, CornerOrigin::Detected { .. }), "{:?}", outline.corners_from);
    assert!(crop.auto_detect && !crop.not_found);
    assert!(outline.invalid.is_none());
    for (got, want) in outline.corners.0.iter().zip(desk_quad().0) {
        assert!(got.distance(want) < 20.0, "{got:?} vs {want:?}");
    }
    // The page being cropped is in the document the whole time.
    assert_eq!(view.tray.len(), 1);
    assert_eq!(view.tray[0].page, outline.page);
}

#[test]
fn a_photo_with_no_page_found_starts_with_the_corners_inside_its_edges() {
    let dir = TempDir::new("notfound");
    let mut engine = open(&dir, Services::default());
    import_desk(&mut engine, &dir, 0);
    let crop = engine.view().crop.expect("Screen 2");
    let outline = &crop.outlines[0];
    // Until page detection is finished the corners start 3% inside the photo; once it is, it finds the sheet.
    match outline.corners_from {
        CornerOrigin::NotFound => {
            assert!(crop.not_found);
            assert!(outline.corners.0[0].distance(Point::new(4.8, 3.6)) < 1e-9);
            assert!(outline.corners.0[2].distance(Point::new(160.0 - 4.8, 120.0 - 3.6)) < 1e-9);
        }
        CornerOrigin::Detected { .. } => assert!(!crop.not_found),
        other => panic!("unexpected origin {other:?}"),
    }
}

#[test]
fn unsupported_files_are_skipped_and_named() {
    let dir = TempDir::new("skip");
    let mut engine = open(&dir, services(desk_quad()));
    let junk = write(&dir, "notes.txt", b"just some words");
    let good = write(&dir, "scan.png", &desk_png(1));
    let empty = write(&dir, "empty.png", b"");
    engine.dispatch(Command::ImportAdd { paths: vec![junk.clone(), good, empty, dir.join("missing.png").to_string_lossy().into_owned()] }).unwrap();
    let notices = texts(&settle(&mut engine));
    assert!(notices.iter().any(|n| n.contains("notes.txt")), "{notices:?}");
    assert!(notices.iter().any(|n| n.contains("empty.png")), "{notices:?}");
    assert!(notices.iter().any(|n| n.contains("missing.png")), "{notices:?}");
    assert_eq!(engine.session().file().photos.len(), 1);
    // Nothing usable at all is an error the caller sees.
    assert!(engine.dispatch(Command::ImportAdd { paths: vec![junk] }).is_err());
    assert!(engine.dispatch(Command::ImportAdd { paths: vec![] }).is_err());
}

#[test]
fn a_picture_that_cannot_be_decoded_is_dropped_with_its_name() {
    let dir = TempDir::new("broken");
    let mut engine = open(&dir, services(desk_quad()));
    // A PNG's signature and nothing else passes the quick look and fails the decode.
    let broken = write(&dir, "broken.png", &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0]);
    engine.dispatch(Command::ImportAdd { paths: vec![broken] }).unwrap();
    let notices = settle(&mut engine);
    assert!(notices.iter().any(|n| n.error && n.text.contains("broken.png")), "{notices:?}");
    assert!(engine.session().file().photos.is_empty());
    assert_eq!(engine.view().screen, Screen::Import);
}

#[test]
fn the_same_file_twice_is_imported_once_and_a_folder_in_natural_order() {
    let dir = TempDir::new("folder");
    let mut engine = open(&dir, services(desk_quad()));
    let folder = dir.join("batch");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, seed) in [("p10.png", 3u8), ("p2.png", 4), ("p1.png", 5), (".hidden.png", 6)] {
        std::fs::write(folder.join(name), desk_png(seed)).unwrap();
    }
    let again = write(&dir, "again.png", &desk_png(3));
    engine.dispatch(Command::ImportAdd { paths: vec![folder.to_string_lossy().into_owned(), again] }).unwrap();
    settle(&mut engine);
    let names: Vec<String> = engine.session().file().photos.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, vec!["p1.png", "p2.png", "p10.png"]);
}

#[test]
fn a_photo_that_is_still_being_read_keeps_the_view_busy_until_it_is_ready() {
    let dir = TempDir::new("busy");
    let (gate, held) = std::sync::mpsc::channel();
    let services =
        Services { detector: Some(Arc::new(HeldDetector { quad: desk_quad(), gate: std::sync::Mutex::new(held) })), recognizer: None, upright: None };
    let mut engine = open(&dir, services);
    let path = write(&dir, "desk.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    assert!(engine.view().busy);
    assert_eq!(engine.view().screen, Screen::Import);
    gate.send(()).unwrap();
    settle(&mut engine);
    assert!(!engine.view().busy);
    // The page joined and opened, finished.
    assert!(matches!(engine.view().screen, Screen::Enhance(_)));
    assert_eq!(engine.view().tray.len(), 1);
}

// ---- the review-first contract (the engine answers it before it does it) ----

#[test]
fn a_tray_page_names_its_photo_and_is_ready_and_no_work_is_reported_when_idle() {
    let dir = TempDir::new("tray-contract");
    let mut engine = engine_with_photo(&dir);
    let (photo, page) = current(&engine);
    let view = engine.view();
    assert_eq!(view.tray.len(), 1);
    assert_eq!(view.tray[0].page, page);
    assert_eq!(view.tray[0].photo, photo);
    assert_eq!(view.tray[0].state, scancraft_core::engine_api::PageState::Ready);
    assert!(view.importing.is_none() && view.reading_text.is_none() && view.exporting.is_none() && view.last_export.is_none());
    // There is nothing to add: the page is already in the document.
    let error = engine.dispatch(Command::PageAdd { page }).unwrap_err();
    assert!(error.contains("already in the document"), "{error}");
}

// ---- the review-first flow ----

use scancraft_core::engine_api::{PageState, Progress, ReviewReason};

/// A photo with nothing on it that looks like a page, so detection finds none.
fn blank_png() -> Vec<u8> {
    png(160, 120, |_, _| [128, 128, 128])
}

#[test]
fn a_page_whose_edges_were_not_found_joins_flagged_and_is_cleared_by_looking_at_it() {
    let dir = TempDir::new("flag");
    let mut engine = open(&dir, Services::default());
    let path = write(&dir, "blank.png", &blank_png());
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    let page = engine.view().tray[0].page;
    assert_eq!(engine.view().tray[0].state, PageState::NeedsLook(ReviewReason::NotFound));
    // Marking it reviewed takes the flag off; it is not an undo step and changes no page.
    let before = engine.session().file().pages.clone();
    engine.dispatch(Command::PageMarkReviewed { page }).unwrap();
    assert_eq!(engine.view().tray[0].state, PageState::Ready);
    assert_eq!(engine.session().file().pages, before);
    assert!(engine.dispatch(Command::PageMarkReviewed { page: PageId(9999) }).is_err());
}

#[test]
fn moving_a_corner_or_pressing_done_takes_the_flag_off() {
    for how in ["corner", "done"] {
        let dir = TempDir::new("flag-clear");
        let mut engine = open(&dir, Services::default());
        let path = write(&dir, "blank.png", &blank_png());
        engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
        settle(&mut engine);
        open_crop_on_first_page(&mut engine);
        let (photo, page) = current(&engine);
        assert_eq!(engine.view().tray[0].state, PageState::NeedsLook(ReviewReason::NotFound));
        if how == "corner" {
            // The corner is the user's now, so the outline is no longer a guess.
            engine.dispatch(Command::CropSetCorner { page, corner: Corner::TopLeft, x: 10.0, y: 10.0 }).unwrap();
        } else {
            engine.dispatch(Command::CropApply { photo }).unwrap();
        }
        settle(&mut engine);
        assert_eq!(engine.view().tray[0].state, PageState::Ready, "after {how}");
    }
}

#[test]
fn a_page_with_an_invalid_outline_is_flagged() {
    let dir = TempDir::new("flag-invalid");
    let mut engine = engine_with_photo(&dir);
    let (_, page) = current(&engine);
    assert_eq!(engine.view().tray[0].state, PageState::Ready);
    engine.dispatch(Command::CropSetCorner { page, corner: Corner::TopLeft, x: -1.0e9, y: 1.0e9 }).unwrap();
    assert_eq!(engine.view().tray[0].state, PageState::NeedsLook(ReviewReason::InvalidOutline));
}

#[test]
fn a_page_says_it_is_preparing_while_it_is_being_flattened() {
    let dir = TempDir::new("preparing");
    let mut engine = engine_with_photo(&dir);
    let (photo, _) = current(&engine);
    settle(&mut engine);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    // The flatten decision runs in the background, and the page is not final until it is done.
    assert_eq!(engine.view().tray[0].state, PageState::Preparing);
    settle(&mut engine);
    assert_eq!(engine.view().tray[0].state, PageState::Ready);
}

#[test]
fn a_page_whose_picture_cannot_be_made_says_why() {
    let dir = TempDir::new("failed-state");
    let mut engine = engine_with_photo(&dir);
    settle(&mut engine);
    engine.flush();
    drop(engine);
    let sources = dir.path().join("Scan A").join("sources");
    for entry in std::fs::read_dir(&sources).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let mut engine = open(&dir, services(desk_quad()));
    let page = engine.view().tray[0].page;
    for _ in 0..100 {
        engine.poll();
        let _ = engine.image(ImageKey::TrayThumb(page));
        if matches!(engine.view().tray[0].state, PageState::Failed(_)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let PageState::Failed(why) = engine.view().tray[0].state.clone() else { panic!("{:?}", engine.view().tray[0].state) };
    assert!(!why.is_empty());
    // Changing the page's outline gives it another chance, so the failure is forgotten.
    open_crop_on_first_page(&mut engine);
    engine.dispatch(Command::CropFullImage { page }).unwrap();
    assert!(!matches!(engine.view().tray[0].state, PageState::Failed(_)));
}

#[test]
fn pages_join_the_document_in_photo_order_without_taking_the_screen() {
    let dir = TempDir::new("order");
    let slow = SlowFor { quad: desk_quad(), marker: 40, delay: std::time::Duration::from_millis(400) };
    let services = Services { detector: Some(Arc::new(slow)), recognizer: None, upright: None };
    let mut engine = open(&dir, services);
    // The first photo is slow to read; the second is quick.
    let first = write(&dir, "a.png", &desk_png(0));
    let second = write(&dir, "b.png", &desk_png(1));
    engine.dispatch(Command::ImportAdd { paths: vec![first, second] }).unwrap();
    assert_eq!(engine.view().importing, Some(Progress { done: 0, total: 2 }));
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while engine.view().tray.is_empty() && std::time::Instant::now() < end {
        engine.poll();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The second photo's page joined first, and it is the page that opened.
    let photos: Vec<_> = engine.session().file().photos.iter().map(|p| p.id).collect();
    assert_eq!(engine.view().tray.len(), 1);
    assert_eq!(engine.view().tray[0].photo, photos[1]);
    assert_eq!(engine.view().importing, Some(Progress { done: 1, total: 2 }));
    let opened = engine.view().tray[0].page;
    assert_eq!(engine.view().screen, Screen::Enhance(opened));
    settle(&mut engine);
    // The first photo's page then took its place before the second's, and the screen stayed put.
    let tray = engine.view().tray;
    assert_eq!(tray.iter().map(|t| t.photo).collect::<Vec<_>>(), photos);
    assert_eq!(engine.view().screen, Screen::Enhance(opened));
    assert!(engine.view().importing.is_none());
}

#[test]
fn cropping_a_page_in_the_document_keeps_its_place_its_look_and_its_strength() {
    // Re-cropping a page reset its strength, and a page in the document could not be cropped at all.
    let dir = TempDir::new("recrop");
    let mut engine = open(&dir, services(desk_quad()));
    for seed in 0..3 {
        import_desk(&mut engine, &dir, seed);
    }
    let ids: Vec<PageId> = engine.view().tray.iter().map(|t| t.page).collect();
    // The middle page, in its own look.
    engine.dispatch(Command::PageOpenCrop { page: ids[1] }).unwrap();
    let (photo, page) = current(&engine);
    assert_eq!(page, ids[1]);
    engine.dispatch(Command::PageSetLook { page, look: Look::BlackWhite }).unwrap();
    engine.dispatch(Command::PageSetStrength { page, value: 33 }).unwrap();
    settle(&mut engine);
    assert!(engine.view().tray[1].text_ready);
    engine.dispatch(Command::CropSetCorner { page, corner: Corner::BottomRight, x: 120.0, y: 95.0 }).unwrap();
    // The recognised text belonged to the old outline.
    assert!(!engine.view().tray[1].text_ready);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    let view = engine.view();
    assert_eq!(view.screen, Screen::Enhance(page));
    assert_eq!(view.tray.iter().map(|t| t.page).collect::<Vec<_>>(), ids, "the page kept its place");
    let kept = engine.session().page(page).unwrap().clone();
    assert_eq!((kept.look, kept.strength), (Look::BlackWhite, 33));
    assert_eq!(kept.corners.0[2], Point::new(120.0, 95.0));
    assert!(kept.proportions.is_some(), "the geometry was worked out again");
    // And the text is read again, for the new outline.
    settle(&mut engine);
    assert!(engine.view().tray[1].text_ready);
}

#[test]
fn the_window_is_never_empty_while_the_document_has_pages() {
    let dir = TempDir::new("never-empty");
    let mut engine = open(&dir, services(desk_quad()));
    for seed in 0..2 {
        import_desk(&mut engine, &dir, seed);
    }
    let photos: Vec<_> = engine.session().file().photos.iter().map(|p| p.id).collect();
    // Removing the photo that is open shows the other page.
    engine.dispatch(Command::ImportDiscard { item: photos[0] }).unwrap();
    assert!(matches!(engine.view().screen, Screen::Enhance(_)), "{:?}", engine.view().screen);
    assert_eq!(engine.view().tray.len(), 1);
    // Deleting the last page is the only way to the empty window.
    let last = engine.view().tray[0].page;
    engine.dispatch(Command::PageDelete { page: last }).unwrap();
    assert_eq!(engine.view().screen, Screen::Import);
    assert!(engine.session().file().photos.is_empty(), "the photo went with its last page");
}

#[test]
fn a_look_for_every_page_is_one_undo_step() {
    let dir = TempDir::new("look-all");
    let mut engine = open(&dir, services(desk_quad()));
    for seed in 0..3 {
        import_desk(&mut engine, &dir, seed);
    }
    let clock = Arc::new(AtomicU64::new(0));
    let c = clock.clone();
    engine.set_clock(move || c.load(Ordering::Relaxed));
    let before: Vec<_> = engine.session().file().pages.iter().map(|p| (p.look, p.strength)).collect();
    clock.fetch_add(10_000, Ordering::Relaxed);
    engine.dispatch(Command::PageSetLookAll { look: Look::Grayscale, strength: 41 }).unwrap();
    let all = |e: &scancraft_engine::Engine| e.session().file().pages.iter().map(|p| (p.look, p.strength)).collect::<Vec<_>>();
    assert_eq!(all(&engine), vec![(Look::Grayscale, 41); 3]);
    // A look with no strength has none, whatever was asked, and is the next page's starting look too.
    clock.fetch_add(10_000, Ordering::Relaxed);
    engine.dispatch(Command::PageSetLookAll { look: Look::Original, strength: 90 }).unwrap();
    assert_eq!(all(&engine), vec![(Look::Original, 0); 3]);
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(all(&engine), vec![(Look::Grayscale, 41); 3], "one undo takes back every page");
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(all(&engine), before);
    // Nothing to change in an empty document.
    let empty = TempDir::new("look-all-empty");
    let mut none = open(&empty, services(desk_quad()));
    assert!(none.dispatch(Command::PageSetLookAll { look: Look::Grayscale, strength: 50 }).is_err());
}

#[test]
fn cancelling_an_export_writes_no_file_and_says_so_and_the_next_export_works() {
    let dir = TempDir::new("cancel");
    let (gate, held) = std::sync::mpsc::channel();
    // One worker thread, held by the text reading, so the export has to wait behind it.
    let services = Services {
        detector: Some(Arc::new(FixedDetector(desk_quad()))),
        recognizer: Some(Arc::new(GatedWord { gate: std::sync::Mutex::new(held) })),
        upright: None,
    };
    let mut engine = open_with(dir.path(), "Scan A", services, 1);
    let path = write(&dir, "desk.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while engine.view().reading_text.is_none() && std::time::Instant::now() < end {
        engine.poll();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(engine.view().reading_text.is_some(), "the text is being read");
    assert!(engine.dispatch(Command::ExportCancel).is_err(), "nothing to cancel yet");
    let out = dir.join("never.pdf");
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    assert_eq!(engine.view().exporting, Some(Progress { done: 0, total: 1 }));
    engine.dispatch(Command::ExportCancel).unwrap();
    gate.send(()).unwrap();
    let notices = texts(&settle(&mut engine));
    assert!(notices.iter().any(|n| n.contains("cancelled")), "{notices:?}");
    assert!(!notices.iter().any(|n| n.contains("pages changed")), "{notices:?}");
    assert!(!out.exists(), "a cancelled export left a file");
    assert!(engine.view().exporting.is_none() && engine.view().last_export.is_none());
    assert!(!engine.is_exporting());
    // The next export goes through.
    let again = dir.join("again.pdf");
    engine.dispatch(Command::ExportPdf { path: again.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    settle(&mut engine);
    assert!(again.is_file());
    assert_eq!(engine.view().last_export.map(|s| s.pages), Some(1));
}

#[test]
fn the_original_preview_is_the_page_without_its_filter() {
    let dir = TempDir::new("original");
    let (bytes, quad) = flat_fixture();
    let mut engine = open(&dir, services(quad));
    let path = write(&dir, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    let page = engine.view().tray[0].page;
    engine.dispatch(Command::PageSetLook { page, look: Look::BlackWhite }).unwrap();
    let (filtered, _) = wait_for_image(&mut engine, ImageKey::LookPreview(page));
    let (original, notices) = wait_for_image(&mut engine, ImageKey::OriginalPreview(page));
    let (filtered, original) = (filtered.expect("the filtered page"), original.expect("the original page"));
    assert!(notices.is_empty(), "{notices:?}");
    assert!(original.width().max(original.height()) <= 1280);
    assert_ne!(original.data(), filtered.data(), "Black & White changed nothing");
    // With no filter on the page, the two are the same picture.
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    let (plain, _) = wait_for_image(&mut engine, ImageKey::LookPreview(page));
    assert_eq!(plain.expect("the page").data(), original.data());
    assert!(engine.image(ImageKey::OriginalPreview(PageId(555))).is_none());
}

#[test]
fn the_engine_is_busy_until_a_finished_job_has_been_applied() {
    // A job's result is applied by `poll`, and applying it queues what comes next: the page joins
    // the document, is turned upright, flattened and read. `busy` went false as soon as the last
    // worker finished, before `poll` had applied that result, so anyone who waited for "not busy"
    // could stop with the pipeline half done: a PDF with a page unfinished, on a slow machine.
    let dir = TempDir::new("busy-until-applied");
    let (bytes, quad) = flat_fixture();
    let mut engine = open(&dir, services(quad));
    let path = write(&dir, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    // Time for the workers to finish, and no `poll`: nothing they made has been applied.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(engine.view().busy, "the engine says it is idle while a result is waiting to be applied");
    settle(&mut engine);
    assert!(!engine.view().busy);
    assert_eq!(engine.view().tray.len(), 1, "the page did not join the document");
}

#[test]
fn the_automatic_turn_is_kept_when_the_flattening_finishes_before_it() {
    // The turn is decided from the page as it was when the page joined, and the flattening that is
    // decided at the same time changes the page's sheet. The turn was dropped as stale whenever the
    // flattening came back first, so whether a page was turned depended on which of the two the
    // workers finished first: one thread and eight made different pages. Here the turn is held back
    // until the flattening has landed, which is the order that lost it.
    let dir = TempDir::new("turn-after-flatten");
    let (bytes, quad) = flat_fixture();
    let (release, gate) = std::sync::mpsc::channel();
    let mut services = services(quad);
    services.upright = Some(Arc::new(GatedHalfTurn { gate: std::sync::Mutex::new(gate) }));
    let mut engine = open_with(dir.path(), "Scan A", services, 2);
    let path = write(&dir, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while engine.session().file().pages.first().is_none_or(|p| p.sheet.is_none()) {
        assert!(std::time::Instant::now() < end, "the flattening never came back");
        engine.poll();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(engine.session().file().pages[0].quarter_turns, 0, "the turn came back before it was released");
    release.send(()).unwrap();
    settle(&mut engine);
    assert_eq!(engine.session().file().pages[0].quarter_turns, 2, "the turn was dropped because the flattening came back first");
}

#[test]
fn the_original_preview_does_not_depend_on_the_number_of_threads() {
    let (bytes, quad) = flat_fixture();
    let mut pictures = Vec::new();
    for threads in [1, 8] {
        let dir = TempDir::new("original-threads");
        let mut engine = open_with(dir.path(), "Scan A", services(quad), threads);
        let path = write(&dir, "flat.png", &bytes);
        engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
        settle(&mut engine);
        let page = engine.view().tray[0].page;
        engine.dispatch(Command::PageSetLook { page, look: Look::Grayscale }).unwrap();
        let (original, _) = wait_for_image(&mut engine, ImageKey::OriginalPreview(page));
        pictures.push(original.expect("the original page").data().to_vec());
    }
    assert!(pictures[0] == pictures[1], "one thread and eight threads made different pictures");
}

// ---- several pages at once ----

fn engine_with_four_pages(dir: &TempDir) -> (scancraft_engine::Engine, Vec<PageId>) {
    let mut engine = open(dir, services(desk_quad()));
    for seed in 0..4 {
        import_desk(&mut engine, dir, seed);
    }
    let ids = engine.view().tray.iter().map(|t| t.page).collect();
    (engine, ids)
}

#[test]
fn a_look_for_the_selected_pages_changes_those_pages_in_one_undo_step() {
    let dir = TempDir::new("look-many");
    let (mut engine, ids) = engine_with_four_pages(&dir);
    let looks = |e: &scancraft_engine::Engine| e.session().file().pages.iter().map(|p| (p.look, p.strength)).collect::<Vec<_>>();
    let before = looks(&engine);
    // Named out of order, one twice: the pages are changed once each.
    engine.dispatch(Command::PageSetLookMany { pages: vec![ids[2], ids[0], ids[2]], look: Look::Grayscale, strength: 33 }).unwrap();
    let after = looks(&engine);
    assert_eq!(after[0], (Look::Grayscale, 33));
    assert_eq!(after[2], (Look::Grayscale, 33));
    assert_eq!((after[1], after[3]), (before[1], before[3]), "a page that was not selected changed");
    // A look with no strength has none, whatever was asked.
    engine.dispatch(Command::PageSetLookMany { pages: vec![ids[1]], look: Look::Original, strength: 80 }).unwrap();
    assert_eq!(looks(&engine)[1], (Look::Original, 0));
    engine.dispatch(Command::EditUndo).unwrap();
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(looks(&engine), before, "two undos take both batches back");
}

#[test]
fn dragging_the_strength_of_the_selected_pages_is_one_undo_step() {
    let dir = TempDir::new("strength-many");
    let (mut engine, ids) = engine_with_four_pages(&dir);
    let clock = Arc::new(AtomicU64::new(10_000));
    let c = clock.clone();
    engine.set_clock(move || c.load(Ordering::Relaxed));
    let strengths = |e: &scancraft_engine::Engine| e.session().file().pages.iter().map(|p| p.strength).collect::<Vec<_>>();
    let before = strengths(&engine);
    // Sixty events of one drag on the same two pages, 10 ms apart.
    for value in 0..60u8 {
        clock.fetch_add(10, Ordering::Relaxed);
        engine.dispatch(Command::PageSetLookMany { pages: vec![ids[0], ids[2]], look: Look::Grayscale, strength: value }).unwrap();
    }
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(strengths(&engine), before, "one undo did not take the whole drag back");
}

#[test]
fn the_selected_pages_are_turned_together_and_one_undo_turns_them_back() {
    let dir = TempDir::new("rotate-many");
    let (mut engine, ids) = engine_with_four_pages(&dir);
    settle(&mut engine);
    let turns = |e: &scancraft_engine::Engine| e.session().file().pages.iter().map(|p| p.quarter_turns).collect::<Vec<_>>();
    let before = turns(&engine);
    engine.dispatch(Command::PageRotateMany { pages: vec![ids[1], ids[3]], quarter_turns: 1 }).unwrap();
    let after = turns(&engine);
    assert_eq!(after[1], (before[1] + 1) % 4);
    assert_eq!(after[3], (before[3] + 1) % 4);
    assert_eq!((after[0], after[2]), (before[0], before[2]));
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(turns(&engine), before);
    settle(&mut engine);
}

#[test]
fn the_selected_pages_are_deleted_together_and_the_window_is_not_left_empty() {
    let dir = TempDir::new("delete-many");
    let (mut engine, ids) = engine_with_four_pages(&dir);
    let before = engine.view();
    engine.dispatch(Command::PageDeleteMany { pages: vec![ids[0], ids[2]] }).unwrap();
    let view = engine.view();
    assert_eq!(view.tray.iter().map(|t| t.page).collect::<Vec<_>>(), vec![ids[1], ids[3]]);
    // The photos of the deleted pages went with them, and what is open is a page that is left.
    assert_eq!(engine.session().file().photos.len(), 2);
    assert!(matches!(view.screen, Screen::Enhance(p) if view.tray.iter().any(|t| t.page == p)), "{:?}", view.screen);
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(engine.view().tray, before.tray, "one undo brings every page back");
    // All of them is allowed, and is the one way to the empty window.
    engine.dispatch(Command::PageDeleteMany { pages: ids.clone() }).unwrap();
    assert!(engine.view().tray.is_empty());
    assert_eq!(engine.view().screen, Screen::Import);
}

#[test]
fn a_batch_that_names_a_page_that_is_not_there_changes_nothing() {
    let dir = TempDir::new("batch-refused");
    let (mut engine, ids) = engine_with_four_pages(&dir);
    let before = engine.view();
    let pages = engine.session().file().pages.clone();
    for command in [
        Command::PageRotateMany { pages: vec![ids[0], PageId(9999)], quarter_turns: 1 },
        Command::PageDeleteMany { pages: vec![PageId(9999), ids[1]] },
        Command::PageSetLookMany { pages: vec![ids[0], PageId(9999)], look: Look::BlackWhite, strength: 50 },
        Command::PageRotateMany { pages: vec![], quarter_turns: 1 },
        Command::PageDeleteMany { pages: vec![] },
    ] {
        assert!(engine.dispatch(command.clone()).is_err(), "{command:?} was accepted");
    }
    assert_eq!(engine.session().file().pages, pages, "a refused batch changed a page");
    assert_eq!(engine.view().tray, before.tray);
}

// ---- Screen 2 ----

#[test]
fn a_corner_is_clamped_marked_manual_and_undone_exactly() {
    let dir = TempDir::new("corner");
    let mut engine = engine_with_photo(&dir);
    let (_, page) = current(&engine);
    let before = engine.view();
    engine.dispatch(Command::CropSetCorner { page, corner: Corner::TopLeft, x: -1.0e9, y: 1.0e9 }).unwrap();
    let crop = engine.view().crop.unwrap();
    let corner = crop.outlines[0].corners.0[0];
    // At most 15% outside the photo.
    assert_eq!(corner, Point::new(-24.0, 138.0));
    assert!(matches!(crop.outlines[0].corners_from, CornerOrigin::Manual));
    assert!(!crop.auto_detect);
    assert!(crop.outlines[0].invalid.is_some(), "a corner dragged below the bottom edge makes the outline invalid");
    assert!(engine.view().can_undo && !engine.view().can_redo);
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(without_redo(engine.view()), before);
    engine.dispatch(Command::EditRedo).unwrap();
    assert_eq!(engine.view().crop.unwrap().outlines[0].corners.0[0], corner);
    assert!(engine.dispatch(Command::CropSetCorner { page, corner: Corner::TopLeft, x: f64::NAN, y: 0.0 }).is_err());
    assert!(engine.dispatch(Command::CropSetCorner { page: PageId(9999), corner: Corner::TopLeft, x: 0.0, y: 0.0 }).is_err());
}

#[test]
fn moving_a_side_slides_its_corners_along_the_neighbours() {
    let dir = TempDir::new("side");
    let mut engine = engine_with_photo(&dir);
    let (_, page) = current(&engine);
    let before = engine.view().crop.unwrap().outlines[0].corners;
    engine.dispatch(Command::CropMoveSide { page, side: Side::Right, offset: 10.0 }).unwrap();
    let after = engine.view().crop.unwrap().outlines[0].corners;
    // The right side moved right by about 10 px, the left side stayed.
    assert!((after.0[1].x - before.0[1].x - 10.0).abs() < 0.5, "{after:?}");
    assert!((after.0[2].x - before.0[2].x - 10.0).abs() < 0.5);
    assert_eq!(after.0[0], before.0[0]);
    assert!(engine.dispatch(Command::CropMoveSide { page, side: Side::Left, offset: f64::INFINITY }).is_err());
}

#[test]
fn bending_a_side_turns_flatten_on_and_straightening_takes_the_bend_away() {
    let dir = TempDir::new("bend");
    let mut engine = engine_with_photo(&dir);
    let (_, page) = current(&engine);
    let handles = [Point::new(50.0, 10.0), Point::new(80.0, 8.0), Point::new(110.0, 10.0)];
    engine.dispatch(Command::CropBendSide { page, side: Side::Top, handles }).unwrap();
    let outline = engine.view().crop.unwrap().outlines[0].clone();
    assert_eq!(outline.flatten, FlattenMode::On);
    assert!(matches!(outline.sides[0], SideShape::Bent { .. }));
    assert!(matches!(outline.corners_from, CornerOrigin::Manual));
    engine.dispatch(Command::CropStraightenSide { page, side: Side::Top }).unwrap();
    assert!(matches!(engine.view().crop.unwrap().outlines[0].sides[0], SideShape::Straight));
    let junk = [Point::new(f64::NAN, 0.0), handles[1], handles[2]];
    assert!(engine.dispatch(Command::CropBendSide { page, side: Side::Top, handles: junk }).is_err());
}

#[test]
fn full_image_and_auto_detect_swap_the_outline_back_and_forth() {
    let dir = TempDir::new("auto");
    let mut engine = engine_with_photo(&dir);
    let (_, page) = current(&engine);
    engine.dispatch(Command::CropFullImage { page }).unwrap();
    let outline = engine.view().crop.unwrap().outlines[0].clone();
    assert!(matches!(outline.corners_from, CornerOrigin::FullImage));
    assert_eq!(outline.corners.0[2], Point::new(160.0, 120.0));
    assert_eq!(outline.flatten, FlattenMode::Off);
    // Flattening isn't offered for a page cut with Use Full Image.
    assert!(engine.dispatch(Command::CropSetFlatten { page, mode: FlattenMode::On }).is_err());
    engine.dispatch(Command::CropSetFlatten { page, mode: FlattenMode::Off }).unwrap();
    engine.dispatch(Command::CropAuto { page }).unwrap();
    let crop = engine.view().crop.unwrap();
    assert!(matches!(crop.outlines[0].corners_from, CornerOrigin::Detected { .. }));
    assert!(crop.auto_detect);
}

#[test]
fn a_drag_is_one_undo_step_and_a_pause_starts_a_new_one() {
    let dir = TempDir::new("drag");
    let mut engine = engine_with_photo(&dir);
    let clock = Arc::new(AtomicU64::new(1_000));
    let c = clock.clone();
    engine.set_clock(move || c.load(Ordering::Relaxed));
    let (_, page) = current(&engine);
    let start = engine.view();
    // 60 events of one drag, 10 ms apart.
    for i in 0..60 {
        clock.fetch_add(10, Ordering::Relaxed);
        engine.dispatch(Command::CropSetCorner { page, corner: Corner::BottomRight, x: 100.0 + f64::from(i), y: 90.0 }).unwrap();
    }
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(without_redo(engine.view()), start);
    // Opening Crop and the import are steps of their own: undoing them takes the photo away, and
    // redoing brings it back (it is read again, and its page joins again).
    assert!(engine.view().can_undo);
    engine.dispatch(Command::EditUndo).unwrap();
    assert!(matches!(engine.view().screen, Screen::Enhance(_)));
    engine.dispatch(Command::EditUndo).unwrap();
    assert!(engine.session().file().photos.is_empty());
    assert_eq!(engine.view().screen, Screen::Import);
    engine.dispatch(Command::EditRedo).unwrap();
    assert_eq!(engine.session().file().photos.len(), 1);
    settle(&mut engine);
    engine.dispatch(Command::EditRedo).unwrap();
    engine.dispatch(Command::EditRedo).unwrap();
    assert_eq!(engine.view().crop.unwrap().outlines[0].corners.0[2], Point::new(159.0, 90.0));
    // A pause longer than the window starts a new step.
    clock.fetch_add(2_000, Ordering::Relaxed);
    engine.dispatch(Command::CropSetCorner { page, corner: Corner::BottomRight, x: 150.0, y: 90.0 }).unwrap();
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(engine.view().crop.unwrap().outlines[0].corners.0[2], Point::new(159.0, 90.0));
}

#[test]
fn undo_holds_two_hundred_steps_and_a_new_command_clears_redo() {
    let dir = TempDir::new("undo200");
    let mut engine = engine_with_photo(&dir);
    let clock = Arc::new(AtomicU64::new(0));
    let c = clock.clone();
    engine.set_clock(move || c.load(Ordering::Relaxed));
    let (_, page) = current(&engine);
    for i in 0..250 {
        clock.fetch_add(10_000, Ordering::Relaxed);
        engine.dispatch(Command::CropSetCorner { page, corner: Corner::TopLeft, x: 10.0 + f64::from(i % 50), y: 10.0 }).unwrap();
    }
    let mut undone = 0;
    while engine.view().can_undo {
        engine.dispatch(Command::EditUndo).unwrap();
        undone += 1;
    }
    assert_eq!(undone, 200);
    assert!(engine.dispatch(Command::EditUndo).is_err());
    assert!(engine.view().can_redo);
    engine.dispatch(Command::PageRotate { page, quarter_turns: 1 }).unwrap();
    assert!(!engine.view().can_redo);
    assert!(engine.dispatch(Command::EditRedo).is_err());
}

#[test]
fn discarding_a_photo_removes_its_page_and_undo_brings_it_back() {
    let dir = TempDir::new("discard");
    let mut engine = engine_with_photo(&dir);
    let (photo, _) = current(&engine);
    let before = engine.view();
    engine.dispatch(Command::ImportDiscard { item: photo }).unwrap();
    assert!(engine.session().file().photos.is_empty());
    assert_eq!(engine.view().screen, Screen::Import);
    assert!(engine.dispatch(Command::ImportDiscard { item: photo }).is_err());
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(without_redo(engine.view()), before);
}

// ---- spreads ----

#[test]
fn book_spreads_cut_a_photo_into_two_pages_that_open_in_reading_order() {
    let dir = TempDir::new("book");
    let mut engine = engine_with_photo(&dir);
    let (photo, _) = current(&engine);
    engine.dispatch(Command::CropBook { on: true }).unwrap();
    let crop = engine.view().crop.unwrap();
    assert!(engine.view().book_spreads);
    assert_eq!(crop.outlines.len(), 2);
    assert_eq!(crop.outlines[0].half, Some(scancraft_core::recipe::Half::Left));
    assert!(matches!(crop.outlines[0].sides[1], SideShape::Spine) && matches!(crop.outlines[1].sides[3], SideShape::Spine));
    let spine = crop.spine.unwrap();
    // The spine is the line between the middles of the top and bottom sides.
    assert!((spine[0].x - 80.0).abs() < 1.0 && (spine[1].x - 80.0).abs() < 1.0);
    // Moving the spine moves both pages' outlines.
    engine.dispatch(Command::CropSetSpine { photo, top: Point::new(90.0, 16.0), bottom: Point::new(95.0, 104.0) }).unwrap();
    let crop = engine.view().crop.unwrap();
    assert_eq!(crop.outlines[0].corners.0[1], Point::new(90.0, 16.0));
    assert_eq!(crop.outlines[1].corners.0[3], Point::new(95.0, 104.0));
    // Both halves are pages of the document the whole time, left then right, and Done opens the
    // left one finished.
    assert_eq!(engine.view().tray.len(), 2);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    let Screen::Enhance(left) = engine.view().screen else { panic!("Screen 3 expected") };
    assert_eq!(engine.view().enhance.unwrap().half, Some(scancraft_core::recipe::Half::Left));
    // Each page has a look of its own.
    engine.dispatch(Command::PageSetLook { page: left, look: Look::Grayscale }).unwrap();
    let view = engine.view();
    assert_eq!(view.tray.len(), 2);
    assert_eq!(view.tray[0].page, left);
    assert!(view.tray.iter().all(|item| item.photo == photo));
    let right = engine.session().file().pages[1].clone();
    assert_eq!(right.spread.as_ref().map(|s| s.half), Some(scancraft_core::recipe::Half::Right));
    assert_eq!(right.look, Look::ColorDocument);
    assert_eq!(engine.session().page(left).unwrap().look, Look::Grayscale);
    settle(&mut engine);
}

#[test]
fn a_right_to_left_book_opens_with_its_right_page() {
    let dir = TempDir::new("rtl");
    let mut engine = engine_with_photo(&dir);
    engine.set_settings(|s| s.book_page_order = scancraft_core::recipe::PageOrder::RightToLeft);
    let (photo, _) = current(&engine);
    engine.dispatch(Command::CropSplit { photo, on: true }).unwrap();
    engine.dispatch(Command::CropApply { photo }).unwrap();
    assert_eq!(engine.view().enhance.unwrap().half, Some(scancraft_core::recipe::Half::Right));
    // Joining again gives back the whole page, with the outer sides as they were.
    engine.dispatch(Command::EditUndo).unwrap();
    engine.dispatch(Command::CropSplit { photo, on: false }).unwrap();
    let crop = engine.view().crop.unwrap();
    assert_eq!(crop.outlines.len(), 1);
    assert!(crop.outlines[0].half.is_none());
    assert!(crop.outlines[0].sides.iter().all(|s| !matches!(s, SideShape::Spine)));
    assert!(engine.dispatch(Command::CropSetSpine { photo, top: Point::new(1.0, 1.0), bottom: Point::new(2.0, 2.0) }).is_err());
}

// ---- Screen 3 and the tray ----

fn to_screen_three(engine: &mut scancraft_engine::Engine) -> PageId {
    let (photo, _) = current(engine);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    let Screen::Enhance(page) = engine.view().screen else { panic!("Screen 3 expected") };
    page
}

/// Asks for an image until the engine has it, polling as the app does between frames; the notices
/// the engine showed on the way come back with it.
fn wait_for_image(engine: &mut scancraft_engine::Engine, key: ImageKey) -> (Option<Arc<scancraft_core::image::Rgb8>>, Vec<String>) {
    let mut notices = Vec::new();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < end {
        engine.poll();
        notices.extend(texts(&engine.view().notices));
        if let Some(image) = engine.image(key) {
            return (Some(image), notices);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    (None, notices)
}

#[test]
fn every_look_has_a_filter_thumbnail_and_a_tray_thumbnail_without_an_error() {
    // A thumbnail of a whole page is under 50 DPI. A18's ink rule refuses that density, so the
    // Black & White thumbnail failed, and the interface, which asks for it on every frame, got an
    // error notice on every frame.
    let dir = TempDir::new("thumbs");
    let mut engine = engine_with_photo(&dir);
    let page = to_screen_three(&mut engine);
    for look in [Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument] {
        let (image, notices) = wait_for_image(&mut engine, ImageKey::FilterThumb(page, look));
        assert!(image.is_some(), "no thumbnail for {look:?}; notices: {notices:?}");
        assert!(notices.is_empty(), "the {look:?} thumbnail raised notices: {notices:?}");
    }
    engine.dispatch(Command::PageSetLook { page, look: Look::BlackWhite }).unwrap();
    let (image, notices) = wait_for_image(&mut engine, ImageKey::TrayThumb(page));
    assert!(image.is_some() && notices.is_empty(), "the Black & White tray thumbnail: {notices:?}");
    let (image, notices) = wait_for_image(&mut engine, ImageKey::LookPreview(page));
    assert!(image.is_some() && notices.is_empty(), "the Black & White preview: {notices:?}");
}

#[test]
fn an_image_that_cannot_be_made_is_reported_once_not_on_every_request() {
    let dir = TempDir::new("failing-image");
    let mut engine = engine_with_photo(&dir);
    let _ = to_screen_three(&mut engine);
    settle(&mut engine);
    engine.flush();
    drop(engine);
    // A new run of the app opens the same session, with an empty memory, and the photo's file gone,
    // so the page's thumbnail cannot be made.
    let sources = dir.path().join("Scan A").join("sources");
    for entry in std::fs::read_dir(&sources).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let mut engine = open(&dir, services(desk_quad()));
    let page = engine.view().tray.first().map(|t| t.page).expect("the page is in the tray");
    // The interface asks again on every frame.
    let mut notices = Vec::new();
    for _ in 0..300 {
        engine.poll();
        notices.extend(texts(&engine.view().notices));
        assert!(engine.image(ImageKey::TrayThumb(page)).is_none());
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(notices.len(), 1, "the same failure was reported {} times: {:?}", notices.len(), notices.first());
}

#[test]
fn a_file_is_imported_again_once_it_has_been_read() {
    let dir = TempDir::new("reimport");
    let mut engine = open(&dir, services(desk_quad()));
    let path = write(&dir, "desk0.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path.clone()] }).unwrap();
    // While the photo is being read, importing it again makes no second photo.
    let error = engine.dispatch(Command::ImportAdd { paths: vec![path.clone()] }).unwrap_err();
    assert!(error.contains("already being read"), "{error}");
    settle(&mut engine);
    assert_eq!(engine.session().file().photos.len(), 1);
    // Once it has been read, the same file can come in again: the user may want another scan of it.
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    assert_eq!(engine.session().file().photos.len(), 2);
    assert_eq!(engine.view().tray.len(), 2);
    // The new page joined without taking the screen from the page that was open.
    assert!(matches!(engine.view().screen, Screen::Enhance(_)));
}

#[test]
fn looks_strengths_and_turns_follow_section_four_point_four() {
    let dir = TempDir::new("looks");
    let mut engine = engine_with_photo(&dir);
    let page = to_screen_three(&mut engine);
    for (look, strength) in [(Look::BlackWhite, 55), (Look::Grayscale, 60), (Look::ColorDocument, 60), (Look::Original, 0)] {
        engine.dispatch(Command::PageSetLook { page, look }).unwrap();
        let e = engine.view().enhance.unwrap();
        assert_eq!((e.look, e.strength), (look, strength));
    }
    engine.dispatch(Command::PageSetStrength { page, value: 250 }).unwrap();
    assert_eq!(engine.view().enhance.unwrap().strength, 100);
    engine.dispatch(Command::PageRotate { page, quarter_turns: -1 }).unwrap();
    assert_eq!(engine.view().enhance.unwrap().quarter_turns, 3);
    engine.dispatch(Command::PageRotate { page, quarter_turns: 127 }).unwrap();
    assert_eq!(engine.view().enhance.unwrap().quarter_turns, (3 + 127) % 4);
    assert!(!engine.view().enhance.unwrap().turned_automatically);
    assert!(engine.dispatch(Command::PageSetLook { page: PageId(77), look: Look::Original }).is_err());
    settle(&mut engine);
}

#[test]
fn the_tray_adds_orders_edits_and_deletes_pages() {
    let dir = TempDir::new("tray");
    let mut engine = open(&dir, services(desk_quad()));
    for seed in 0..3 {
        import_desk(&mut engine, &dir, seed);
    }
    // The three pages joined the document by themselves, in the order of their photos.
    let tray = |e: &scancraft_engine::Engine| e.view().tray.iter().map(|t| t.page).collect::<Vec<_>>();
    let ids = tray(&engine);
    assert_eq!(ids.len(), 3);
    let photos: Vec<_> = engine.session().file().photos.iter().map(|p| p.id).collect();
    assert_eq!(engine.view().tray.iter().map(|t| t.photo).collect::<Vec<_>>(), photos);
    assert_eq!(engine.view().tray.iter().map(|t| t.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(engine.dispatch(Command::PageAdd { page: ids[0] }).is_err(), "there is nothing to add");
    engine.dispatch(Command::PageMove { from: 2, to: 0 }).unwrap();
    assert_eq!(tray(&engine), vec![ids[2], ids[0], ids[1]]);
    assert!(engine.dispatch(Command::PageMove { from: 3, to: 0 }).is_err());
    assert!(engine.dispatch(Command::PageMove { from: 0, to: u32::MAX }).is_err());
    engine.dispatch(Command::PageEdit { page: ids[1] }).unwrap();
    assert_eq!(engine.view().screen, Screen::Enhance(ids[1]));
    engine.dispatch(Command::PageDelete { page: ids[1] }).unwrap();
    // The document still has pages, so the window is never empty: the page that was last shows.
    assert_eq!(engine.view().screen, Screen::Enhance(ids[0]));
    assert_eq!(tray(&engine), vec![ids[2], ids[0]]);
    // A photo with no page left goes with its page.
    assert_eq!(engine.session().file().photos.len(), 2);
    engine.dispatch(Command::EditUndo).unwrap();
    assert_eq!(tray(&engine), vec![ids[2], ids[0], ids[1]]);
    assert!(engine.dispatch(Command::PageDelete { page: PageId(4242) }).is_err());
    settle(&mut engine);
}

#[test]
fn recognised_text_reaches_the_tray_and_a_stale_result_is_dropped() {
    let dir = TempDir::new("text");
    let mut engine = engine_with_photo(&dir);
    let page = to_screen_three(&mut engine);
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    settle(&mut engine);
    assert!(engine.view().tray[0].text_ready);
    let text = engine.session().file().pages[0].text.clone().unwrap();
    assert_eq!(text.engine, "fake");
    // Turning the page throws the text away, because its positions belonged to the old turn, and
    // reads it again for the new one.
    engine.dispatch(Command::PageRotate { page, quarter_turns: 1 }).unwrap();
    assert!(!engine.view().tray[0].text_ready);
    assert!(engine.view().reading_text.is_some(), "the view says the text is being read");
    settle(&mut engine);
    assert!(engine.view().tray[0].text_ready);
    assert!(engine.view().reading_text.is_none());
}

// ---- images ----

#[test]
fn images_are_made_in_the_background_and_come_back_from_the_cache() {
    let dir = TempDir::new("images");
    let mut engine = engine_with_photo(&dir);
    let (photo, _) = current(&engine);
    // The detection job decoded the photo, but the display copy is a job of its own.
    let mut copy = engine.image(ImageKey::DisplayCopy(photo));
    settle(&mut engine);
    if copy.is_none() {
        copy = engine.image(ImageKey::DisplayCopy(photo));
    }
    assert_eq!(copy.map(|i| (i.width(), i.height())), Some((160, 120)));
    let page = to_screen_three(&mut engine);
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    // Asking for an image starts its job; the answer comes once the job has run.
    let first_ask = engine.image(ImageKey::LookPreview(page));
    settle(&mut engine);
    let preview = first_ask.or_else(|| engine.image(ImageKey::LookPreview(page))).expect("the preview is ready after the job ran");
    assert!(preview.width().max(preview.height()) <= 1280);
    let thumb = engine.image(ImageKey::TrayThumb(page));
    settle(&mut engine);
    let thumb = thumb.or_else(|| engine.image(ImageKey::TrayThumb(page))).expect("a thumbnail");
    assert!(thumb.width().max(thumb.height()) <= 256);
    // Other pages and photos have no images.
    assert!(engine.image(ImageKey::LookPreview(PageId(555))).is_none());
    assert!(engine.image(ImageKey::DisplayCopy(PhotoId(555))).is_none());
    // The same preview from the file cache after the memory copy is gone: a second engine reads the session's cache.
    drop(engine);
    let mut again = open(&dir, services(desk_quad()));
    let from_disk = again.image(ImageKey::LookPreview(page));
    assert!(from_disk.is_some(), "the preview was kept in the session's cache folder");
}

#[test]
fn the_loupe_cuts_a_square_and_cuts_again_only_when_the_node_moves_a_quarter() {
    let dir = TempDir::new("loupe");
    let mut engine = engine_with_photo(&dir);
    let (photo, _) = current(&engine);
    let first = engine.loupe(photo, Point::new(80.0, 60.0), 40.0).expect("the photo is decoded");
    // 1.5 times the area: the side grows by the square root of 1.5.
    let wanted = 40.0 * 1.5f64.sqrt();
    assert_eq!(first.image.width(), wanted.round() as u32);
    assert_eq!(first.image.height(), first.image.width());
    assert!((first.origin.x - (80.0 - wanted / 2.0)).abs() < 1e-9);
    assert!((first.scale * f64::from(first.image.width()) - wanted).abs() < 1e-9);
    // A small move keeps the crop; a move of more than a quarter of its side cuts a new one.
    let near = engine.loupe(photo, Point::new(80.0 + wanted * 0.2, 60.0), 40.0).unwrap();
    assert_eq!(near.origin, first.origin);
    let far = engine.loupe(photo, Point::new(80.0 + wanted * 0.3, 60.0), 40.0).unwrap();
    assert_ne!(far.origin, first.origin);
    // A big area is capped at 1024 px a side and the crop samples the photo more coarsely.
    let big = engine.loupe(photo, Point::new(80.0, 60.0), 4000.0).unwrap();
    assert_eq!(big.image.width(), 1024);
    assert!(big.scale > 1.0);
    // Outside the photo it is white; junk gives nothing.
    let corner = engine.loupe(photo, Point::new(-500.0, -500.0), 40.0).unwrap();
    assert!(corner.image.data().iter().all(|p| *p == [255, 255, 255]));
    assert!(engine.loupe(photo, Point::new(f64::NAN, 0.0), 40.0).is_none());
    assert!(engine.loupe(photo, Point::new(0.0, 0.0), -3.0).is_none());
    assert!(engine.loupe(PhotoId(99), Point::new(0.0, 0.0), 40.0).is_none());
}

// ---- export ----

fn finish_one_page(engine: &mut scancraft_engine::Engine) -> PageId {
    let page = to_screen_three(engine);
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    page
}

#[test]
fn a_pdf_is_written_and_the_session_is_marked_exported() {
    let dir = TempDir::new("export");
    let (bytes, quad) = flat_fixture();
    let mut engine = open(&dir, services(quad));
    let path = write(&dir, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let page = finish_one_page(&mut engine);
    settle(&mut engine);
    let out = dir.join("Scan.pdf");
    assert!(engine.view().last_export.is_none());
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    // The export is reported as progress, and the screen is not taken away from the page.
    assert_eq!(engine.view().exporting, Some(scancraft_core::engine_api::Progress { done: 0, total: 1 }));
    assert_eq!(engine.view().screen, Screen::Enhance(page));
    let notices = texts(&settle(&mut engine));
    let pdf = std::fs::read(&out).expect("the PDF exists");
    assert!(pdf.starts_with(b"%PDF-"), "{notices:?}");
    assert!(engine.session().file().exported);
    assert!(notices.iter().any(|n| n.starts_with("Exported 1 page")), "{notices:?}");
    assert_eq!(engine.view().screen, Screen::Enhance(page));
    // What the export wrote is told in the view: where, how many pages, how big.
    assert!(engine.view().exporting.is_none());
    let summary = engine.view().last_export.expect("the export's summary");
    assert_eq!(summary.pages, 1);
    assert_eq!(summary.path, out.to_string_lossy());
    assert_eq!(summary.bytes, pdf.len() as u64);
    // A PDF can be imported again: each page is a photo whose default is Use Full Image.
    let mut second = open_with(dir.path(), "Scan B", services(quad), 2);
    second.dispatch(Command::ImportAdd { paths: vec![out.to_string_lossy().into_owned()] }).unwrap();
    settle(&mut second);
    open_crop_on_first_page(&mut second);
    let crop = second.view().crop.expect("the PDF's page is on Screen 2");
    assert!(matches!(crop.outlines[0].corners_from, CornerOrigin::FullImage));
    assert!(second.dispatch(Command::CropSetFlatten { page: crop.outlines[0].page, mode: FlattenMode::On }).is_err());
}

#[test]
fn a_pdf_gives_one_item_for_each_of_its_pages() {
    let dir = TempDir::new("pdfpages");
    // Two pages, written with the same writer the export uses.
    let mut picture = Vec::new();
    for y in 0..40 {
        for x in 0..30 {
            picture.push([200 - (x * 3) as u8, 210, 220 - (y * 2) as u8]);
        }
    }
    let image = scancraft_core::image::Plane::from_vec(30, 40, picture).unwrap();
    let encoded = scancraft_io::encode::encode(&scancraft_io::encode::PageImage::Colour(image), scancraft_core::recipe::Quality::Small).unwrap();
    let pages: Vec<scancraft_io::pdf_write::PdfPage> = (0..2)
        .map(|_| scancraft_io::pdf_write::PdfPage {
            image: encoded.clone(),
            size_pt: (595.0, 842.0),
            image_rect_pt: [0.0, 0.0, 595.0, 842.0],
            text: None,
        })
        .collect();
    let path = dir.join("two.pdf");
    scancraft_io::pdf_write::write_pdf(&pages, &scancraft_io::pdf_write::DocInfo { title: "two".to_string(), producer: "test".to_string() }, &path)
        .unwrap();
    let mut engine = open(&dir, services(desk_quad()));
    engine.dispatch(Command::ImportAdd { paths: vec![path.to_string_lossy().into_owned()] }).unwrap();
    settle(&mut engine);
    let photos = &engine.session().file().photos;
    assert_eq!(photos.len(), 2);
    assert_eq!(photos[0].name, "two.pdf, page 1");
    assert_eq!(photos[1].name, "two.pdf, page 2");
    // A PDF page's default is Use Full Image, and flattening isn't offered for it.
    open_crop_on_first_page(&mut engine);
    let crop = engine.view().crop.expect("Screen 2");
    assert!(matches!(crop.outlines[0].corners_from, CornerOrigin::FullImage));
}

#[test]
fn a_page_is_exported_without_anything_being_added() {
    let dir = TempDir::new("export3");
    let mut engine = engine_with_photo(&dir);
    let page = to_screen_three(&mut engine);
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    // The page was in the document before it was looked at.
    assert_eq!(engine.view().tray.len(), 1);
    let out = dir.join("from3.pdf");
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    settle(&mut engine);
    assert_eq!(engine.view().tray.len(), 1);
    assert_eq!(engine.view().tray[0].page, page);
    assert!(out.is_file());
}

#[test]
fn exporting_nothing_is_an_error_and_so_is_exporting_while_a_photo_is_still_being_read() {
    let dir = TempDir::new("confirm");
    let (gate, held) = std::sync::mpsc::channel();
    let services =
        Services { detector: Some(Arc::new(HeldDetector { quad: desk_quad(), gate: std::sync::Mutex::new(held) })), recognizer: None, upright: None };
    let mut engine = open(&dir, services);
    let out = dir.join("two.pdf").to_string_lossy().into_owned();
    let export = |e: &mut scancraft_engine::Engine| e.dispatch(Command::ExportPdf { path: out.clone(), options: ExportOptions::default() });
    assert!(export(&mut engine).unwrap_err().contains("no pages"));
    let path = write(&dir, "desk.png", &desk_png(0));
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    // The photo has no page yet and would be left out without a word: the export says to wait.
    let error = export(&mut engine).unwrap_err();
    assert!(error.contains("still being read"), "{error}");
    assert!(!dir.join("two.pdf").exists());
    gate.send(()).unwrap();
    settle(&mut engine);
    export(&mut engine).unwrap();
    settle(&mut engine);
    assert!(dir.join("two.pdf").exists());
    assert!(engine.dispatch(Command::ExportPdf { path: String::new(), options: ExportOptions::default() }).is_err());
}

#[test]
fn a_pdf_does_not_depend_on_the_number_of_threads() {
    let (bytes, quad) = flat_fixture();
    let mut outputs: Vec<((usize, Option<Look>), Vec<u8>)> = Vec::new();
    for threads in [1, 8] {
        let dir = TempDir::new("threads");
        let mut engine = open_with(dir.path(), "Scan A", services(quad), threads);
        let path = write(&dir, "flat.png", &bytes);
        engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
        settle(&mut engine);
        open_crop_on_first_page(&mut engine);
        let page = to_screen_three(&mut engine);
        for look in [Look::Original, Look::Grayscale, Look::BlackWhite] {
            engine.dispatch(Command::PageSetLook { page, look }).unwrap();
            let preview = loop {
                match engine.image(ImageKey::LookPreview(page)) {
                    Some(image) => break image,
                    None => {
                        settle(&mut engine);
                    }
                }
            };
            outputs.push(((threads, Some(look)), preview.data().iter().flatten().copied().collect::<Vec<u8>>()));
        }
        engine.dispatch(Command::PageSetLook { page, look: Look::Grayscale }).unwrap();
        settle(&mut engine);
        let out = dir.join("t.pdf");
        engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
        settle(&mut engine);
        let mut pdf = std::fs::read(&out).unwrap();
        // The two dates in the document information are the only thing that depends on the clock.
        let mut at = 0;
        while let Some(i) = pdf[at..].windows(3).position(|w| w == b"(D:") {
            let start = at + i + 3;
            for b in pdf[start..].iter_mut().take_while(|b| **b != b')') {
                *b = b'0';
            }
            at = start;
        }
        outputs.push(((threads, None), pdf));
    }
    let one: Vec<_> = outputs.iter().filter(|o| o.0.0 == 1).collect();
    let eight: Vec<_> = outputs.iter().filter(|o| o.0.0 == 8).collect();
    assert_eq!(one.len(), eight.len());
    for (a, b) in one.iter().zip(&eight) {
        assert!(a.1 == b.1, "one thread and eight threads differ for {:?}", a.0);
    }
}

// ---- sessions ----

#[test]
fn a_session_comes_back_after_the_engine_is_closed() {
    let dir = TempDir::new("reopen");
    {
        let mut engine = engine_with_photo(&dir);
        finish_one_page(&mut engine);
        settle(&mut engine);
        engine.flush();
    }
    let engine = open(&dir, services(desk_quad()));
    assert_eq!(engine.view().tray.len(), 1);
    assert_eq!(engine.session().file().photos.len(), 1);
    assert_eq!(engine.session().file().pages[0].look, Look::Original);
}

#[test]
fn the_last_unexported_session_is_offered_and_restored() {
    let dir = TempDir::new("offer");
    {
        let mut engine = engine_with_photo(&dir);
        finish_one_page(&mut engine);
        settle(&mut engine);
        engine.flush();
    }
    let mut engine = open_with(dir.path(), "Scan Z", services(desk_quad()), 2);
    assert_eq!(engine.view().restore_offer, Some(1));
    assert!(engine.view().tray.is_empty());
    engine.dispatch(Command::SessionRestore).unwrap();
    assert_eq!(engine.view().tray.len(), 1);
    assert_eq!(engine.view().restore_offer, None);
    assert!(engine.dispatch(Command::SessionRestore).is_err(), "nothing left to restore");
}

#[test]
fn a_new_session_starts_empty_in_its_own_folder_and_keeps_the_old_one() {
    let dir = TempDir::new("new");
    let mut engine = engine_with_photo(&dir);
    finish_one_page(&mut engine);
    settle(&mut engine);
    let old = engine.session_dir().clone();
    engine.dispatch(Command::SessionNew).unwrap();
    let new = engine.session_dir().clone();
    assert_ne!(old, new);
    assert!(new.file_name().unwrap().to_string_lossy().starts_with("Scan 20"));
    assert!(old.join("session.json").is_file(), "the old session stays for a restore");
    assert!(engine.view().tray.is_empty() && !engine.view().can_undo);
    assert_eq!(engine.view().screen, Screen::Import);
    assert_eq!(engine.view().restore_offer, Some(1));
    // The new session works.
    import_desk(&mut engine, &dir, 9);
    assert!(matches!(engine.view().screen, Screen::Crop(_)));
}

#[test]
fn a_new_scans_folder_is_used_by_the_next_new_scan() {
    let dir = TempDir::new("scans-folder");
    let elsewhere = TempDir::new("scans-folder-elsewhere");
    let mut engine = engine_with_photo(&dir);
    let chosen = elsewhere.path().to_string_lossy().into_owned();
    engine.set_settings(|s| s.sessions_dir = Some(chosen));
    engine.dispatch(Command::SessionNew).unwrap();
    assert_eq!(engine.session_dir().parent(), Some(elsewhere.path()), "the new scan is not in the folder that was chosen");
    // The scan that was open stays where it was.
    assert!(dir.path().join("Scan A").join("session.json").is_file());
}

#[test]
fn book_spreads_stays_on_for_the_next_session() {
    let dir = TempDir::new("bookdefault");
    let mut engine = engine_with_photo(&dir);
    engine.dispatch(Command::CropBook { on: true }).unwrap();
    engine.dispatch(Command::SessionNew).unwrap();
    assert!(engine.view().book_spreads);
    import_desk(&mut engine, &dir, 2);
    assert_eq!(engine.view().crop.unwrap().outlines.len(), 2);
}

#[test]
fn the_session_is_written_at_most_once_a_second() {
    let dir = TempDir::new("saves");
    let mut engine = engine_with_photo(&dir);
    let clock = Arc::new(AtomicU64::new(10_000));
    let c = clock.clone();
    engine.set_clock(move || c.load(Ordering::Relaxed));
    engine.save_now().unwrap();
    let (session, pending) = (engine.session_dir().join("session.json"), engine.session_dir().join("pending.json"));
    let (_, page) = current(&engine);
    // Every page is in session.json: there is no pending.json any more.
    let on_disk = || (std::fs::read(&session).unwrap(), std::fs::read(&pending).unwrap_or_default());
    let saved = on_disk();
    clock.store(10_400, Ordering::Relaxed);
    engine.dispatch(Command::PageRotate { page, quarter_turns: 1 }).unwrap();
    engine.poll();
    assert_eq!(on_disk(), saved, "400 ms after a save nothing is written");
    clock.store(10_999, Ordering::Relaxed);
    engine.poll();
    assert_eq!(on_disk(), saved);
    clock.store(11_000, Ordering::Relaxed);
    engine.poll();
    assert_ne!(on_disk(), saved, "a second after the save the change is written");
}

#[test]
fn back_to_crop_opens_the_corners_screen_with_the_pages_corners() {
    let dir = TempDir::new("opencrop");
    let (bytes, quad) = flat_fixture();
    let mut engine = open(&dir, services(quad));
    let path = write(&dir, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let (photo, page) = current(&engine);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    assert!(matches!(engine.view().screen, Screen::Enhance(_)));
    engine.dispatch(Command::PageOpenCrop { page }).unwrap();
    let view = engine.view();
    assert!(matches!(view.screen, Screen::Crop(p) if p == photo), "{:?}", view.screen);
    let crop = view.crop.expect("Screen 2");
    assert_eq!(crop.outlines.len(), 1);
    assert_eq!(crop.outlines[0].page, page);
    // The corners are the ones the page had.
    for (got, want) in crop.outlines[0].corners.0.iter().zip(quad.0.iter()) {
        assert!(got.distance(*want) < 1.0, "{got:?} against {want:?}");
    }
}

/// Imports the flat fixture from `set` (so the photo is in the reference folder) and returns its page.
fn labelled_setup(dir: &TempDir, set: &TempDir) -> (scancraft_engine::Engine, PageId) {
    let (bytes, quad) = flat_fixture();
    let mut engine = open(dir, services(quad));
    let path = write(set, "flat.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let (photo, page) = current(&engine);
    engine.dispatch(Command::CropApply { photo }).unwrap();
    settle(&mut engine);
    (engine, page)
}

#[test]
fn a_label_is_written_and_written_again() {
    let dir = TempDir::new("label");
    let set = TempDir::new("labelset");
    let (mut engine, page) = labelled_setup(&dir, &set);
    let folder = set.path().to_string_lossy().into_owned();
    engine.dispatch(Command::ReferenceLabel { page, folder: folder.clone(), category: "dark desk".to_string() }).unwrap();
    let text = std::fs::read_to_string(set.join("reference.json")).expect("the label file");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let photos = value["photos"].as_array().expect("the photos");
    assert_eq!(photos.len(), 1, "{text}");
    assert_eq!(photos[0]["file"], "flat.png");
    assert_eq!(photos[0]["category"], "dark desk");
    assert_eq!(photos[0]["sha256"].as_str().map(str::len), Some(64));
    let corners = photos[0]["corners"].as_array().expect("the corners");
    assert_eq!(corners.len(), 4);
    // Labelling the same photo again replaces its record, and keeps the others.
    engine.dispatch(Command::ReferenceLabel { page, folder: folder.clone(), category: "wood".to_string() }).unwrap();
    let text = std::fs::read_to_string(set.join("reference.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let photos = value["photos"].as_array().unwrap();
    assert_eq!(photos.len(), 1, "{text}");
    assert_eq!(photos[0]["category"], "wood");
    // A blank category, one that is not among the 18, and a blank folder are refused.
    assert!(engine.dispatch(Command::ReferenceLabel { page, folder: folder.clone(), category: "  ".to_string() }).is_err());
    let error = engine.dispatch(Command::ReferenceLabel { page, folder: folder.clone(), category: "a nice photo".to_string() }).unwrap_err();
    assert!(error.contains("a nice photo"), "{error}");
    // The refused label left the one before it in place.
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(set.join("reference.json")).unwrap()).unwrap();
    assert_eq!(value["photos"][0]["category"], "wood");
    assert!(engine.dispatch(Command::ReferenceLabel { page, folder: " ".to_string(), category: "wood".to_string() }).is_err());
}

#[test]
fn a_label_holds_the_corners_and_nothing_the_pipeline_guessed() {
    let dir = TempDir::new("label-truth");
    let set = TempDir::new("label-truth-set");
    let (mut engine, page) = labelled_setup(&dir, &set);
    let folder = set.path().to_string_lossy().into_owned();
    engine.dispatch(Command::ReferenceLabel { page, folder, category: "dark desk".to_string() }).unwrap();
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(set.join("reference.json")).unwrap()).unwrap();
    let record = &value["photos"][0];
    // The ratio on the page is the pipeline's estimate (A11), and a label is the person's truth.
    assert!(record.get("ratio").is_none(), "the label carries the pipeline's own ratio: {record}");
    // And the reference loader reads it: the file is there, and its bytes hash to the label's.
    let loaded = scancraft_synth::datasets::reference::load(set.path()).unwrap();
    assert_eq!(loaded.samples.len(), 1, "skipped: {:?}", loaded.skipped);
    assert!(loaded.samples[0].corners.is_some());
}

#[test]
fn a_photo_that_is_not_in_the_reference_folder_is_not_labelled() {
    // The label names the file and its hash, and the loader looks for both in the folder: a label
    // for a photo kept elsewhere would be skipped when the set is read, and say nothing.
    let dir = TempDir::new("label-elsewhere");
    let elsewhere = TempDir::new("label-elsewhere-photos");
    let set = TempDir::new("label-elsewhere-set");
    let (mut engine, page) = labelled_setup(&dir, &elsewhere);
    let folder = set.path().to_string_lossy().into_owned();
    let error = engine.dispatch(Command::ReferenceLabel { page, folder, category: "wood".to_string() }).unwrap_err();
    assert!(error.contains("flat.png"), "{error}");
    assert!(!set.join("reference.json").exists(), "a label file was written for a photo that is not in the folder");
}

#[test]
fn a_photo_with_the_same_name_and_other_bytes_is_not_labelled() {
    let dir = TempDir::new("label-changed");
    let set = TempDir::new("label-changed-set");
    let (mut engine, page) = labelled_setup(&dir, &set);
    // Another picture with the name of the one that was imported.
    std::fs::write(set.join("flat.png"), desk_png(9)).unwrap();
    let folder = set.path().to_string_lossy().into_owned();
    assert!(engine.dispatch(Command::ReferenceLabel { page, folder, category: "wood".to_string() }).is_err());
    assert!(!set.join("reference.json").exists());
}

#[test]
fn a_page_of_a_pdf_is_not_labelled() {
    // A PDF page is not a photo with a file of its own, so a label for it could never be read back.
    let dir = TempDir::new("label-pdf");
    let set = TempDir::new("label-pdf-set");
    let mut picture = Vec::new();
    for y in 0..40 {
        for x in 0..30 {
            picture.push([200 - (x * 3) as u8, 210, 220 - (y * 2) as u8]);
        }
    }
    let image = scancraft_core::image::Plane::from_vec(30, 40, picture).unwrap();
    let encoded = scancraft_io::encode::encode(&scancraft_io::encode::PageImage::Colour(image), scancraft_core::recipe::Quality::Small).unwrap();
    let pages: Vec<scancraft_io::pdf_write::PdfPage> = (0..2)
        .map(|_| scancraft_io::pdf_write::PdfPage {
            image: encoded.clone(),
            size_pt: (595.0, 842.0),
            image_rect_pt: [0.0, 0.0, 595.0, 842.0],
            text: None,
        })
        .collect();
    let path = set.join("two.pdf");
    scancraft_io::pdf_write::write_pdf(&pages, &scancraft_io::pdf_write::DocInfo { title: "two".to_string(), producer: "test".to_string() }, &path)
        .unwrap();
    let mut engine = open(&dir, services(desk_quad()));
    engine.dispatch(Command::ImportAdd { paths: vec![path.to_string_lossy().into_owned()] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let (_, page) = current(&engine);
    let folder = set.path().to_string_lossy().into_owned();
    let error = engine.dispatch(Command::ReferenceLabel { page, folder, category: "receipt".to_string() }).unwrap_err();
    assert!(error.contains("PDF"), "{error}");
    assert!(!set.join("reference.json").exists());
}

#[test]
fn a_book_spread_is_labelled_with_the_whole_spread_and_its_spine() {
    // The photo is cut into two pages whose corners are halves of the spread. The truth of the
    // photo is the spread's outline and its spine; a half's corners would be wrong, and the second
    // half would replace the first.
    let dir = TempDir::new("label-spread");
    let set = TempDir::new("label-spread-set");
    let (bytes, quad) = flat_fixture();
    let mut engine = open(&dir, services(quad));
    let path = write(&set, "spread.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let (photo, _) = current(&engine);
    engine.dispatch(Command::CropBook { on: true }).unwrap();
    engine.dispatch(Command::CropSetSpine { photo, top: Point::new(430.0, 70.0), bottom: Point::new(450.0, 590.0) }).unwrap();
    let crop = engine.view().crop.unwrap();
    assert_eq!(crop.outlines.len(), 2);
    let (left, right) = (crop.outlines[0].page, crop.outlines[1].page);
    let folder = set.path().to_string_lossy().into_owned();
    let label = |engine: &mut scancraft_engine::Engine, page| {
        engine.dispatch(Command::ReferenceLabel { page, folder: folder.clone(), category: "book spread".to_string() }).unwrap();
        let text = std::fs::read_to_string(set.join("reference.json")).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()
    };
    let from_left = label(&mut engine, left);
    let from_right = label(&mut engine, right);
    // One record for the photo whichever half was labelled, and the same one.
    assert_eq!(from_left["photos"].as_array().unwrap().len(), 1);
    assert_eq!(from_left, from_right);
    let record = &from_left["photos"][0];
    // The spine is there, and it is the one that was set.
    assert_eq!(record["spine"], serde_json::json!([[430.0, 70.0], [450.0, 590.0]]));
    // The corners are the spread's, so the spine lies between the left and right sides.
    let corners = record["corners"].as_array().unwrap();
    let (left_x, right_x) = (corners[0][0].as_f64().unwrap(), corners[1][0].as_f64().unwrap());
    assert!(left_x < 430.0 && 450.0 < right_x, "the spine is not inside the labelled outline: {corners:?}");
    // The loader reads it back with the spine.
    let loaded = scancraft_synth::datasets::reference::load(set.path()).unwrap();
    assert_eq!(loaded.samples.len(), 1, "skipped: {:?}", loaded.skipped);
    assert!(loaded.samples[0].spine.is_some());
}

#[test]
fn book_spreads_put_the_cut_on_the_spine_a30_finds() {
    // The drawn spread: its true spine is in the truth file. The detector is given the true
    // outline, so the only thing under test is where the pages are cut.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let bytes = std::fs::read(root.join("spread.png")).unwrap();
    let truth: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("spread.json")).unwrap()).unwrap();
    let point = |v: &serde_json::Value| Point::new(v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
    let quad = scancraft_core::geom::Quad([
        point(&truth["corners"][0]),
        point(&truth["corners"][1]),
        point(&truth["corners"][2]),
        point(&truth["corners"][3]),
    ]);
    let spine = [point(&truth["spine"][0]), point(&truth["spine"][1])];
    let dir = TempDir::new("spine");
    let set = TempDir::new("spine-set");
    let mut engine = open(&dir, services(quad));
    let path = write(&set, "spread.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    engine.dispatch(Command::CropBook { on: true }).unwrap();
    settle(&mut engine);
    let crop = engine.view().crop.unwrap();
    assert_eq!(crop.outlines.len(), 2);
    let found = crop.spine.unwrap();
    let width = quad.tl().distance(quad.tr());
    for end in 0..2 {
        // Along the spine an end may slide to where the spread's own top or bottom side is; across
        // it, the spine has to be within 1% of the spread's width (§10.4).
        let miss = (found[end].x - spine[end].x).abs();
        assert!(miss < 0.01 * width, "end {end} of the spine is {miss:.1} px off the truth ({:?} against {:?})", found[end], spine[end]);
    }
}

#[test]
fn a_spine_the_user_moved_is_not_moved_back_by_a30() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let bytes = std::fs::read(root.join("spread.png")).unwrap();
    let (_, quad) = flat_fixture();
    let dir = TempDir::new("spine-user");
    let set = TempDir::new("spine-user-set");
    let mut engine = open(&dir, services(quad));
    let path = write(&set, "spread.png", &bytes);
    engine.dispatch(Command::ImportAdd { paths: vec![path] }).unwrap();
    settle(&mut engine);
    open_crop_on_first_page(&mut engine);
    let (photo, _) = current(&engine);
    engine.dispatch(Command::CropBook { on: true }).unwrap();
    engine.dispatch(Command::CropSetSpine { photo, top: Point::new(430.0, 70.0), bottom: Point::new(450.0, 590.0) }).unwrap();
    settle(&mut engine);
    let found = engine.view().crop.unwrap().spine.unwrap();
    assert_eq!((found[0], found[1]), (Point::new(430.0, 70.0), Point::new(450.0, 590.0)));
}
