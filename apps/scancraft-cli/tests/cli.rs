//! The subcommands of §6.9 as a user runs them: the real binary, on WP0's fixtures.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scancraft-cli")).args(args).output().unwrap()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scancraft-cli-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn detect_reports_the_corners_of_the_flat_fixture_as_json() {
    let flat = fixture("flat.png");
    let out = run(&["detect", flat.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let records: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let record = &records[0];
    assert_eq!(record["id"], "flat");
    assert_eq!(record["found"], true);
    let truth: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture("flat.json")).unwrap()).unwrap();
    for i in 0..4 {
        let (x, y) = (record["corners"][i][0].as_f64().unwrap(), record["corners"][i][1].as_f64().unwrap());
        let (tx, ty) = (truth["corners"][i]["x"].as_f64().unwrap(), truth["corners"][i]["y"].as_f64().unwrap());
        assert!((x - tx).hypot(y - ty) < 2.0, "corner {i}: ({x}, {y}) against ({tx}, {ty})");
    }
}

#[test]
fn flatten_reports_the_decision() {
    let curved = fixture("curved.png");
    let out = run(&["flatten", curved.to_str().unwrap(), "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let records: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(records[0]["flattened"].is_boolean());
    assert!(records[0]["pages"][0]["decision"]["reason"].is_string() || records[0]["pages"][0]["decision"].is_null());
    // Off never flattens.
    let off = run(&["flatten", curved.to_str().unwrap(), "--flatten", "off", "--json"]);
    let records: serde_json::Value = serde_json::from_slice(&off.stdout).unwrap();
    assert_eq!(records[0]["flattened"], false);
}

#[test]
fn page_writes_a_png_of_the_page() {
    let dir = temp("page");
    let out_path = dir.join("page.png");
    let flat = fixture("flat.png");
    let out = run(&["page", flat.to_str().unwrap(), "--look", "bw", "--out", out_path.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&out_path).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    let (w, h) = image::image_dimensions(&out_path).unwrap();
    // An A4 page at 300 DPI, whichever way up.
    assert!(w.min(h) > 2000 && w.max(h) > 3000, "{w} x {h}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_writes_the_same_bytes_on_one_thread_and_on_eight() {
    let dir = temp("pdf");
    let flat = fixture("flat.png");
    let curved = fixture("curved.png");
    let mut outputs = Vec::new();
    for threads in ["1", "8"] {
        // The title is the file's name, so each run gets a folder of its own.
        std::fs::create_dir_all(dir.join(threads)).unwrap();
        let path = dir.join(threads).join("scan.pdf");
        let out =
            run(&["pdf", flat.to_str().unwrap(), curved.to_str().unwrap(), "--look", "gray", "--threads", threads, "--out", path.to_str().unwrap()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        outputs.push(bytes);
    }
    assert!(outputs[0] == outputs[1], "one thread and eight wrote different PDFs");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_command_lines_exit_with_64_and_never_panic() {
    let flat = fixture("flat.png");
    let flat = flat.to_str().unwrap();
    for args in [
        vec!["detect"],
        vec!["detect", flat, "--nonsense"],
        vec!["page", flat],
        vec!["page", flat, "--out", "x.png", "--look", "sepia"],
        vec!["page", flat, flat, "--out", "x.png"],
        vec!["pdf", flat],
        vec!["pdf", flat, "--out", "x.pdf", "--strength", "300"],
        vec!["pdf", flat, "--out", "x.pdf", "--threads", "-1"],
        vec!["flatten", flat, "--flatten", "maybe"],
        vec!["detect", flat, "--order", "up"],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(64), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn junk_files_fail_cleanly() {
    let dir = temp("junk");
    let junk = dir.join("junk.png");
    std::fs::write(&junk, b"this is not a picture").unwrap();
    let truncated = dir.join("cut.png");
    let bytes = std::fs::read(fixture("flat.png")).unwrap();
    std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
    for file in [junk, truncated, dir.join("missing.png")] {
        let f = file.to_str().unwrap();
        for args in [
            vec!["detect", f, "--json"],
            vec!["flatten", f],
            vec!["page", f, "--out", dir.join("o.png").to_str().unwrap()],
            vec!["pdf", f, "--out", dir.join("o.pdf").to_str().unwrap()],
        ] {
            let out = run(&args);
            assert_eq!(out.status.code(), Some(1), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}
