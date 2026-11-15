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
fn the_first_photo_opens_screen_two_with_the_outline_a_detector_found() {
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
    assert!(view.tray.is_empty());
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
    assert!(matches!(engine.view().screen, Screen::Crop(_)));
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
    // The import is a step of its own: undoing it takes the photo away, and redoing brings it back.
    assert!(engine.view().can_undo);
    engine.dispatch(Command::EditUndo).unwrap();
    assert!(engine.session().file().photos.is_empty());
    assert_eq!(engine.view().screen, Screen::Import);
    engine.dispatch(Command::EditRedo).unwrap();
    assert_eq!(engine.session().file().photos.len(), 1);
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
    // Applying opens the left page; adding it opens the right page on the same look.
    engine.dispatch(Command::CropApply { photo }).unwrap();
    let Screen::Enhance(left) = engine.view().screen else { panic!("Screen 3 expected") };
    assert_eq!(engine.view().enhance.unwrap().half, Some(scancraft_core::recipe::Half::Left));
    engine.dispatch(Command::PageSetLook { page: left, look: Look::Grayscale }).unwrap();
    assert!(engine.view().enhance.unwrap().more_to_come);
    engine.dispatch(Command::PageAdd { page: left }).unwrap();
    let view = engine.view();
    let enhance = view.enhance.unwrap();
    assert_eq!(enhance.half, Some(scancraft_core::recipe::Half::Right));
    assert_eq!(enhance.look, Look::Grayscale);
    assert!(!enhance.more_to_come);
    // Adding the second page ends the batch.
    engine.dispatch(Command::PageAdd { page: enhance.page }).unwrap();
    assert_eq!(engine.view().screen, Screen::Import);
    assert_eq!(engine.view().tray.len(), 2);
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
    let mut ids = Vec::new();
    for _ in 0..3 {
        let page = to_screen_three(&mut engine);
        engine.dispatch(Command::PageAdd { page }).unwrap();
        ids.push(page);
    }
    assert_eq!(engine.view().screen, Screen::Import);
    let tray = |e: &scancraft_engine::Engine| e.view().tray.iter().map(|t| t.page).collect::<Vec<_>>();
    assert_eq!(tray(&engine), ids);
    assert_eq!(engine.view().tray.iter().map(|t| t.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(engine.dispatch(Command::PageAdd { page: ids[0] }).is_err(), "a page is added once");
    engine.dispatch(Command::PageMove { from: 2, to: 0 }).unwrap();
    assert_eq!(tray(&engine), vec![ids[2], ids[0], ids[1]]);
    assert!(engine.dispatch(Command::PageMove { from: 3, to: 0 }).is_err());
    assert!(engine.dispatch(Command::PageMove { from: 0, to: u32::MAX }).is_err());
    engine.dispatch(Command::PageEdit { page: ids[1] }).unwrap();
    assert_eq!(engine.view().screen, Screen::Enhance(ids[1]));
    engine.dispatch(Command::PageDelete { page: ids[1] }).unwrap();
    assert_eq!(engine.view().screen, Screen::Import);
    assert_eq!(tray(&engine), vec![ids[2], ids[0]]);
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
    engine.dispatch(Command::PageAdd { page }).unwrap();
    settle(&mut engine);
    assert!(engine.view().tray[0].text_ready);
    let text = engine.session().file().pages[0].text.clone().unwrap();
    assert_eq!(text.engine, "fake");
    // Turning the page throws the text away: its positions belonged to the old turn.
    engine.dispatch(Command::PageRotate { page, quarter_turns: 1 }).unwrap();
    assert!(!engine.view().tray[0].text_ready);
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
    engine.dispatch(Command::PageAdd { page }).unwrap();
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
    finish_one_page(&mut engine);
    settle(&mut engine);
    let out = dir.join("Scan.pdf");
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    assert_eq!(engine.view().screen, Screen::Export);
    let notices = texts(&settle(&mut engine));
    let pdf = std::fs::read(&out).expect("the PDF exists");
    assert!(pdf.starts_with(b"%PDF-"), "{notices:?}");
    assert!(engine.session().file().exported);
    assert!(notices.iter().any(|n| n.starts_with("Exported 1 page")), "{notices:?}");
    assert_eq!(engine.view().screen, Screen::Import);
    // A PDF can be imported again: each page is a photo whose default is Use Full Image.
    let mut second = open_with(dir.path(), "Scan B", services(quad), 2);
    second.dispatch(Command::ImportAdd { paths: vec![out.to_string_lossy().into_owned()] }).unwrap();
    settle(&mut second);
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
    let crop = engine.view().crop.expect("Screen 2");
    assert!(matches!(crop.outlines[0].corners_from, CornerOrigin::FullImage));
}

#[test]
fn exporting_from_screen_three_adds_the_page_being_looked_at_first() {
    let dir = TempDir::new("export3");
    let mut engine = engine_with_photo(&dir);
    let page = to_screen_three(&mut engine);
    engine.dispatch(Command::PageSetLook { page, look: Look::Original }).unwrap();
    // The page is still on Screen 3, not in the tray.
    assert!(engine.view().tray.is_empty());
    let out = dir.join("from3.pdf");
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options: ExportOptions::default() }).unwrap();
    settle(&mut engine);
    assert_eq!(engine.view().tray.len(), 1);
    assert_eq!(engine.view().tray[0].page, page);
    assert!(out.is_file());
}

#[test]
fn exporting_with_photos_not_added_asks_first_and_exporting_nothing_is_an_error() {
    let dir = TempDir::new("confirm");
    let mut engine = open(&dir, services(desk_quad()));
    let out = dir.join("two.pdf").to_string_lossy().into_owned();
    let export = |e: &mut scancraft_engine::Engine| e.dispatch(Command::ExportPdf { path: out.clone(), options: ExportOptions::default() });
    import_desk(&mut engine, &dir, 0);
    assert!(export(&mut engine).unwrap_err().contains("no pages"));
    import_desk(&mut engine, &dir, 1);
    let first = to_screen_three(&mut engine);
    engine.dispatch(Command::PageSetLook { page: first, look: Look::Original }).unwrap();
    engine.dispatch(Command::PageAdd { page: first }).unwrap();
    settle(&mut engine);
    let question = export(&mut engine).unwrap_err();
    assert_eq!(question, "1 photo hasn't been added. Export the 1 added page?");
    assert!(!dir.join("two.pdf").exists());
    // Another command in between withdraws the question; asking again repeats it.
    engine.dispatch(Command::PageEdit { page: first }).unwrap();
    assert!(export(&mut engine).is_err());
    // Asked and confirmed: the same call goes through.
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
        engine.dispatch(Command::PageAdd { page }).unwrap();
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
    // The page being worked on lives in pending.json until it is added to the tray.
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
