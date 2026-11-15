//! The undo stack (§6.3): one per session, up to 200 steps, and a whole drag counts as one step.

use scancraft_core::command::Command;
use scancraft_core::engine_api::Screen;
use scancraft_core::recipe::{Page, PageId, PhotoId, SessionFile};

pub const MAX_UNDO: usize = 200;
/// A step of the same kind on the same target within this many milliseconds joins the one before.
pub const COALESCE_MS: u64 = 500;

/// Everything undo restores: the session file, the pages not yet in the tray, and where the user was.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub file: SessionFile,
    pub pending: Vec<Page>,
    pub screen: Screen,
    pub current_photo: Option<PhotoId>,
    pub current_page: Option<PageId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UndoStep {
    pub command: Command,
    pub before: Snapshot,
    pub after: Snapshot,
    /// The engine's clock when the step was made, for the coalescing window.
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
}

/// What a command acts on, for deciding whether two steps are one drag.
fn target(command: &Command) -> Option<(u8, u64)> {
    Some(match command {
        Command::CropSetCorner { page, .. } => (0, page.0),
        Command::CropMoveSide { page, side, .. } => (1, page.0.wrapping_mul(4).wrapping_add(side.index() as u64)),
        Command::CropBendSide { page, side, .. } => (2, page.0.wrapping_mul(4).wrapping_add(side.index() as u64)),
        Command::CropSetSpine { photo, .. } => (3, photo.0),
        Command::PageSetStrength { page, .. } => (4, page.0),
        _ => return None,
    })
}

impl UndoStack {
    pub fn new() -> UndoStack {
        UndoStack::default()
    }

    /// Records a step. It joins the newest step when both are the same drag.
    pub fn record(&mut self, step: UndoStep) {
        self.redo.clear();
        if let Some(last) = self.undo.last_mut()
            && let (Some(a), Some(b)) = (target(&last.command), target(&step.command))
            && a == b
            && step.at_ms.saturating_sub(last.at_ms) <= COALESCE_MS
        {
            // Keep the older "before", take the newer "after".
            last.command = step.command;
            last.after = step.after;
            last.at_ms = step.at_ms;
            return;
        }
        self.undo.push(step);
        if self.undo.len() > MAX_UNDO {
            let drop = self.undo.len() - MAX_UNDO;
            self.undo.drain(..drop);
        }
    }

    /// Takes the newest step off the undo stack and puts it on the redo stack; the caller restores `before`.
    pub fn undo(&mut self) -> Option<Snapshot> {
        let step = self.undo.pop()?;
        let before = step.before.clone();
        self.redo.push(step);
        Some(before)
    }

    /// The reverse; the caller restores `after`.
    pub fn redo(&mut self) -> Option<Snapshot> {
        let step = self.redo.pop()?;
        let after = step.after.clone();
        self.undo.push(step);
        Some(after)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scancraft_core::command::Corner;
    use scancraft_core::recipe::{ExportOptions, SESSION_FORMAT};

    fn snapshot(photos: usize) -> Snapshot {
        Snapshot {
            file: SessionFile {
                format: SESSION_FORMAT,
                photos: Vec::new(),
                pages: Vec::new(),
                export: ExportOptions::default(),
                book_spreads: photos % 2 == 1,
                exported: false,
            },
            pending: Vec::new(),
            screen: Screen::Import,
            current_photo: Some(PhotoId(photos as u64)),
            current_page: None,
        }
    }

    fn step(command: Command, n: usize, at_ms: u64) -> UndoStep {
        UndoStep { command, before: snapshot(n), after: snapshot(n + 1), at_ms }
    }

    fn corner(page: u64, x: f64) -> Command {
        Command::CropSetCorner { page: PageId(page), corner: Corner::TopLeft, x, y: 0.0 }
    }

    #[test]
    fn undo_and_redo_walk_the_snapshots() {
        let mut s = UndoStack::new();
        s.record(step(Command::SessionNew, 0, 0));
        s.record(step(Command::SessionRestore, 1, 5_000));
        assert!(s.can_undo() && !s.can_redo());
        assert_eq!(s.undo().unwrap(), snapshot(1));
        assert!(s.can_redo());
        assert_eq!(s.undo().unwrap(), snapshot(0));
        assert!(s.undo().is_none());
        assert_eq!(s.redo().unwrap(), snapshot(1));
        assert_eq!(s.redo().unwrap(), snapshot(2));
        assert!(s.redo().is_none());
    }

    #[test]
    fn the_stack_holds_two_hundred_steps() {
        let mut s = UndoStack::new();
        for i in 0..250 {
            s.record(step(Command::SessionNew, i, i as u64 * 10_000));
        }
        assert_eq!(s.len(), MAX_UNDO);
        // The oldest fifty are gone: undoing everything reaches step 50's "before".
        let mut last = None;
        while let Some(b) = s.undo() {
            last = Some(b);
        }
        assert_eq!(last.unwrap(), snapshot(50));
    }

    #[test]
    fn a_drag_counts_as_one_step() {
        let mut s = UndoStack::new();
        for i in 0..20 {
            s.record(step(corner(1, f64::from(i)), i as usize, u64::from(i as u32) * 10));
        }
        assert_eq!(s.len(), 1);
        // The one step spans the whole drag.
        assert_eq!(s.undo().unwrap(), snapshot(0));
        assert_eq!(s.redo().unwrap(), snapshot(20));
        // Another page, or a pause, starts a new step.
        let mut s = UndoStack::new();
        s.record(step(corner(1, 0.0), 0, 0));
        s.record(step(corner(2, 0.0), 1, 10));
        s.record(step(corner(2, 1.0), 2, 10 + COALESCE_MS + 1));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn a_strength_slider_drag_counts_as_one_step() {
        // Every tick of the slider is a command. Each used to be a step of its own, so one drag
        // filled the 200-step stack and Undo took it back one tick at a time.
        let strength = |page: u64, value: u8| Command::PageSetStrength { page: PageId(page), value };
        let mut s = UndoStack::new();
        for i in 0..50u8 {
            s.record(step(strength(1, i), usize::from(i), u64::from(i) * 10));
        }
        assert_eq!(s.len(), 1);
        assert_eq!(s.undo().unwrap(), snapshot(0));
        // Another page's slider, or a pause, starts a new step.
        let mut s = UndoStack::new();
        s.record(step(strength(1, 10), 0, 0));
        s.record(step(strength(2, 10), 1, 10));
        s.record(step(strength(2, 20), 2, 10 + COALESCE_MS + 1));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn a_new_command_clears_the_redo_stack() {
        let mut s = UndoStack::new();
        s.record(step(Command::SessionNew, 0, 0));
        s.undo();
        assert!(s.can_redo());
        s.record(step(Command::SessionRestore, 0, 10_000));
        assert!(!s.can_redo());
        s.clear();
        assert!(s.is_empty() && !s.can_undo());
    }
}
