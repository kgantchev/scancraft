//! The app's engine with Apple's Vision services, end to end on the drawn fixtures: import, find
//! the page, crop, add it to the tray and write the PDF. macOS only, as Vision is.
#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use scancraft_core::command::Command;
use scancraft_core::engine_api::{EngineApi, Screen};
use scancraft_engine::{Engine, EngineOptions, Services};

fn services() -> Services {
    Services {
        detector: Some(Arc::new(scancraft_apple::VisionPageDetector)),
        recognizer: Some(Arc::new(scancraft_apple::VisionRecognizer)),
        upright: Some(Arc::new(scancraft_apple::VisionUprightVoter)),
    }
}

fn settle(engine: &mut Engine) {
    let end = Instant::now() + Duration::from_secs(300);
    loop {
        engine.poll();
        if !engine.view().busy {
            return;
        }
        assert!(Instant::now() < end, "the engine did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fixture(name: &str) -> String {
    format!("{}/../../fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn truth(name: &str) -> Vec<(f64, f64)> {
    let text = std::fs::read_to_string(fixture(name)).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["corners"].as_array().unwrap().iter().map(|c| (c["x"].as_f64().unwrap(), c["y"].as_f64().unwrap())).collect()
}

#[test]
fn a_photo_goes_from_import_to_pdf_with_vision_services() {
    let dir = std::env::temp_dir().join(format!("scancraft-app-test-{}", std::process::id()));
    let mut options = EngineOptions::new(dir.join("session"), services());
    options.settings = craft_settings::Location::Memory;
    options.sessions_root = Some(dir.clone());
    let mut engine = Engine::open(options).unwrap();
    engine.dispatch(Command::ImportAdd { paths: vec![fixture("flat.png")] }).unwrap();
    settle(&mut engine);

    let view = engine.view();
    let crop = view.crop.expect("Screen 2 opens on the photo");
    let outline = crop.outlines.first().expect("one page outline");
    // Found, by Vision or by the classical detector behind it: a corner within 5% of the long side.
    let tolerance = 0.05 * f64::from(crop.photo_size.0.max(crop.photo_size.1));
    for (found, want) in outline.corners.0.iter().zip(truth("flat.json")) {
        let miss = ((found.x - want.0).powi(2) + (found.y - want.1).powi(2)).sqrt();
        assert!(miss < tolerance, "a corner is {miss:.1} px from the truth ({found:?} against {want:?})");
    }

    let Screen::Crop(photo) = engine.view().screen else { panic!("not on Screen 2") };
    engine.dispatch(Command::CropApply { photo }).unwrap();
    settle(&mut engine);
    let Screen::Enhance(page) = engine.view().screen else { panic!("not on Screen 3") };
    engine.dispatch(Command::PageAdd { page }).unwrap();
    settle(&mut engine);
    assert_eq!(engine.view().tray.len(), 1);

    let out = dir.join("out.pdf");
    let options = scancraft_core::recipe::ExportOptions::default();
    engine.dispatch(Command::ExportPdf { path: out.to_string_lossy().into_owned(), options }).unwrap();
    settle(&mut engine);
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"%PDF-"), "the export is a PDF");
    let _ = std::fs::remove_dir_all(&dir);
}
