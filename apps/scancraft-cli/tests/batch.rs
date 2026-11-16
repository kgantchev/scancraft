//! The `synth` and `eval` subcommands, run as the program: what they write, and that junk
//! arguments exit with a message and never panic (the fuzz gate of §10.4).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scancraft-cli")).args(args).output().unwrap()
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scancraft-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn read_all(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .map(|e| (e.file_name().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap()))
        .collect();
    files.sort();
    files
}

#[test]
fn the_synth_command_writes_a_set_and_the_same_seed_writes_the_same_bytes() {
    let (a, b, c) = (temp("synth-a"), temp("synth-b"), temp("synth-c"));
    let out = run(&["synth", "--count", "2", "--seed", "7", "--out", a.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("wrote 2 flat scenes"));
    let files = read_all(&a);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "scene_00000.ink.png",
            "scene_00000.json",
            "scene_00000.png",
            "scene_00000.txt",
            "scene_00001.ink.png",
            "scene_00001.json",
            "scene_00001.png",
            "scene_00001.txt"
        ]
    );
    assert!(run(&["synth", "--count", "2", "--seed", "7", "--out", b.to_str().unwrap()]).status.success());
    assert_eq!(files, read_all(&b));
    assert!(run(&["synth", "--curved", "--count", "1", "--seed", "7", "--out", c.to_str().unwrap()]).status.success());
    let json = std::fs::read_to_string(c.join("scene_00000.json")).unwrap();
    assert!(json.contains("Bent") && json.contains("page_grid"));
    for d in [a, b, c] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn the_eval_command_writes_a_report() {
    let (set, report) = (temp("eval-set"), temp("eval-report").with_extension("json"));
    assert!(run(&["synth", "--curved", "--count", "2", "--seed", "3", "--out", set.to_str().unwrap()]).status.success());
    // Without the pipeline's records the report says nothing was scored.
    let out = run(&["eval", "--set", "synthetic", "--root", set.to_str().unwrap(), "--report", report.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(r["set"], "synthetic");
    assert_eq!(r["photos"], 2);
    assert_eq!(r["scored"], 0);
    assert!(r["metrics"].get("jaccard_mean").is_some());
    // With records that are the truth, the page is found perfectly.
    let truth: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(set.join("scene_00000.json")).unwrap()).unwrap();
    let corners: Vec<[f64; 2]> = truth["corners"].as_array().unwrap().iter().map(|p| [p["x"].as_f64().unwrap(), p["y"].as_f64().unwrap()]).collect();
    let found = set.join("found.json");
    std::fs::write(&found, serde_json::json!([{"id": "scene_00000", "corners": corners, "size": truth["size"]}]).to_string()).unwrap();
    let out = run(&[
        "eval",
        "--set",
        "synthetic",
        "--root",
        set.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
        "--found",
        found.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(r["scored"], 1);
    assert!(r["per_photo"][0]["jaccard"].as_f64().unwrap() > 0.999);
    assert!(r["metrics"]["not_found_rate"].as_f64().unwrap() > 0.4);
    let _ = std::fs::remove_dir_all(set);
    let _ = std::fs::remove_file(report);
}

#[test]
fn the_cli_rejects_junk_parameters() {
    let out = temp("junk");
    let o = out.to_str().unwrap();
    let cases: Vec<Vec<&str>> = vec![
        vec!["synth"],
        vec!["synth", "--count", "5", "--seed", "1"],
        vec!["synth", "--seed", "1", "--out", o],
        vec!["synth", "--count", "5", "--out", o],
        vec!["synth", "--count", "x", "--seed", "1", "--out", o],
        vec!["synth", "--count", "-3", "--seed", "1", "--out", o],
        vec!["synth", "--count", "0", "--seed", "1", "--out", o],
        vec!["synth", "--count", "99999999", "--seed", "1", "--out", o],
        vec!["synth", "--count", "5", "--seed", "-1", "--out", o],
        vec!["synth", "--count", "5", "--seed", "1", "--out"],
        vec!["synth", "--count", "5", "--seed", "1", "--out", o, "--bogus"],
        vec!["synth", "--count", "5", "--count", "6", "--seed", "1", "--out", o],
        vec!["synth", "--curved", "--curved", "--count", "5", "--seed", "1", "--out", o],
        vec!["synth", "count", "5"],
        vec!["eval"],
        vec!["eval", "--set", "smartdoc2015"],
        vec!["eval", "--set", "nonsense", "--root", o, "--report", o],
        vec!["eval", "--root", o, "--report", o],
        vec!["eval", "--set", "uvdoc", "--root", o],
        vec!["eval", "--set", "uvdoc", "--root"],
        vec![""],
        vec!["--curved"],
    ];
    for args in cases {
        let result = run(&args);
        assert_eq!(
            result.status.code(),
            Some(64),
            "{args:?}: {} / {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!result.stderr.is_empty(), "{args:?} printed no message");
        assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"), "{args:?}");
    }
    // Nothing was written by any of them.
    assert!(!out.exists());
}

#[test]
fn a_job_that_cannot_run_exits_one_with_a_message() {
    let missing = temp("nowhere");
    let report = temp("report").with_extension("json");
    for set in ["smartdoc2015", "uvdoc", "reference", "synthetic"] {
        let out = run(&["eval", "--set", set, "--root", missing.to_str().unwrap(), "--report", report.to_str().unwrap()]);
        assert_eq!(out.status.code(), Some(1), "{set}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("scancraft-cli eval"));
    }
    // A file where the output folder should be.
    let file = temp("a-file");
    std::fs::write(&file, b"x").unwrap();
    let out = run(&["synth", "--count", "1", "--seed", "1", "--out", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let _ = std::fs::remove_file(file);
    // The pipeline's subcommands fail the same way on a file that is not there (tests/cli.rs has the rest).
    assert_eq!(run(&["detect", "x.jpg"]).status.code(), Some(1));
    assert!(run(&["commands"]).status.success());
}
