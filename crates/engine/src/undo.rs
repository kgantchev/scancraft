//! The undo stack (§6.3): one per session, up to 200 steps, and a whole drag counts as one step.
//!
//! A step keeps only what it changed. The pages and photos a command left alone are not copied
//! into it: a session of 100 pages with their recognised text is several megabytes, and 200 steps
//! each holding a copy of it came to about two gigabytes against the specification's budget of
//! 1.5. What a step keeps is the pages, photos and settings that differ before and after, with
//! the order of each list. Undoing builds the earlier state from the present one: the items the
//! step changed come back as they were, and the rest are the present ones.

use std::collections::{BTreeMap, BTreeSet};

use scancraft_core::command::Command;
use scancraft_core::engine_api::Screen;
use scancraft_core::recipe::{ExportOptions, Page, PageId, PhotoId, PhotoItem, SessionFile};

pub const MAX_UNDO: usize = 200;
/// A step of the same kind on the same target within this many milliseconds joins the one before.
pub const COALESCE_MS: u64 = 500;

/// Everything undo restores: the session file, the pages not yet in the tray, and where the user was.
///
/// This is the whole state at one moment. It is built to take a step's difference and to restore
/// from one, and it is not what the stack keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub file: SessionFile,
    pub pending: Vec<Page>,
    pub screen: Screen,
    pub current_photo: Option<PhotoId>,
    pub current_page: Option<PageId>,
}

/// Which end of a step: the state before the command, or after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Before,
    After,
}

/// Something with an id, kept in a list.
trait Keyed {
    fn key(&self) -> u64;
}

impl Keyed for Page {
    fn key(&self) -> u64 {
        self.id.0
    }
}

impl Keyed for PhotoItem {
    fn key(&self) -> u64 {
        self.id.0
    }
}

/// The items of a list by their ids.
fn by_id<T: Keyed>(list: &[T]) -> BTreeMap<u64, &T> {
    list.iter().map(|item| (item.key(), item)).collect()
}

/// One list of items as a step changed it.
#[derive(Debug, Clone, PartialEq)]
enum ListDiff<T> {
    /// The ids in order before and after, and the items that were added, removed or changed, as
    /// `(id, before, after)`. An item that is in neither list's difference is the same in both.
    Items { before: Vec<u64>, after: Vec<u64>, items: Vec<(u64, Option<T>, Option<T>)> },
    /// The two lists whole. This is for a list with an id twice, which only a damaged session
    /// file has: there an id doesn't name one item, so there is no difference to take.
    Whole { before: Vec<T>, after: Vec<T> },
}

impl<T: Keyed + Clone + PartialEq> ListDiff<T> {
    fn new(before: &[T], after: &[T]) -> ListDiff<T> {
        let ids_of = |list: &[T]| list.iter().map(Keyed::key).collect::<Vec<u64>>();
        let (before_ids, after_ids) = (ids_of(before), ids_of(after));
        let unique = |ids: &[u64]| ids.iter().collect::<BTreeSet<_>>().len() == ids.len();
        if !unique(&before_ids) || !unique(&after_ids) {
            return ListDiff::Whole { before: before.to_vec(), after: after.to_vec() };
        }
        let (old, new) = (by_id(before), by_id(after));
        let all: BTreeSet<u64> = before_ids.iter().chain(after_ids.iter()).copied().collect();
        let mut items = Vec::new();
        for id in all {
            let (was, is) = (old.get(&id).copied(), new.get(&id).copied());
            if was != is {
                items.push((id, was.cloned(), is.cloned()));
            }
        }
        ListDiff::Items { before: before_ids, after: after_ids, items }
    }

    /// The list at one end of the step, given the list as it is now: the items the step changed
    /// are taken from it, and the others from `now`.
    fn apply(&self, which: Which, now: &[T]) -> Vec<T> {
        match self {
            ListDiff::Whole { before, after } => match which {
                Which::Before => before.clone(),
                Which::After => after.clone(),
            },
            ListDiff::Items { before, after, items } => {
                let order = if which == Which::Before { before } else { after };
                let changed: BTreeMap<u64, Option<&T>> =
                    items.iter().map(|(id, was, is)| (*id, if which == Which::Before { was.as_ref() } else { is.as_ref() })).collect();
                let mut present: BTreeMap<u64, &T> = BTreeMap::new();
                for item in now {
                    present.entry(item.key()).or_insert(item);
                }
                order
                    .iter()
                    .filter_map(|id| match changed.get(id) {
                        Some(item) => item.cloned(),
                        None => present.get(id).map(|item| (*item).clone()),
                    })
                    .collect()
            }
        }
    }

    /// Whether this step and the next can be one. Lists kept whole can't, so a drag in a damaged
    /// session is more than one step and nothing else is lost.
    fn mergeable(&self, next: &ListDiff<T>) -> bool {
        matches!((self, next), (ListDiff::Items { .. }, ListDiff::Items { .. }))
    }

    /// This step and the next as one: each item as it was before the first and is after the
    /// second. Only for two lists that are `mergeable`; for others it gives the later one, which
    /// is why `Diff::merge` checks first.
    fn merge(self, next: ListDiff<T>) -> ListDiff<T> {
        match (self, next) {
            (ListDiff::Items { before, items: first, .. }, ListDiff::Items { after, items: second, .. }) => {
                let mut merged: BTreeMap<u64, (Option<T>, Option<T>)> = BTreeMap::new();
                for (id, was, is) in first {
                    merged.insert(id, (was, is));
                }
                for (id, was, is) in second {
                    match merged.get_mut(&id) {
                        // Changed by the first: it was as the first had it, and is as the second leaves it.
                        Some(entry) => entry.1 = is,
                        // Left alone by the first: the second's "before" is what it was all along.
                        None => {
                            merged.insert(id, (was, is));
                        }
                    }
                }
                let items = merged.into_iter().filter(|(_, (was, is))| was != is).map(|(id, (was, is))| (id, was, is)).collect();
                ListDiff::Items { before, after, items }
            }
            (_, next) => next,
        }
    }

    /// How many items this keeps whole, for the tests and for measuring.
    fn stored(&self) -> usize {
        match self {
            ListDiff::Items { items, .. } => items.len(),
            ListDiff::Whole { before, after } => before.len() + after.len(),
        }
    }
}

/// A setting as a step changed it, when it did.
#[derive(Debug, Clone, PartialEq)]
struct Pair<T> {
    before: T,
    after: T,
}

impl<T: Clone + PartialEq> Pair<T> {
    fn between(before: &T, after: &T) -> Option<Pair<T>> {
        (before != after).then(|| Pair { before: before.clone(), after: after.clone() })
    }

    fn at(pair: &Option<Pair<T>>, which: Which, now: &T) -> T {
        match pair {
            Some(p) if which == Which::Before => p.before.clone(),
            Some(p) => p.after.clone(),
            None => now.clone(),
        }
    }

    fn merge(first: Option<Pair<T>>, second: Option<Pair<T>>) -> Option<Pair<T>> {
        match (first, second) {
            (None, other) | (other, None) => other,
            (Some(a), Some(b)) => (a.before != b.after).then_some(Pair { before: a.before, after: b.after }),
        }
    }
}

/// Where the user was: the screen and the photo and page it showed.
#[derive(Debug, Clone, PartialEq)]
struct Place {
    screen: Screen,
    photo: Option<PhotoId>,
    page: Option<PageId>,
}

impl Place {
    fn of(snapshot: &Snapshot) -> Place {
        Place { screen: snapshot.screen, photo: snapshot.current_photo, page: snapshot.current_page }
    }
}

/// What one step changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    photos: ListDiff<PhotoItem>,
    pages: ListDiff<Page>,
    pending: ListDiff<Page>,
    export: Option<Pair<ExportOptions>>,
    book_spreads: Option<Pair<bool>>,
    exported: Option<Pair<bool>>,
    before_place: Place,
    after_place: Place,
}

impl Diff {
    /// The difference between the state before a command and the state after it.
    pub fn between(before: &Snapshot, after: &Snapshot) -> Diff {
        Diff {
            photos: ListDiff::new(&before.file.photos, &after.file.photos),
            pages: ListDiff::new(&before.file.pages, &after.file.pages),
            pending: ListDiff::new(&before.pending, &after.pending),
            export: Pair::between(&before.file.export, &after.file.export),
            book_spreads: Pair::between(&before.file.book_spreads, &after.file.book_spreads),
            exported: Pair::between(&before.file.exported, &after.file.exported),
            before_place: Place::of(before),
            after_place: Place::of(after),
        }
    }

    /// The state at one end of the step, built from the state now.
    ///
    /// Of what the step did not change, the present state is kept: a page's text that arrived
    /// after the step, and a session already exported, are not taken back by undoing something
    /// else.
    fn apply(&self, which: Which, now: &Snapshot) -> Snapshot {
        let place = if which == Which::Before { &self.before_place } else { &self.after_place };
        Snapshot {
            file: SessionFile {
                format: now.file.format,
                photos: self.photos.apply(which, &now.file.photos),
                pages: self.pages.apply(which, &now.file.pages),
                export: Pair::at(&self.export, which, &now.file.export),
                book_spreads: Pair::at(&self.book_spreads, which, &now.file.book_spreads),
                exported: Pair::at(&self.exported, which, &now.file.exported),
            },
            pending: self.pending.apply(which, &now.pending),
            screen: place.screen,
            current_photo: place.photo,
            current_page: place.page,
        }
    }

    /// Whether this step and the next can be one: not when a list is kept whole.
    fn mergeable(&self, next: &Diff) -> bool {
        self.photos.mergeable(&next.photos) && self.pages.mergeable(&next.pages) && self.pending.mergeable(&next.pending)
    }

    /// This step followed by the next as one. Only for two steps that are `mergeable`.
    fn merge(self, next: Diff) -> Diff {
        Diff {
            photos: self.photos.merge(next.photos),
            pages: self.pages.merge(next.pages),
            pending: self.pending.merge(next.pending),
            export: Pair::merge(self.export, next.export),
            book_spreads: Pair::merge(self.book_spreads, next.book_spreads),
            exported: Pair::merge(self.exported, next.exported),
            before_place: self.before_place,
            after_place: next.after_place,
        }
    }

    /// How many pages and photos this step keeps whole.
    pub fn stored_items(&self) -> usize {
        self.photos.stored() + self.pages.stored() + self.pending.stored()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UndoStep {
    pub command: Command,
    diff: Diff,
    /// The engine's clock when the step was made, for the coalescing window.
    pub at_ms: u64,
}

impl UndoStep {
    /// A step from the state before a command and the state after it.
    pub fn new(command: Command, before: &Snapshot, after: &Snapshot, at_ms: u64) -> UndoStep {
        UndoStep { command, diff: Diff::between(before, after), at_ms }
    }
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
        let same_drag = self.undo.last().is_some_and(|last| {
            matches!((target(&last.command), target(&step.command)), (Some(a), Some(b)) if a == b)
                && step.at_ms.saturating_sub(last.at_ms) <= COALESCE_MS
                // A step that keeps a list whole (a damaged session) is left as it is.
                && last.diff.mergeable(&step.diff)
        });
        if same_drag && let Some(last) = self.undo.pop() {
            // Keep the older "before", take the newer "after".
            let diff = last.diff.merge(step.diff);
            self.undo.push(UndoStep { command: step.command, diff, at_ms: step.at_ms });
            return;
        }
        self.undo.push(step);
        if self.undo.len() > MAX_UNDO {
            let drop = self.undo.len() - MAX_UNDO;
            self.undo.drain(..drop);
        }
    }

    /// Takes the newest step off the undo stack and puts it on the redo stack; the caller restores
    /// the state it returns, which is `now` as it was before the step.
    pub fn undo(&mut self, now: &Snapshot) -> Option<Snapshot> {
        let step = self.undo.pop()?;
        let restored = step.diff.apply(Which::Before, now);
        self.redo.push(step);
        Some(restored)
    }

    /// The reverse; the caller restores the state it returns, which is `now` as it was after the step.
    pub fn redo(&mut self, now: &Snapshot) -> Option<Snapshot> {
        let step = self.redo.pop()?;
        let restored = step.diff.apply(Which::After, now);
        self.undo.push(step);
        Some(restored)
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

    /// How many pages and photos the steps keep whole, in all.
    pub fn stored_items(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|step| step.diff.stored_items()).sum()
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
    use scancraft_core::determinism::Xorshift64Star;
    use scancraft_core::geom::{Point, Quad};
    use scancraft_core::recipe::{
        CornerOrigin, FlattenMode, ImportOrigin, Look, RecognizedText, SESSION_FORMAT, SideShape, SourceId, TextLine, TurnOrigin, Word,
    };

    fn page(id: u64, photo: u64) -> Page {
        Page {
            id: PageId(id),
            photo: PhotoId(photo),
            source: SourceId("ab".into()),
            corners: Quad([Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)]),
            corners_from: CornerOrigin::Manual,
            sides: [SideShape::Straight, SideShape::Straight, SideShape::Straight, SideShape::Straight],
            spread: None,
            flatten: FlattenMode::Auto,
            sheet: None,
            proportions: None,
            page_size_mm: None,
            quarter_turns: 0,
            turns_from: TurnOrigin::Manual,
            look: Look::ColorDocument,
            strength: 60,
            text: None,
        }
    }

    fn photo(id: u64) -> PhotoItem {
        PhotoItem {
            id: PhotoId(id),
            source: SourceId("ab".into()),
            name: format!("p{id}.jpg"),
            origin: ImportOrigin::File,
            size: (100, 80),
            focal: Default::default(),
        }
    }

    /// Recognised text of `words` words, as a page of print has.
    fn text(words: usize) -> RecognizedText {
        let line = |n: usize| TextLine {
            words: (0..10)
                .map(|w| Word { text: format!("word{n}x{w}"), rect: [0.1, 0.1, 0.2, 0.2], confidence: 0.9, source: "vision".to_string() })
                .collect(),
        };
        RecognizedText { engine: "vision".to_string(), engine_version: "27".to_string(), lines: (0..words / 10).map(line).collect() }
    }

    fn snapshot(photos: Vec<PhotoItem>, pages: Vec<Page>, pending: Vec<Page>) -> Snapshot {
        Snapshot {
            file: SessionFile { format: SESSION_FORMAT, photos, pages, export: ExportOptions::default(), book_spreads: false, exported: false },
            pending,
            screen: Screen::Import,
            current_photo: None,
            current_page: None,
        }
    }

    fn step(command: Command, before: &Snapshot, after: &Snapshot, at_ms: u64) -> UndoStep {
        UndoStep::new(command, before, after, at_ms)
    }

    fn corner(page: u64, x: f64) -> Command {
        Command::CropSetCorner { page: PageId(page), corner: Corner::TopLeft, x, y: 0.0 }
    }

    fn strength(page: u64, value: u8) -> Command {
        Command::PageSetStrength { page: PageId(page), value }
    }

    /// A state with `n` pages that have a little of a book each, and the ones before it.
    fn states(n: u64) -> Vec<Snapshot> {
        (0..=n)
            .map(|i| {
                let mut s = snapshot(vec![photo(1)], vec![page(1, 1)], Vec::new());
                s.file.pages[0].strength = (i % 100) as u8;
                s.file.book_spreads = i % 2 == 1;
                s
            })
            .collect()
    }

    #[test]
    fn undo_and_redo_walk_the_states() {
        let s = states(2);
        let mut stack = UndoStack::new();
        stack.record(step(Command::SessionNew, &s[0], &s[1], 0));
        stack.record(step(Command::SessionRestore, &s[1], &s[2], 5_000));
        assert!(stack.can_undo() && !stack.can_redo());
        assert_eq!(stack.undo(&s[2]).unwrap(), s[1]);
        assert!(stack.can_redo());
        assert_eq!(stack.undo(&s[1]).unwrap(), s[0]);
        assert!(stack.undo(&s[0]).is_none());
        assert_eq!(stack.redo(&s[0]).unwrap(), s[1]);
        assert_eq!(stack.redo(&s[1]).unwrap(), s[2]);
        assert!(stack.redo(&s[2]).is_none());
    }

    #[test]
    fn the_stack_holds_two_hundred_steps() {
        let s = states(250);
        let mut stack = UndoStack::new();
        for i in 0..250 {
            stack.record(step(Command::SessionNew, &s[i], &s[i + 1], i as u64 * 10_000));
        }
        assert_eq!(stack.len(), MAX_UNDO);
        // The oldest fifty are gone: undoing everything reaches the state before step 50.
        let mut now = s[250].clone();
        while let Some(before) = stack.undo(&now) {
            now = before;
        }
        assert_eq!(now, s[50]);
    }

    #[test]
    fn a_drag_counts_as_one_step() {
        let s = states(20);
        let mut stack = UndoStack::new();
        for i in 0..20usize {
            stack.record(step(corner(1, i as f64), &s[i], &s[i + 1], i as u64 * 10));
        }
        assert_eq!(stack.len(), 1);
        // The one step spans the whole drag.
        assert_eq!(stack.undo(&s[20]).unwrap(), s[0]);
        assert_eq!(stack.redo(&s[0]).unwrap(), s[20]);
        // Another page, or a pause, starts a new step.
        let mut stack = UndoStack::new();
        stack.record(step(corner(1, 0.0), &s[0], &s[1], 0));
        stack.record(step(corner(2, 0.0), &s[1], &s[2], 10));
        stack.record(step(corner(2, 1.0), &s[2], &s[3], 10 + COALESCE_MS + 1));
        assert_eq!(stack.len(), 3);
    }

    #[test]
    fn a_strength_slider_drag_counts_as_one_step() {
        // Every tick of the slider is a command. Each used to be a step of its own, so one drag
        // filled the 200-step stack and Undo took it back one tick at a time.
        let s = states(50);
        let mut stack = UndoStack::new();
        for i in 0..50usize {
            stack.record(step(strength(1, i as u8), &s[i], &s[i + 1], i as u64 * 10));
        }
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.undo(&s[50]).unwrap(), s[0]);
        // Another page's slider, or a pause, starts a new step.
        let mut stack = UndoStack::new();
        stack.record(step(strength(1, 10), &s[0], &s[1], 0));
        stack.record(step(strength(2, 10), &s[1], &s[2], 10));
        stack.record(step(strength(2, 20), &s[2], &s[3], 10 + COALESCE_MS + 1));
        assert_eq!(stack.len(), 3);
    }

    #[test]
    fn a_new_command_clears_the_redo_stack() {
        let s = states(2);
        let mut stack = UndoStack::new();
        stack.record(step(Command::SessionNew, &s[0], &s[1], 0));
        stack.undo(&s[1]);
        assert!(stack.can_redo());
        stack.record(step(Command::SessionRestore, &s[0], &s[1], 10_000));
        assert!(!stack.can_redo());
        stack.clear();
        assert!(stack.is_empty() && !stack.can_undo());
    }

    /// The state after a few random edits of `from`: pages and photos changed, added, removed and
    /// reordered, the settings turned over, and the user somewhere else.
    fn edited(from: &Snapshot, rng: &mut Xorshift64Star, next_id: &mut u64) -> Snapshot {
        let mut s = from.clone();
        for _ in 0..1 + rng.below(4) {
            match rng.below(12) {
                0 | 1 => {
                    let at = rng.below(s.file.pages.len().max(1) as u64) as usize;
                    if let Some(p) = s.file.pages.get_mut(at) {
                        p.strength = rng.below(101) as u8;
                    }
                }
                2 => {
                    let at = rng.below(s.file.pages.len().max(1) as u64) as usize;
                    if let Some(p) = s.file.pages.get_mut(at) {
                        p.text = if p.text.is_some() { None } else { Some(text(20)) };
                    }
                }
                3 => {
                    *next_id += 1;
                    let at = rng.below(s.file.pages.len() as u64 + 1) as usize;
                    s.file.pages.insert(at, page(*next_id, 1));
                }
                4 => {
                    if !s.file.pages.is_empty() {
                        let at = rng.below(s.file.pages.len() as u64) as usize;
                        s.file.pages.remove(at);
                    }
                }
                5 => {
                    let n = s.file.pages.len();
                    if n > 1 {
                        s.file.pages.swap(rng.below(n as u64) as usize, rng.below(n as u64) as usize);
                    }
                }
                6 => {
                    *next_id += 1;
                    s.pending.push(page(*next_id, 1));
                }
                7 => {
                    if !s.pending.is_empty() {
                        let at = rng.below(s.pending.len() as u64) as usize;
                        s.pending.remove(at);
                    }
                }
                8 => {
                    *next_id += 1;
                    s.file.photos.push(photo(*next_id));
                }
                9 => {
                    let at = rng.below(s.file.photos.len().max(1) as u64) as usize;
                    if let Some(p) = s.file.photos.get_mut(at) {
                        p.size.0 += 1;
                    }
                }
                10 => {
                    s.file.book_spreads = !s.file.book_spreads;
                    s.file.export.searchable = !s.file.export.searchable;
                }
                _ => {
                    s.screen = if rng.below(2) == 0 { Screen::Import } else { Screen::Export };
                    s.current_page = Some(PageId(rng.below(5)));
                }
            }
        }
        s
    }

    #[test]
    fn a_step_gives_back_exactly_the_state_on_either_side_of_it() {
        let mut rng = Xorshift64Star::new(0xC0FFEE);
        let mut next_id = 100;
        for round in 0..300 {
            let a = edited(&snapshot(vec![photo(1), photo(2)], vec![page(1, 1), page(2, 1), page(3, 2)], vec![page(4, 2)]), &mut rng, &mut next_id);
            let b = edited(&a, &mut rng, &mut next_id);
            let c = edited(&b, &mut rng, &mut next_id);
            let (one, two) = (UndoStep::new(corner(1, 0.0), &a, &b, 0), UndoStep::new(corner(1, 1.0), &b, &c, 10));
            assert_eq!(one.diff.apply(Which::Before, &b), a, "round {round}: undo");
            assert_eq!(one.diff.apply(Which::After, &a), b, "round {round}: redo");
            // Two steps joined into one are the same as the whole way across.
            assert!(one.diff.mergeable(&two.diff), "round {round}: the steps could not be joined");
            let joined = one.diff.merge(two.diff);
            assert_eq!(joined.apply(Which::Before, &c), a, "round {round}: joined undo");
            assert_eq!(joined.apply(Which::After, &a), c, "round {round}: joined redo");
        }
    }

    #[test]
    fn a_step_keeps_only_the_pages_it_changed() {
        // 100 pages with a page of recognised text each: what the stack kept per step was all of it.
        let pages: Vec<Page> = (1..=100)
            .map(|id| {
                let mut p = page(id, 1);
                p.text = Some(text(400));
                p
            })
            .collect();
        let mut state = snapshot(vec![photo(1)], pages, Vec::new());
        let mut stack = UndoStack::new();
        for i in 0..250u64 {
            let before = state.clone();
            // One page's strength per step, a pause apart so the steps stay apart.
            // 61 to 100, never the 60 a page starts at, and never what the page had the last time (100 steps ago).
            state.file.pages[(i % 100) as usize].strength = 61 + (i % 40) as u8;
            stack.record(UndoStep::new(Command::PageSetStrength { page: PageId(i % 100 + 1), value: 1 }, &before, &state, i * 10_000));
        }
        assert_eq!(stack.len(), MAX_UNDO);
        // One page in each step, and not a hundred.
        assert_eq!(stack.stored_items(), MAX_UNDO, "a step holds more than the page it changed");
        // And undoing all of it still ends at the state before the oldest step kept.
        let mut now = state.clone();
        let mut count = 0;
        while let Some(before) = stack.undo(&now) {
            now = before;
            count += 1;
        }
        assert_eq!(count, MAX_UNDO);
        assert_eq!(now.file.pages.len(), 100);
        assert!(now.file.pages.iter().all(|p| p.text.is_some()));
    }

    #[test]
    fn what_a_step_did_not_change_stays_as_it_is_now() {
        // A step that changed page 1; then page 2's text arrived (a job's result, which is no step).
        let before = snapshot(vec![photo(1)], vec![page(1, 1), page(2, 1)], Vec::new());
        let mut after = before.clone();
        after.file.pages[0].strength = 90;
        let step = UndoStep::new(strength(1, 90), &before, &after, 0);
        let mut now = after.clone();
        now.file.pages[1].text = Some(text(20));
        let undone = step.diff.apply(Which::Before, &now);
        assert_eq!(undone.file.pages[0].strength, 60, "the page the step changed comes back");
        assert!(undone.file.pages[1].text.is_some(), "the text that arrived since is not taken back");
        // And a session that was exported since is still exported.
        now.file.exported = true;
        assert!(step.diff.apply(Which::Before, &now).file.exported);
    }

    #[test]
    fn a_step_that_changes_a_setting_changes_it_back() {
        let before = snapshot(vec![photo(1)], vec![page(1, 1)], Vec::new());
        let mut after = before.clone();
        after.file.book_spreads = true;
        after.file.export.searchable = false;
        let step = UndoStep::new(Command::CropBook { on: true }, &before, &after, 0);
        assert_eq!(step.diff.stored_items(), 0, "a setting is not a page");
        assert_eq!(step.diff.apply(Which::Before, &after), before);
        assert_eq!(step.diff.apply(Which::After, &before), after);
    }

    #[test]
    fn a_damaged_session_with_an_id_twice_still_undoes_exactly() {
        // A session file is input, and an id twice is what a hand edit can make. There an id
        // doesn't name one page, so the step keeps the lists whole, and is still exact.
        let before = snapshot(vec![photo(1)], vec![page(1, 1), page(1, 1)], Vec::new());
        let mut after = before.clone();
        after.file.pages[1].strength = 5;
        let step = UndoStep::new(strength(1, 5), &before, &after, 0);
        assert_eq!(step.diff.apply(Which::Before, &after), before);
        assert_eq!(step.diff.apply(Which::After, &before), after);
        // Such steps are not joined, and nothing is lost by that.
        let mut stack = UndoStack::new();
        let third = {
            let mut t = after.clone();
            t.file.pages[1].strength = 6;
            t
        };
        stack.record(step);
        stack.record(UndoStep::new(strength(1, 6), &after, &third, 10));
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.undo(&third).unwrap(), after);
        assert_eq!(stack.undo(&after).unwrap(), before);
    }
}
