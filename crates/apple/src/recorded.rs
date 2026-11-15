//! Tests that start from a recorded Vision result (build plan WP5: "recorded Vision results for
//! CI"). The two recordings in `tests/fixtures` are what Vision answered on a Mac; these tests
//! push them through the same conversion the app runs, so CI on any platform checks the
//! conversion without asking Vision again, and without depending on what a later macOS returns.
//!
//! `vision::tests::record_vision_results` writes the recordings.

use crate::parse::{NormQuad, VisionLine, corners_to_quad, parse_lines, vision_rect_to_page};
use crate::testpage::render;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn recording(name: &str) -> Value {
    read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name))
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn norm_quad(v: &Value) -> NormQuad {
    let point = |i: usize| (number(&v[i][0]), number(&v[i][1]));
    [point(0), point(1), point(2), point(3)]
}

#[test]
fn recorded_rectangle_gives_the_page_quad() {
    let recorded = recording("vision_rectangle.json");
    let (w, h) = (recorded["size"][0].as_u64().unwrap() as u32, recorded["size"][1].as_u64().unwrap() as u32);
    let quad = corners_to_quad(norm_quad(&recorded["corners"]), w, h).unwrap();

    // The recording was made from fixtures/flat.png, whose truth file has the page's corners.
    let truth = read(&fixtures_dir().join("flat.json"));
    let truth_size = (truth["size"][0].as_u64().unwrap() as u32, truth["size"][1].as_u64().unwrap() as u32);
    assert_eq!((w, h), truth_size, "the recording and the truth file describe the same image");
    for (i, got) in quad.0.iter().enumerate() {
        let (x, y) = (number(&truth["corners"][i]["x"]), number(&truth["corners"][i]["y"]));
        // Vision's own estimate: within 7 px on the Mac the recording came from.
        assert!((got.x - x).abs() < 10.0 && (got.y - y).abs() < 10.0, "corner {i}: {got:?} vs ({x}, {y})");
    }
}

#[test]
fn recorded_text_gives_the_recognised_words() {
    let recorded = recording("vision_text.json");
    let page = &recorded["page"];
    let (w, h) = (page["width"].as_u64().unwrap() as u32, page["height"].as_u64().unwrap() as u32);
    let truth = render(w, h, page["cell"].as_u64().unwrap() as u32, page["seed"].as_u64().unwrap());

    let observations = recorded["observations"].as_array().unwrap();
    assert!(observations.len() >= 5, "the recording holds several lines");
    let lines: Vec<VisionLine> = observations
        .iter()
        .map(|o| VisionLine {
            text: o["text"].as_str().unwrap().to_owned(),
            confidence: number(&o["confidence"]) as f32,
            line_box: [number(&o["line_box"][0]), number(&o["line_box"][1]), number(&o["line_box"][2]), number(&o["line_box"][3])],
            word_quads: o["word_boxes"].as_array().unwrap().iter().map(|q| if q.is_null() { None } else { Some(norm_quad(q)) }).collect(),
        })
        .collect();
    let parsed = parse_lines(&lines, w, h);

    // One parsed line per recorded observation, in the order Vision gave them.
    assert_eq!(parsed.len(), observations.len());
    for ((line, observation), parsed_line) in lines.iter().zip(observations).zip(&parsed) {
        let truth_line = &truth.lines[observation["truth_line"].as_u64().unwrap() as usize];
        // The same words as were drawn.
        let words: Vec<String> = parsed_line.words.iter().map(|w| w.text.to_uppercase()).collect();
        let drawn: Vec<&str> = truth_line.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(words, drawn);
        let line_rect = vision_rect_to_page(line.line_box, w, h);
        for (word, truth_word) in parsed_line.words.iter().zip(truth_line) {
            assert_eq!((word.confidence, word.source.as_str()), (line.confidence, "vision"));
            // Each box is centred on the drawn word: the y flip and the pixel scale are right.
            let centre = ((word.rect[0] + word.rect[2]) / 2.0, (word.rect[1] + word.rect[3]) / 2.0);
            let truth_centre = ((truth_word.rect[0] + truth_word.rect[2]) / 2.0, (truth_word.rect[1] + truth_word.rect[3]) / 2.0);
            assert!(
                (centre.0 - truth_centre.0).abs() < 6.0 && (centre.1 - truth_centre.1).abs() < 6.0,
                "{} at {:?} vs {:?}",
                word.text,
                word.rect,
                truth_word.rect
            );
            // Vision's box is a little larger than the letters, never smaller.
            assert!(
                word.rect[0] <= truth_word.rect[0] + 1.0 && word.rect[2] >= truth_word.rect[2] - 1.0,
                "{} at {:?} vs {:?}",
                word.text,
                word.rect,
                truth_word.rect
            );
            assert!(
                word.rect[1] <= truth_word.rect[1] + 1.0 && word.rect[3] >= truth_word.rect[3] - 1.0,
                "{} at {:?} vs {:?}",
                word.text,
                word.rect,
                truth_word.rect
            );
            // The line's own box, which the proportional fallback splits, holds the word.
            assert!(line_rect[0] <= truth_word.rect[0] + 1.0 && line_rect[2] >= truth_word.rect[2] - 1.0, "{line_rect:?} vs {:?}", truth_word.rect);
            assert!(line_rect[1] <= truth_word.rect[1] + 1.0 && line_rect[3] >= truth_word.rect[3] - 1.0, "{line_rect:?} vs {:?}", truth_word.rect);
        }
    }
}

#[test]
fn recorded_text_without_word_boxes_falls_back_to_the_line_box() {
    let recorded = recording("vision_text.json");
    let page = &recorded["page"];
    let (w, h) = (page["width"].as_u64().unwrap() as u32, page["height"].as_u64().unwrap() as u32);
    let o = &recorded["observations"][0];
    let text = o["text"].as_str().unwrap();
    let line = VisionLine {
        text: text.to_owned(),
        confidence: number(&o["confidence"]) as f32,
        line_box: [number(&o["line_box"][0]), number(&o["line_box"][1]), number(&o["line_box"][2]), number(&o["line_box"][3])],
        // Vision gave no box for any word.
        word_quads: Vec::new(),
    };
    let parsed = parse_lines(std::slice::from_ref(&line), w, h);
    let words = &parsed[0].words;
    assert_eq!(words.len(), text.split(' ').count());
    // The shares tile the line's box from left to right, each with the line's full height.
    let line_rect = vision_rect_to_page(line.line_box, w, h);
    assert!((words[0].rect[0] - line_rect[0]).abs() < 0.01 && (words[words.len() - 1].rect[2] - line_rect[2]).abs() < 0.01);
    for pair in words.windows(2) {
        assert!((pair[0].rect[2] - pair[1].rect[0]).abs() < 0.01);
    }
    assert!(words.iter().all(|word| word.rect[1] == line_rect[1] && word.rect[3] == line_rect[3]));
}
