//! The tests of A15 (upright), from the specification: forty turned synthetic text pages, ten per
//! rotation, all come out upright; a page without text stays as it is. The Vision half of the rule is
//! tested with a fake voter, the fallback with the pages themselves.

mod common;

use common::{SHEET, rotate, sized_sheet, upright_input};
use scancraft_core::determinism::Xorshift64Star;
use scancraft_core::image::{Gray8, Plane};
use scancraft_core::traits::{ServiceError, UprightVote, UprightVoter};
use scancraft_enhance::upright::{Upright, UprightBy, upright};
use scancraft_synth::page::{Canvas, PAPER};

/// A voter that answers with what it was given.
struct Fixed(Result<UprightVote, ServiceError>);

impl UprightVoter for Fixed {
    fn vote(&self, _: &Gray8) -> Result<UprightVote, ServiceError> {
        self.0.clone()
    }
}

fn vote(score: [f32; 4], chars: [u32; 4]) -> Fixed {
    Fixed(Ok(UprightVote { score, chars }))
}

/// The page's 1024 px Grayscale picture, upright.
fn page(seed: u64, size_pt: f64) -> Gray8 {
    upright_input(&sized_sheet(seed, size_pt))
}

fn turns_needed(page_turned_by: u8) -> u8 {
    (4 - page_turned_by) % 4
}

#[test]
fn forty_turned_text_pages_all_come_out_upright() {
    let mut upright_pages = [0usize; 4];
    for seed in 1..=10u64 {
        let upright_page = page(seed, 10.0);
        for turned in 0..4u8 {
            let result = upright(&rotate(&upright_page, turned), None).unwrap();
            if result == (Upright { quarter_turns: turns_needed(turned), by: UprightBy::Profiles }) {
                upright_pages[usize::from(turned)] += 1;
            } else {
                eprintln!("seed {seed}, turned {turned}: {result:?}");
            }
        }
    }
    assert_eq!(upright_pages, [10; 4], "pages out of 10 that came out upright, by clockwise quarter turns 0, 1, 2, 3");
}

/// What a page turned `turned` quarter turns must come out as when it is judged right, and the one
/// answer that is no harm when the evidence is thin.
fn right_or_unsure(result: Upright, turned: u8) -> bool {
    result == Upright { quarter_turns: turns_needed(turned), by: UprightBy::Profiles }
        || result == Upright { quarter_turns: 0, by: UprightBy::Unchanged }
}

#[test]
fn small_text_is_turned_right_or_left_alone() {
    // 5 pt text on this sheet is about the size 10 pt text has on an A4 sheet at 1024 px. Its lines
    // blur into one another and the difference between the ascenders and the descenders is faint,
    // so the fallback is often unsure; it must never be wrong.
    let mut turned_right = 0;
    for seed in 11..=20u64 {
        let upright_page = page(seed, 5.0);
        for turned in 0..4u8 {
            let result = upright(&rotate(&upright_page, turned), None).unwrap();
            assert!(right_or_unsure(result, turned), "seed {seed} turned {turned}: {result:?}");
            turned_right += usize::from(result.by == UprightBy::Profiles);
        }
    }
    eprintln!("{turned_right} of 40 small-text pages were turned upright by the profiles; the others were left as they were");
}

#[test]
fn capital_letters_give_no_evidence_so_the_page_is_left_alone() {
    // A page of words in capitals has no ascenders and no descenders.
    let mut c = Canvas::new(SHEET, PAPER).unwrap();
    let words = ["TOTAL", "NUMBER", "PAGE", "DATE", "INVOICE", "REPORT", "SCHEDULE", "LETTER"];
    let mut rng = Xorshift64Star::new(4);
    let mut baseline = 40.0;
    while baseline < SHEET.1 - 24.0 {
        let mut x = 20.0;
        loop {
            let word = words[rng.below(words.len() as u64) as usize];
            if x + 8.0 * word.len() as f64 > SHEET.0 - 20.0 {
                break;
            }
            x = c.text(x, baseline, 10.0, word, scancraft_synth::page::INK) + 8.0;
        }
        baseline += 14.0;
    }
    for turned in 0..4u8 {
        let result = upright(&rotate(&upright_input(&c), turned), None).unwrap();
        assert!(right_or_unsure(result, turned), "turned {turned}: {result:?}");
    }
}

#[test]
fn a_page_without_text_stays_as_it_is() {
    let unchanged = Upright { quarter_turns: 0, by: UprightBy::Unchanged };
    // Blank paper.
    let blank = upright_input(&Canvas::new(SHEET, PAPER).unwrap());
    assert_eq!(upright(&blank, None).unwrap(), unchanged);
    // A black box, and a picture: a block of ink has no lines in it.
    let mut c = Canvas::new(SHEET, PAPER).unwrap();
    c.fill_rect((40.0, 60.0, 200.0, 220.0), [0, 0, 0], true);
    for turned in 0..4u8 {
        assert_eq!(upright(&rotate(&upright_input(&c), turned), None).unwrap(), unchanged, "a box turned {turned}");
    }
    // Specks scattered over the page.
    let mut rng = Xorshift64Star::new(3);
    let mut specks = Canvas::new(SHEET, PAPER).unwrap();
    for _ in 0..300 {
        let (x, y) = (10.0 + rng.next_f64() * (SHEET.0 - 20.0), 10.0 + rng.next_f64() * (SHEET.1 - 20.0));
        specks.stroke((x, y), (x, y), 1.0, [20, 20, 20], true);
    }
    for turned in 0..4u8 {
        assert_eq!(upright(&rotate(&upright_input(&specks), turned), None).unwrap(), unchanged, "specks turned {turned}");
    }
    // Pages of a single colour, of any size.
    for (w, h, v) in [(1u32, 1u32, 0u8), (1, 1, 255), (7, 5, 128), (300, 200, 255), (300, 200, 0)] {
        assert_eq!(upright(&Plane::new(w, h, v).unwrap(), None).unwrap(), unchanged, "{w}×{h} of {v}");
    }
}

#[test]
fn a_decisive_vote_turns_the_page() {
    // Rotation 1 reads best: three times the score at 0°, 50 characters.
    let result = upright(&page(1, 10.0), Some(&vote([10.0, 30.0, 8.0, 5.0], [20, 50, 10, 8]))).unwrap();
    assert_eq!(result, Upright { quarter_turns: 1, by: UprightBy::Vision });
    // Exactly 1.5 times the score at 0° is enough.
    let result = upright(&page(1, 10.0), Some(&vote([10.0, 4.0, 15.0, 5.0], [30, 10, 40, 8]))).unwrap();
    assert_eq!(result, Upright { quarter_turns: 2, by: UprightBy::Vision });
    // Every rotation is a candidate, the last one too.
    let result = upright(&page(1, 10.0), Some(&vote([1.0, 1.0, 1.0, 9.0], [0, 0, 0, 90]))).unwrap();
    assert_eq!(result, Upright { quarter_turns: 3, by: UprightBy::Vision });
}

#[test]
fn an_unsure_vote_leaves_the_page_and_does_not_ask_the_fallback() {
    // The page is upside down, so the fallback would say 2, but Vision read 20 characters
    // somewhere and the best rotation is not 1.5 times as good as 0°: the page stays.
    let upside_down = rotate(&page(2, 10.0), 2);
    let not_enough = upright(&upside_down, Some(&vote([10.0, 4.0, 14.9, 5.0], [30, 10, 40, 8]))).unwrap();
    assert_eq!(not_enough, Upright { quarter_turns: 0, by: UprightBy::Vision });
    // The best rotation read only 19 characters.
    let few_chars = upright(&upside_down, Some(&vote([10.0, 4.0, 100.0, 5.0], [30, 10, 19, 8]))).unwrap();
    assert_eq!(few_chars, Upright { quarter_turns: 0, by: UprightBy::Vision });
    // 0° is the best: the page stays.
    let straight = upright(&upside_down, Some(&vote([30.0, 4.0, 5.0, 5.0], [30, 10, 40, 8]))).unwrap();
    assert_eq!(straight, Upright { quarter_turns: 0, by: UprightBy::Vision });
}

#[test]
fn without_a_usable_vote_the_fallback_decides() {
    let upside_down = rotate(&page(3, 10.0), 2);
    let by_profiles = Upright { quarter_turns: 2, by: UprightBy::Profiles };
    // No voter.
    assert_eq!(upright(&upside_down, None).unwrap(), by_profiles);
    // No rotation read 20 characters.
    assert_eq!(upright(&upside_down, Some(&vote([1.0, 2.0, 3.0, 0.5], [3, 5, 19, 0]))).unwrap(), by_profiles);
    // The service is unavailable, slow or failing.
    for error in [ServiceError::Unavailable, ServiceError::TimedOut(500), ServiceError::Failed("no".into())] {
        assert_eq!(upright(&upside_down, Some(&Fixed(Err(error.clone())))).unwrap(), by_profiles, "{error:?}");
    }
}

#[test]
fn the_fallback_needs_a_margin_between_rows_and_columns() {
    // Square marks in a grid have rows and columns alike: neither profile stands out.
    let mut grid = vec![255u8; 256 * 256];
    for gy in 0..8 {
        for gx in 0..8 {
            for y in 0..14 {
                for x in 0..14 {
                    grid[(gy * 32 + 8 + y) * 256 + gx * 32 + 8 + x] = 0;
                }
            }
        }
    }
    let result = upright(&Plane::from_vec(256, 256, grid).unwrap(), None).unwrap();
    assert_eq!(result, Upright { quarter_turns: 0, by: UprightBy::Unchanged });
}
