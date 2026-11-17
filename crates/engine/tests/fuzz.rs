//! Hostile input: whatever a program or a damaged session throws at the engine, it answers with an
//! error or does the sensible thing, and the session stays one the engine can still read.

mod common;

use common::*;
use scancraft_core::command::{Command, Corner};
use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::engine_api::{EngineApi, ImageKey};
use scancraft_core::geom::{Point, Side};
use scancraft_core::recipe::{ExportOptions, FlattenMode, Look, PageId, PhotoId, SessionFile};
use scancraft_engine::Engine;

const ITERATIONS: usize = 12_000;

struct Fuzz {
    rng: Xorshift64Star,
    /// The pages and photos the session has now, so most commands name one that exists: pages are
    /// numbered up from 1 and the numbers run away from any fixed range as photos come and go.
    pages: Vec<u64>,
    photos: Vec<u64>,
    files: Vec<String>,
    folder: String,
    out: String,
}

impl Fuzz {
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.rng.below(items.len() as u64) as usize]
    }

    fn float(&mut self) -> f64 {
        const SPECIAL: [f64; 12] = [0.0, -0.0, 1.0, 80.0, -1e300, 1e300, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MIN_POSITIVE, 1e-320, -1.0];
        if self.rng.below(3) == 0 { self.pick(&SPECIAL) } else { self.rng.next_f64() * 300.0 - 70.0 }
    }

    fn point(&mut self) -> Point {
        Point::new(self.float(), self.float())
    }

    fn page(&mut self) -> PageId {
        if !self.pages.is_empty() && self.rng.below(4) != 0 {
            return PageId(self.pick(&self.pages.clone()));
        }
        PageId(if self.rng.below(8) == 0 { self.pick(&[0, u64::MAX, 1 << 40]) } else { self.rng.below(14) })
    }

    fn photo(&mut self) -> PhotoId {
        if !self.photos.is_empty() && self.rng.below(4) != 0 {
            return PhotoId(self.pick(&self.photos.clone()));
        }
        PhotoId(if self.rng.below(8) == 0 { self.pick(&[0, u64::MAX, 1 << 40]) } else { self.rng.below(6) })
    }

    fn corner(&mut self) -> Corner {
        self.pick(&[Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft])
    }

    fn side(&mut self) -> Side {
        self.pick(&Side::ALL)
    }

    fn index(&mut self) -> u32 {
        if self.rng.below(4) == 0 { self.pick(&[u32::MAX, 500, 1 << 31]) } else { self.rng.below(6) as u32 }
    }

    fn command(&mut self) -> Command {
        // The commands that touch the disk or replace the session are rare, so a run gets somewhere.
        match self.rng.below(3000) {
            0 => return Command::SessionNew,
            1 => return Command::SessionRestore,
            2..=10 => return Command::ExportPdf { path: self.out.clone(), options: ExportOptions::default() },
            11..=40 => {
                let mut paths = vec![self.pick_file()];
                if self.rng.below(4) == 0 {
                    paths.push(self.folder.clone());
                }
                return Command::ImportAdd { paths };
            }
            // A label touches the disk, so it is rare too. Its folder is the one the photos were
            // imported from (where a label can succeed) or a junk path (where it can't).
            41..=190 => {
                let folder = match self.rng.below(5) {
                    0 => String::new(),
                    1 => "\0".to_string(),
                    2 => "/definitely/not/here".to_string(),
                    _ => self.folder.clone(),
                };
                let category = self.pick(&["", "dark desk", "wood", "not a category", "\0"]).to_string();
                return Command::ReferenceLabel { page: self.page(), folder, category };
            }
            // The commands of the review-first flow are rare too, so the run's other commands keep
            // the same share of it.
            191..=210 => return Command::PageMarkReviewed { page: self.page() },
            211..=230 => {
                let look = self.pick(&[Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument]);
                return Command::PageSetLookAll { look, strength: self.rng.below(256) as u8 };
            }
            231..=240 => return Command::ExportCancel,
            241..=250 => return Command::PageRotateMany { pages: vec![self.page(), self.page()], quarter_turns: self.rng.below(256) as u8 as i8 },
            251..=255 => return Command::PageDeleteMany { pages: vec![self.page(), self.page()] },
            256..=265 => {
                let look = self.pick(&[Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument]);
                return Command::PageSetLookMany { pages: vec![self.page(), self.page()], look, strength: self.rng.below(256) as u8 };
            }
            _ => {}
        }
        match self.rng.below(23) {
            0 => Command::ImportDiscard { item: self.photo() },
            1 | 2 => Command::CropSetCorner { page: self.page(), corner: self.corner(), x: self.float(), y: self.float() },
            3 | 4 => Command::CropMoveSide { page: self.page(), side: self.side(), offset: self.float() },
            5 => Command::CropBendSide { page: self.page(), side: self.side(), handles: [self.point(), self.point(), self.point()] },
            6 => Command::CropStraightenSide { page: self.page(), side: self.side() },
            7 => Command::CropSetFlatten { page: self.page(), mode: self.pick(&[FlattenMode::Auto, FlattenMode::On, FlattenMode::Off]) },
            8 => Command::CropBook { on: self.rng.below(2) == 0 },
            9 => Command::CropSplit { photo: self.photo(), on: self.rng.below(2) == 0 },
            10 => Command::CropSetSpine { photo: self.photo(), top: self.point(), bottom: self.point() },
            11 => Command::CropAuto { page: self.page() },
            12 => Command::CropFullImage { page: self.page() },
            13 => Command::CropApply { photo: self.photo() },
            14 => {
                Command::PageSetLook { page: self.page(), look: self.pick(&[Look::Original, Look::BlackWhite, Look::Grayscale, Look::ColorDocument]) }
            }
            15 => Command::PageSetStrength { page: self.page(), value: self.rng.below(256) as u8 },
            16 => Command::PageRotate { page: self.page(), quarter_turns: self.rng.below(256) as u8 as i8 },
            17 => Command::PageAdd { page: self.page() },
            18 => Command::PageDelete { page: self.page() },
            19 => Command::PageMove { from: self.index(), to: self.index() },
            20 => Command::PageEdit { page: self.page() },
            21 => Command::PageOpenCrop { page: self.page() },
            _ => {
                if self.rng.below(2) == 0 {
                    Command::EditUndo
                } else {
                    Command::EditRedo
                }
            }
        }
    }

    fn pick_file(&mut self) -> String {
        match self.rng.below(8) {
            0 => String::new(),
            1 => "\0".to_string(),
            2 => "/definitely/not/here.png".to_string(),
            3 => self.out.clone(),
            _ => {
                let n = self.files.len() as u64;
                self.files[self.rng.below(n) as usize].clone()
            }
        }
    }
}

/// What must hold of the session whatever was done to it.
fn check(engine: &Engine) {
    let file: &SessionFile = engine.session().file();
    assert!(file.pages.len() <= 500);
    let mut ids = std::collections::BTreeSet::new();
    for page in file.pages.iter().chain(engine.session().pending()) {
        assert!(ids.insert(page.id), "page {:?} twice", page.id);
        assert!(page.corners.is_finite(), "{:?}", page.corners);
        assert!(page.quarter_turns < 4 && page.strength <= 100);
        assert!(file.photos.iter().any(|p| p.id == page.photo), "a page without its photo");
    }
    // The view can always be built.
    let _ = engine.view();
}

fn run(seed: u64) {
    let dir = TempDir::new("fuzz");
    let mut engine = open(&dir, services(desk_quad()));
    let files: Vec<String> = (0..4).map(|i| write(&dir, &format!("desk{i}.png"), &desk_png(i))).collect();
    let folder = dir.path().to_string_lossy().into_owned();
    let out = dir.join("fuzz.pdf").to_string_lossy().into_owned();
    let mut fuzz = Fuzz { rng: Xorshift64Star::new(seed), pages: Vec::new(), photos: Vec::new(), files, folder, out };
    // The run starts from one page and one good label. Which commands the fuzzer can aim at a real
    // page, and so whether any label gets through, otherwise depends on how fast the workers read
    // the photos: on a loaded machine the loop ended before a page had joined, and the run wrote no
    // label at all. With the label there from the start, the junk that follows races a file that
    // exists, which is what the check at the end is about.
    engine.dispatch(Command::ImportAdd { paths: vec![fuzz.files[0].clone()] }).expect("the first photo imports");
    settle(&mut engine);
    let first = engine.view().tray.first().map(|t| t.page).expect("the first photo's page joins");
    let label = Command::ReferenceLabel { page: first, folder: fuzz.folder.clone(), category: "dark desk".to_string() };
    engine.dispatch(label).expect("the first label is written");
    let (mut accepted, mut most_pages) = (0usize, 0usize);
    for i in 0..ITERATIONS {
        let command = fuzz.command();
        if engine.dispatch(command).is_ok() {
            accepted += 1;
        }
        let tray = engine.view().tray;
        most_pages = most_pages.max(tray.len());
        fuzz.pages = tray.iter().map(|t| t.page.0).collect();
        fuzz.photos = engine.session().file().photos.iter().map(|p| p.id.0).collect();
        if i % 20 == 0 {
            engine.poll();
        }
        if i % 7 == 0 {
            let key = match fuzz.rng.below(4) {
                0 => ImageKey::DisplayCopy(fuzz.photo()),
                1 => ImageKey::LookPreview(fuzz.page()),
                2 => ImageKey::FilterThumb(fuzz.page(), Look::BlackWhite),
                _ => ImageKey::TrayThumb(fuzz.page()),
            };
            let _ = engine.image(key);
            let (photo, centre, side) = (fuzz.photo(), fuzz.point(), fuzz.float());
            let _ = engine.loupe(photo, centre, side);
        }
        if i % 1000 == 999 {
            settle(&mut engine);
            check(&engine);
        }
    }
    settle(&mut engine);
    check(&engine);
    // The run did real work: plenty of commands went through, and pages reached the tray.
    assert!(accepted > ITERATIONS / 20, "only {accepted} commands were accepted");
    assert!(most_pages > 0, "no page ever reached the tray");
    // Whatever labels were written, the reference loader reads them: a label file is never left
    // half written or with a record it can't use.
    let label_file = std::path::Path::new(&fuzz.folder).join("reference.json");
    // The run's first label is in it, or this checks nothing.
    assert!(label_file.exists(), "the label file is gone");
    let set = scancraft_synth::datasets::reference::load(std::path::Path::new(&fuzz.folder)).expect("the label file reads");
    assert!(!set.samples.is_empty(), "the label file holds no usable label");
    assert_eq!(set.skipped_count, 0, "a label the loader skips: {:?}", set.skipped);
    assert!(set.samples.iter().all(|s| s.corners.is_some_and(|q| q.is_finite())));
    // Saved after all that, the session reads back.
    engine.flush();
    let session = engine.session_dir().clone();
    drop(engine);
    let again = scancraft_engine::session::Session::open(session).expect("the session still opens");
    assert!(again.file().pages.len() <= 500);
}

#[test]
fn junk_commands_never_crash_the_engine_or_break_its_session() {
    run(0x5CA1_AB1E);
}

#[test]
fn junk_commands_with_another_seed() {
    run(0xFEED_FACE_0BAD_CAFE);
}

#[test]
fn every_command_id_has_a_command_and_parses_from_json() {
    let samples = [
        r#"{"id":"import.add","params":{"paths":["a.png"]}}"#,
        r#"{"id":"import.discard","params":{"item":1}}"#,
        r#"{"id":"crop.set_corner","params":{"page":1,"corner":"TopLeft","x":1.0,"y":2.0}}"#,
        r#"{"id":"crop.move_side","params":{"page":1,"side":"Top","offset":3.0}}"#,
        r#"{"id":"crop.bend_side","params":{"page":1,"side":"Top","handles":[{"x":1,"y":1},{"x":2,"y":1},{"x":3,"y":1}]}}"#,
        r#"{"id":"crop.straighten_side","params":{"page":1,"side":"Left"}}"#,
        r#"{"id":"crop.set_flatten","params":{"page":1,"mode":"On"}}"#,
        r#"{"id":"crop.book","params":{"on":true}}"#,
        r#"{"id":"crop.split","params":{"photo":1,"on":true}}"#,
        r#"{"id":"crop.set_spine","params":{"photo":1,"top":{"x":1,"y":0},"bottom":{"x":1,"y":9}}}"#,
        r#"{"id":"crop.auto","params":{"page":1}}"#,
        r#"{"id":"crop.full_image","params":{"page":1}}"#,
        r#"{"id":"crop.apply","params":{"photo":1}}"#,
        r#"{"id":"page.set_look","params":{"page":1,"look":"Grayscale"}}"#,
        r#"{"id":"page.set_strength","params":{"page":1,"value":70}}"#,
        r#"{"id":"page.set_look_all","params":{"look":"Grayscale","strength":70}}"#,
        r#"{"id":"page.set_look_many","params":{"pages":[1,2],"look":"Grayscale","strength":70}}"#,
        r#"{"id":"page.rotate","params":{"page":1,"quarter_turns":-1}}"#,
        r#"{"id":"page.rotate_many","params":{"pages":[1,2],"quarter_turns":1}}"#,
        r#"{"id":"page.add","params":{"page":1}}"#,
        r#"{"id":"page.mark_reviewed","params":{"page":1}}"#,
        r#"{"id":"page.delete","params":{"page":1}}"#,
        r#"{"id":"page.delete_many","params":{"pages":[1,2]}}"#,
        r#"{"id":"page.move","params":{"from":0,"to":1}}"#,
        r#"{"id":"page.edit","params":{"page":1}}"#,
        r#"{"id":"page.open_crop","params":{"page":1}}"#,
        r#"{"id":"reference.label","params":{"page":1,"folder":"/definitely/not/here","category":"wood"}}"#,
        r#"{"id":"export.pdf","params":{"path":"a.pdf","options":{"page_size":"Auto","quality":"Best","searchable":true,"languages":[]}}}"#,
        r#"{"id":"export.cancel"}"#,
        r#"{"id":"session.new"}"#,
        r#"{"id":"session.restore"}"#,
        r#"{"id":"edit.undo"}"#,
        r#"{"id":"edit.redo"}"#,
    ];
    assert_eq!(samples.len(), Command::IDS.len());
    let dir = TempDir::new("ids");
    let mut engine = open(&dir, services(desk_quad()));
    for id in Command::IDS {
        let sample = samples.iter().find(|s| s.contains(&format!("\"{id}\""))).unwrap_or_else(|| panic!("no sample for {id}"));
        let command = Command::from_json(sample).unwrap_or_else(|e| panic!("{id}: {e}"));
        let back = serde_json::to_value(&command).unwrap();
        assert_eq!(back["id"], *id);
        // On an empty session each answers, with an error or with nothing to do; none panics.
        let _ = engine.dispatch(command);
        engine.poll();
    }
}
