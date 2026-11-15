//! A15: upright, Vision's vote first, then the ink-profile fallback (specification A15, §5.3).
//!
//! The fallback stands in for PdfCraft's `detect_orientation`, which compares the ink profiles of
//! the rows and of the columns. This is a clean-room implementation from the specification's
//! description, with the same margin of 1.3.
//!
//! - **Which way the lines run.** A page of horizontal text has rows that alternate between a line
//!   of ink and the gap above the next one, and columns that are the same all the way across. The
//!   contrast of a profile, its standard deviation over its mean across the stretch that holds ink,
//!   tells the two apart: if the rows are at least 1.3 times as contrasty as the columns, the lines
//!   are horizontal, and the other way round.
//! - **Which end is up.** The profiles cannot tell 0° from 180°. Letters with an ascender or a
//!   capital's height (b, d, f, h, k, l, t, every capital and digit) are several times as common as
//!   letters with a descender (g, j, p, q, y), so the ink of a line of text is heavier above its
//!   body than below it. The profile of the rows is folded at the line pitch, which the profile's
//!   autocorrelation gives, into the profile of one typical line, and the skewness of that profile
//!   says which side its tail is on: toward the top for an upright page, toward the bottom for one
//!   upside down. Folding makes lines that touch no harder to judge than lines that don't. The same
//!   holds along the columns of a sideways page.
//!
//! The decision does not depend on where a typeface draws its baseline, only on there being more
//! ink above it than below it. Where the evidence is thin the page is left as it is: a page turned
//! the wrong way is worse than one left alone.

use crate::EnhanceError;
use crate::helpers::percentile;
use scancraft_core::image::Gray8;
use scancraft_core::traits::{UprightVote, UprightVoter};

/// The margin between the row and the column profiles that the fallback needs (A15 step 4).
const PROFILE_MARGIN: f64 = 1.3;
/// The least contrast a profile needs to count as lines of text: a page that is one block of ink,
/// or a page of specks, has none.
const MIN_CONTRAST: f64 = 0.4;
/// The skewness of a typical line's profile that tells its top from its bottom. A page of English
/// text has one of about −0.5 the right way up; 0.1 is well clear of what chance gives.
const MIN_SKEW: f64 = 0.1;
/// The shortest line pitch looked for, in pixels, and the share of the profile's length the longest
/// may take: at least three lines are needed to see a pitch.
const MIN_PITCH: usize = 4;
/// How strongly a profile must repeat at its pitch for the lines to be trusted, out of 1.
const MIN_PERIODICITY: f64 = 0.2;
/// The characters a rotation must read before Vision's vote counts (A15 step 3).
const MIN_CHARS: u32 = 20;
/// How many times the score at 0° the best rotation must reach (A15 step 3).
const SCORE_MARGIN: f32 = 1.5;
/// The share of the paper's level a pixel must be darker than the paper by before it counts as ink.
const INK_FLOOR: f32 = 0.08;

/// Who decided the page's rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UprightBy {
    /// Vision's vote of A15 step 2.
    Vision,
    /// The ink profiles of the fallback.
    Profiles,
    /// Unsure: the page stays as it is.
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upright {
    /// Clockwise quarter turns to apply.
    pub quarter_turns: u8,
    pub by: UprightBy,
}

/// `page` is a 1024 px Grayscale version of the page (A15 step 1).
pub fn upright(page: &Gray8, voter: Option<&dyn UprightVoter>) -> Result<Upright, EnhanceError> {
    // Vision's vote first. A service that is missing, slow or failing is not an error of the page:
    // the fallback decides.
    if let Some(vote) = voter.and_then(|v| v.vote(page).ok())
        && let Some(decided) = decide(&vote)
    {
        return Ok(decided);
    }
    Ok(detect_orientation(page))
}

/// A15 steps 3 and 4 on Vision's vote: the page turns to the best rotation only if its score is at
/// least 1.5 times the score at 0° and it read at least 20 characters; otherwise it stays. `None`
/// when no rotation read 20 characters, which hands the page to the fallback.
fn decide(vote: &UprightVote) -> Option<Upright> {
    if !vote.chars.iter().any(|&c| c >= MIN_CHARS) {
        return None;
    }
    let mut best = 0usize;
    for (i, &score) in vote.score.iter().enumerate() {
        if score > vote.score.get(best).copied().unwrap_or(0.0) {
            best = i;
        }
    }
    let score = vote.score.get(best).copied().unwrap_or(0.0);
    let straight = vote.score.first().copied().unwrap_or(0.0);
    let chars = vote.chars.get(best).copied().unwrap_or(0);
    let turns = if best != 0 && score >= SCORE_MARGIN * straight && chars >= MIN_CHARS { best as u8 } else { 0 };
    Some(Upright { quarter_turns: turns, by: UprightBy::Vision })
}

/// The fallback: the ink profiles of the rows and of the columns.
fn detect_orientation(page: &Gray8) -> Upright {
    let unsure = Upright { quarter_turns: 0, by: UprightBy::Unchanged };
    let (w, h) = (page.width() as usize, page.height() as usize);
    if w == 0 || h == 0 {
        return unsure;
    }
    // The paper's level is the P90 of the page. The ink of a pixel is how far below the paper it
    // is, less a margin for the paper's own noise, so that a thin stroke that covers part of a
    // pixel counts for the part it covers.
    let lumas: Vec<f32> = page.data().iter().map(|&v| f32::from(v) / 255.0).collect();
    let paper = percentile(&lumas, 0.90).unwrap_or(1.0).max(1e-3);
    let mut rows = vec![0f64; h];
    let mut columns = vec![0f64; w];
    for (i, &v) in lumas.iter().enumerate() {
        let ink = f64::from(((paper - v) / paper - INK_FLOOR).max(0.0));
        if ink > 0.0 {
            if let Some(slot) = rows.get_mut(i / w) {
                *slot += ink;
            }
            if let Some(slot) = columns.get_mut(i % w) {
                *slot += ink;
            }
        }
    }
    let (row_contrast, column_contrast) = (contrast(&rows), contrast(&columns));
    let turns = if row_contrast >= MIN_CONTRAST && row_contrast >= PROFILE_MARGIN * column_contrast {
        // The lines are horizontal. The tail of a typical line is at the top if the page is upright.
        match line_skewness(&rows) {
            Some(s) if s < -MIN_SKEW => Some(0),
            Some(s) if s > MIN_SKEW => Some(2),
            _ => None,
        }
    } else if column_contrast >= MIN_CONTRAST && column_contrast >= PROFILE_MARGIN * row_contrast {
        // The lines are vertical. On a page turned a quarter turn clockwise the tops of the letters
        // point right, so the tail is on the right and three turns bring the page back; on a page
        // turned the other way the tail is on the left and one turn does.
        match line_skewness(&columns) {
            Some(s) if s > MIN_SKEW => Some(3),
            Some(s) if s < -MIN_SKEW => Some(1),
            _ => None,
        }
    } else {
        None
    };
    turns.map_or(unsure, |quarter_turns| Upright { quarter_turns, by: UprightBy::Profiles })
}

/// How much a profile varies, as its standard deviation over its mean, across the stretch from its
/// first sample with ink to its last.
fn contrast(profile: &[f64]) -> f64 {
    let (Some(first), Some(last)) = (profile.iter().position(|&v| v > 0.0), profile.iter().rposition(|&v| v > 0.0)) else {
        return 0.0;
    };
    let stretch: &[f64] = profile.get(first..=last).unwrap_or(&[]);
    let n = stretch.len() as f64;
    let mean = stretch.iter().sum::<f64>() / n.max(1.0);
    if mean <= 0.0 {
        return 0.0;
    }
    let variance = stretch.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt() / mean
}

/// The skewness of one typical line of a profile, or `None` when the profile does not repeat.
///
/// 1. The pitch is the lag at which the profile, with its mean taken off, best matches itself: the
///    first strong peak of its autocorrelation, so that two lines are not taken for one.
/// 2. The profile is folded at the pitch: sample `i` of the typical line is the sum of the samples
///    `i`, `i + pitch`, `i + 2 · pitch`, … of the profile.
/// 3. The typical line is cut at its emptiest stretch, the gap between two lines, so that it starts
///    at one line's gap and ends at the next one's.
/// 4. The skewness is the third central moment of the ink along the line over the cube of its
///    standard deviation: negative when the tail is toward the start of the line.
fn line_skewness(profile: &[f64]) -> Option<f64> {
    let (first, last) = (profile.iter().position(|&v| v > 0.0)?, profile.iter().rposition(|&v| v > 0.0)?);
    let stretch: Vec<f64> = profile.get(first..=last)?.to_vec();
    let pitch = line_pitch(&stretch)?;
    let mut folded = vec![0f64; pitch];
    for (i, v) in stretch.iter().take(stretch.len() / pitch * pitch).enumerate() {
        if let Some(slot) = folded.get_mut(i % pitch) {
            *slot += v;
        }
    }
    // The emptiest stretch of a fifth of the pitch, around the cycle.
    let gap = (pitch / 5).max(2);
    let window = |start: usize| (0..gap).map(|k| folded.get((start + k) % pitch).copied().unwrap_or(0.0)).sum::<f64>();
    let emptiest = (0..pitch).min_by(|&a, &b| window(a).total_cmp(&window(b)))?;
    let line: Vec<f64> = (0..pitch).map(|k| folded.get((emptiest + gap / 2 + k) % pitch).copied().unwrap_or(0.0)).collect();
    skewness(&line)
}

/// The pitch of the lines of a profile: the first lag from `MIN_PITCH` to a third of the profile's
/// length at which its autocorrelation has a local maximum of at least 85 % of the greatest, and at
/// least `MIN_PERIODICITY` of the profile's variance.
fn line_pitch(profile: &[f64]) -> Option<usize> {
    let n = profile.len();
    let mean = profile.iter().sum::<f64>() / n as f64;
    let centred: Vec<f64> = profile.iter().map(|v| v - mean).collect();
    let energy: f64 = centred.iter().map(|v| v * v).sum();
    if energy <= 0.0 || n < 3 * MIN_PITCH {
        return None;
    }
    let longest = n / 3;
    let correlation: Vec<f64> =
        (0..=longest + 1).map(|lag| centred.iter().zip(centred.iter().skip(lag)).map(|(a, b)| a * b).sum::<f64>() / energy).collect();
    let at = |lag: usize| correlation.get(lag).copied().unwrap_or(f64::NEG_INFINITY);
    let peaks: Vec<usize> = (MIN_PITCH..=longest).filter(|&lag| at(lag) > at(lag - 1) && at(lag) >= at(lag + 1)).collect();
    let best = peaks.iter().map(|&lag| at(lag)).fold(f64::NEG_INFINITY, f64::max);
    if best < MIN_PERIODICITY {
        return None;
    }
    peaks.into_iter().find(|&lag| at(lag) >= 0.85 * best)
}

/// The skewness of a distribution of mass over positions 0, 1, 2, …: `None` if it has no mass or no
/// spread.
fn skewness(mass: &[f64]) -> Option<f64> {
    let total: f64 = mass.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mean = mass.iter().enumerate().map(|(i, m)| i as f64 * m).sum::<f64>() / total;
    let variance = mass.iter().enumerate().map(|(i, m)| (i as f64 - mean).powi(2) * m).sum::<f64>() / total;
    if variance <= 1e-12 {
        return None;
    }
    let third = mass.iter().enumerate().map(|(i, m)| (i as f64 - mean).powi(3) * m).sum::<f64>() / total;
    Some(third / variance.powf(1.5))
}
