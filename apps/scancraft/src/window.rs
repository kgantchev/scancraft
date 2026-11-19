//! The window's memory: where it was and how big, kept between launches (specification §4.1).
//!
//! The record is `window.json` beside `config.toml`, written by `craft-window`. The window opens
//! where it was, at the size it was, maximised if it was; and when the screens have changed since,
//! `craft-window` keeps it on one.
//!
//! eframe has no way to list the monitors before the window exists, so the window opens with the
//! saved geometry as it was saved, at least the smallest size the interface works at. Everything
//! that decides anything is here as plain functions of numbers, so it is tested without a window.

use std::time::{Duration, Instant};

use craft_window::restore::{Defaults, MAIN, Monitor, Placement, Store, place};

/// The size of the very first window, in points.
pub const DEFAULT_SIZE: [f32; 2] = [1200.0, 800.0];
/// The smallest window the interface works at, in points.
pub const MIN_SIZE: [f32; 2] = [860.0, 600.0];
/// How long the window must stay as it is before the record is written: a drag of the window's
/// edge makes a change on every frame, and one write at the end is enough.
const QUIET: Duration = Duration::from_millis(600);

/// What the window looks like, in the numbers the memory keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    /// The size inside the frame, in points.
    pub inner_size: [f32; 2],
    /// The top-left corner of the frame, in points, when the system says.
    pub outer_position: Option<[f32; 2]>,
    /// Physical pixels to a point on the screen the window is on.
    pub scale: f32,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// What egui knows of the window, as [`Seen`]; `None` before the window has a size.
pub fn seen(info: &egui::ViewportInfo) -> Option<Seen> {
    let inner = info.inner_rect?;
    let scale = info.native_pixels_per_point.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    Some(Seen {
        inner_size: [inner.width(), inner.height()],
        outer_position: info.outer_rect.map(|r| [r.min.x, r.min.y]),
        scale,
        maximized: info.maximized.unwrap_or(false),
        fullscreen: info.fullscreen.unwrap_or(false),
    })
}

/// The record for a window that looks like `seen`. A maximised or full-screen window keeps the
/// size and place of the window it was before, so un-maximising goes back to them.
pub fn placement(seen: &Seen, before: Option<&Placement>) -> Placement {
    let mut record = Placement::new(seen.inner_size, seen.scale);
    record.maximized = seen.maximized;
    record.fullscreen = seen.fullscreen;
    record.position_px = seen.outer_position.map(|p| [(p[0] * seen.scale).round() as i32, (p[1] * seen.scale).round() as i32]);
    if (seen.maximized || seen.fullscreen)
        && let Some(before) = before
    {
        record.normal = before.normal;
        record.position_px = before.position_px;
    } else if let Some(position) = record.position_px {
        record.normal.pos = [position[0] as f32 / seen.scale, position[1] as f32 / seen.scale];
    }
    record
}

/// The defaults the window opens with when nothing is saved.
fn defaults() -> Defaults {
    Defaults::new(DEFAULT_SIZE, MIN_SIZE)
}

/// What the window opens with: its size, its position when it is known, and whether it is maximised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opening {
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}

/// Where the window opens, given what was saved and the monitors there are (none, when the platform
/// cannot say before the window exists: the saved geometry is then kept as it is).
pub fn opening(saved: Option<&Placement>, monitors: &[Monitor]) -> Opening {
    let placed = place(saved, monitors, None, &defaults());
    let scale = if placed.scale.is_finite() && placed.scale > 0.0 { placed.scale } else { 1.0 };
    Opening {
        size: placed.normal.size,
        position: placed.position_px.map(|p| [p[0] as f32 / scale, p[1] as f32 / scale]),
        maximized: placed.maximized,
    }
}

/// The window's memory while the app runs.
pub struct Memory {
    store: Store,
    /// The last record, so a frame that changed nothing writes nothing.
    last: Option<Placement>,
    /// When the window last changed.
    changed: Option<Instant>,
}

impl Memory {
    /// The memory kept in `window.json` beside the app's `config.toml`.
    pub fn open() -> Memory {
        Memory::with_store(Store::open("scancraft"))
    }

    /// A memory over `store`: a file of the test's own, or none.
    pub fn with_store(store: Store) -> Memory {
        let last = store.get(MAIN);
        Memory { store, last, changed: None }
    }

    /// The viewport the window opens with.
    pub fn viewport(&self) -> egui::ViewportBuilder {
        let opening = opening(self.last.as_ref(), &[]);
        let mut viewport = egui::ViewportBuilder::default().with_inner_size(opening.size).with_min_inner_size(MIN_SIZE);
        if let Some(position) = opening.position {
            viewport = viewport.with_position(position);
        }
        if opening.maximized {
            viewport = viewport.with_maximized(true);
        }
        viewport
    }

    /// Notes what the window looks like now, once a frame, and writes the record when it has
    /// stayed as it is for a moment.
    pub fn track(&mut self, ctx: &egui::Context) {
        let seen = ctx.input(|i| seen(i.viewport()));
        self.note(seen.as_ref(), Instant::now());
        if self.store.is_dirty() {
            // Come back when the quiet is over, though nothing else asks for a frame.
            ctx.request_repaint_after(QUIET);
        }
    }

    /// The part of [`Memory::track`] that needs no egui: `seen` at the time `now`.
    pub fn note(&mut self, seen: Option<&Seen>, now: Instant) {
        if let Some(seen) = seen {
            let record = placement(seen, self.last.as_ref());
            if self.last.as_ref() != Some(&record) {
                self.store.set(MAIN, &record);
                self.last = Some(record);
                self.changed = Some(now);
            }
        }
        if self.store.is_dirty() && self.changed.is_none_or(|at| now.saturating_duration_since(at) >= QUIET) {
            self.save();
        }
    }

    /// Writes the record now (when the app quits). A record that cannot be written is not worth
    /// refusing to quit for: the window opens at its default next time.
    pub fn save(&mut self) {
        if let Err(e) = self.store.save() {
            eprintln!("ScanCraft couldn't remember the window: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_window::restore::MonitorKey;

    fn seen(size: [f32; 2], position: Option<[f32; 2]>) -> Seen {
        Seen { inner_size: size, outer_position: position, scale: 2.0, maximized: false, fullscreen: false }
    }

    #[test]
    fn the_first_launch_opens_at_the_default_size_and_leaves_the_position_to_the_system() {
        let opening = opening(None, &[]);
        assert_eq!(opening, Opening { size: DEFAULT_SIZE, position: None, maximized: false });
    }

    #[test]
    fn the_window_opens_where_and_how_big_it_was() {
        // Seen at 1000 × 700 points, its corner at (300, 120) points on a 2× screen.
        let record = placement(&seen([1000.0, 700.0], Some([300.0, 120.0])), None);
        let opening = opening(Some(&record), &[]);
        assert_eq!(opening.size, [1000.0, 700.0]);
        assert_eq!(opening.position, Some([300.0, 120.0]));
        assert!(!opening.maximized);
    }

    #[test]
    fn a_saved_window_is_never_smaller_than_the_interface_works_at() {
        let record = placement(&seen([300.0, 200.0], Some([10.0, 10.0])), None);
        let opening = opening(Some(&record), &[]);
        assert!(opening.size[0] >= MIN_SIZE[0] && opening.size[1] >= MIN_SIZE[1], "{opening:?}");
    }

    #[test]
    fn a_maximised_window_comes_back_maximised_and_unmaximising_goes_to_its_old_size() {
        let normal = placement(&seen([1000.0, 700.0], Some([300.0, 120.0])), None);
        let mut big = seen([2560.0, 1400.0], Some([0.0, 0.0]));
        big.maximized = true;
        let maximised = placement(&big, Some(&normal));
        assert!(maximised.maximized);
        assert_eq!(maximised.normal, normal.normal, "the size before maximising was lost");
        let opening = opening(Some(&maximised), &[]);
        assert!(opening.maximized);
        assert_eq!(opening.size, [1000.0, 700.0]);
    }

    #[test]
    fn a_window_that_is_off_every_screen_is_brought_back_when_the_screens_are_known() {
        // Saved on a monitor to the right that is not there now.
        let record = placement(&seen([1000.0, 700.0], Some([4000.0, 200.0])), None);
        let only = Monitor::new(MonitorKey::new("Built-in", [2880, 1800]), [0, 0], [2880, 1800], 2.0);
        let opening = opening(Some(&record), &[only]);
        let position = opening.position.expect("a position");
        assert!(position[0] < 1440.0 && position[0] >= 0.0, "the window stayed off the screen: {position:?}");
    }

    #[test]
    fn nonsense_in_the_file_opens_the_default_window() {
        let mut record = Placement::new([f32::NAN, 600.0], 2.0);
        record.position_px = Some([i32::MAX, i32::MIN]);
        let opening = opening(Some(&record), &[]);
        assert!(opening.size[0].is_finite() && opening.size[0] >= MIN_SIZE[0]);
    }

    #[test]
    fn the_record_is_written_after_the_window_has_been_still_for_a_moment() {
        let dir = std::env::temp_dir().join(format!("scancraft-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("window.json");
        let mut memory = Memory::with_store(Store::at(&path));
        let start = Instant::now();
        // The window is dragged: a change on every frame, and nothing is written yet.
        for step in 0..5u64 {
            memory.note(Some(&seen([1000.0, 700.0], Some([100.0 + step as f32, 100.0]))), start + Duration::from_millis(step * 16));
        }
        assert!(!path.exists(), "the file was written while the window was still moving");
        // Still, a moment later: written.
        memory.note(None, start + Duration::from_millis(64 + 700));
        let text = std::fs::read_to_string(&path).expect("the record is written");
        assert!(text.contains("window") && text.contains("1000"), "{text}");
        // A new launch reads it back.
        let again = Memory::with_store(Store::at(&path));
        let opening = opening(again.last.as_ref(), &[]);
        assert_eq!(opening.size, [1000.0, 700.0]);
        assert_eq!(opening.position, Some([104.0, 100.0]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_frame_that_changed_nothing_writes_nothing() {
        let mut memory = Memory::with_store(Store::in_memory());
        let now = Instant::now();
        let window = seen([1000.0, 700.0], Some([100.0, 100.0]));
        memory.note(Some(&window), now);
        let changed = memory.changed;
        memory.note(Some(&window), now + Duration::from_millis(100));
        assert_eq!(memory.changed, changed, "an unchanged window was taken for a change");
    }
}
