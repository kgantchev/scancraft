//! The system's Reduce Motion and Increase Contrast, read for the interface (§4.9).
//!
//! macOS keeps them in the user's preferences. They are read with the system's own `defaults`
//! tool, one process at a time and never through a shell, on a thread of their own, at startup and
//! again whenever the window comes back to the front, which is when someone who has just been to
//! System Settings to change one will look. Where there is no such tool both are off.
//!
//! Which key holds each setting changed between versions of macOS, so both the current key and
//! the older one are read, and a setting is on if either says so.

use std::sync::mpsc::{self, Receiver};

use scancraft_ui_egui::Accessibility;

/// Where macOS keeps Reduce Motion: the domain and the key, the current one first.
const REDUCE_MOTION: [(&str, &str); 2] = [("com.apple.Accessibility", "ReduceMotionEnabled"), ("com.apple.universalaccess", "reduceMotion")];
/// Where macOS keeps Increase Contrast.
const INCREASE_CONTRAST: [(&str, &str); 2] =
    [("com.apple.Accessibility", "EnhancedBackgroundContrastEnabled"), ("com.apple.universalaccess", "increaseContrast")];

/// What `defaults read` printed, as on or off; `None` when it is neither (the key is not there,
/// and `defaults` says so on its error output and prints nothing here).
pub fn parse_flag(output: &str) -> Option<bool> {
    match output.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// A setting is on if any of the places it may be kept says so.
pub fn any_on(values: impl IntoIterator<Item = Option<bool>>) -> bool {
    values.into_iter().any(|v| v == Some(true))
}

/// One key, read with `defaults read <domain> <key>`.
#[cfg(target_os = "macos")]
fn read_flag(domain: &str, key: &str) -> Option<bool> {
    let output = std::process::Command::new("defaults").args(["read", domain, key]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_flag(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn read_flag(_domain: &str, _key: &str) -> Option<bool> {
    None
}

/// Both settings, as the system has them now.
pub fn read() -> Accessibility {
    Accessibility {
        reduce_motion: any_on(REDUCE_MOTION.iter().map(|(domain, key)| read_flag(domain, key))),
        increase_contrast: any_on(INCREASE_CONTRAST.iter().map(|(domain, key)| read_flag(domain, key))),
    }
}

/// Reads the settings now and again each time the window comes to the front, without making a
/// frame wait for the answer.
pub struct Refresh {
    was_focused: bool,
    pending: Option<Receiver<Accessibility>>,
}

impl Refresh {
    /// A reader that reads at once, on the first [`Refresh::step`].
    pub fn new() -> Refresh {
        Refresh { was_focused: false, pending: None }
    }

    /// Once a frame: starts a read when the window has just come to the front, and returns what a
    /// read that has finished found. `wake` asks for a frame when an answer is ready.
    pub fn step(
        &mut self,
        focused: bool,
        wake: impl Fn() + Send + 'static,
        read: impl FnOnce() -> Accessibility + Send + 'static,
    ) -> Option<Accessibility> {
        if focused && !self.was_focused && self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(read());
                wake();
            });
        }
        self.was_focused = focused;
        let found = self.pending.as_ref().and_then(|rx| rx.try_recv().ok());
        if found.is_some() {
            self.pending = None;
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn defaults_output_is_read_as_on_or_off_and_anything_else_as_neither() {
        assert_eq!(parse_flag("1\n"), Some(true));
        assert_eq!(parse_flag(" 0 \n"), Some(false));
        assert_eq!(parse_flag("true"), Some(true));
        assert_eq!(parse_flag("No"), Some(false));
        for junk in ["", "\n", "2", "The domain/default pair of (x, y) does not exist", "{ a = 1; }"] {
            assert_eq!(parse_flag(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn a_setting_is_on_if_any_place_it_is_kept_says_so() {
        assert!(any_on([Some(false), Some(true)]));
        assert!(any_on([None, Some(true)]));
        assert!(!any_on([Some(false), None]));
        assert!(!any_on([]));
    }

    /// Steps `refresh` until it answers, or fails.
    fn answer(refresh: &mut Refresh, focused: bool, read: impl FnOnce() -> Accessibility + Send + 'static) -> Accessibility {
        let mut read = Some(read);
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let found = match read.take() {
                Some(read) => refresh.step(focused, || {}, read),
                None => refresh.step(focused, || {}, Accessibility::default),
            };
            if let Some(found) = found {
                return found;
            }
            assert!(Instant::now() < end, "no answer");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_settings_are_read_when_the_window_first_comes_to_the_front_and_not_again_while_it_stays() {
        let mut refresh = Refresh::new();
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = |reads: &Arc<AtomicUsize>| {
            let reads = reads.clone();
            move || {
                reads.fetch_add(1, Ordering::SeqCst);
                Accessibility { reduce_motion: true, increase_contrast: false }
            }
        };
        // Not in front yet: nothing is read.
        assert_eq!(refresh.step(false, || {}, counted(&reads)), None);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        // In front: it is.
        let found = answer(&mut refresh, true, counted(&reads));
        assert_eq!(found, Accessibility { reduce_motion: true, increase_contrast: false });
        // Staying in front reads nothing more.
        for _ in 0..20 {
            assert_eq!(refresh.step(true, || {}, counted(&reads)), None);
        }
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        // Away and back: read again.
        assert_eq!(refresh.step(false, || {}, counted(&reads)), None);
        let again = answer(&mut refresh, true, counted(&reads));
        assert!(again.reduce_motion);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_frame_is_woken_when_an_answer_is_ready() {
        let mut refresh = Refresh::new();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        refresh.step(
            true,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
            Accessibility::default,
        );
        let end = Instant::now() + Duration::from_secs(5);
        while woken.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < end, "the frame was never woken");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
