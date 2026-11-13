//! The watches Atlas keeps, and what is saved of them so a trial is followed after Atlas restarts.
//!
//! What is saved is what a watch needs and nothing else: the board's key, the trial's slot id and
//! when the trial started. The board keeps its own records; Atlas re-reads them when the board is
//! seen again. A watch is released (removed from the saved list) once its outcome has been
//! reported, so the outcome of a trial is reported once.
//!
//! The bound: at most [`MAX_WATCHES`] watches are saved. A new trial on a board replaces that
//! board's watch. Beyond the bound the oldest watch (by start time) is dropped and returned to
//! the caller, so the drop is shown, not silent. A watch for a board that is never seen again
//! therefore stays until it is either reported or pushed out by newer trials.
//!
//! The store is a trait: this module does no I/O. The file-backed store is the driver's
//! (`atlas-board-slots`, `trial` module), in the app's data directory the caller passes in.

use crate::watch::{NodeKey, TrialState, Watch};

/// The most watches saved at once. Each is one small record (see [`Saved`]).
pub const MAX_WATCHES: usize = 16;

/// What is saved of one watch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub board: NodeKey,
    pub slot: String,
    pub started_ms: u64,
}

/// Where the saved watches live. A store that cannot be read or written says why, in words.
pub trait WatchStore: std::fmt::Debug {
    /// The saved watches. A store with no saved watches yet returns an empty list.
    ///
    /// # Errors
    ///
    /// A message when the saved watches cannot be read.
    fn load(&self) -> Result<Vec<Saved>, String>;

    /// Replaces the saved watches with `watches`.
    ///
    /// # Errors
    ///
    /// A message when the watches cannot be written.
    fn save(&self, watches: &[Saved]) -> Result<(), String>;
}

/// The result of starting a watch.
#[derive(Debug)]
pub struct Begun {
    /// The watch dropped to keep the saved list within [`MAX_WATCHES`], if one was.
    pub evicted: Option<Watch>,
    /// `Err` when the saved list could not be written. The watch is followed in memory anyway,
    /// but it will not be followed after an Atlas restart.
    pub saved: Result<(), String>,
}

/// The watches of one app, saved through a store.
#[derive(Debug)]
pub struct Trials<S: WatchStore> {
    store: S,
    watches: Vec<Watch>,
    problem: Option<String>,
}

impl<S: WatchStore> Trials<S> {
    /// Opens the saved watches. A saved list that cannot be read starts empty, and the reason is
    /// kept in [`Trials::problem`]. A saved list over the bound keeps the newest watches.
    pub fn open(store: S) -> Self {
        let (saved, problem) = match store.load() {
            Ok(saved) => (saved, None),
            Err(reason) => (Vec::new(), Some(reason)),
        };
        let mut watches: Vec<Watch> = Vec::with_capacity(saved.len());
        for entry in saved {
            // One watch per board: a later entry for the same board replaces an earlier one.
            watches.retain(|watch| watch.board() != entry.board);
            watches.push(Watch::resume(entry.board, entry.slot, entry.started_ms));
        }
        watches.sort_by_key(Watch::started_ms);
        let mut trials = Self {
            store,
            watches,
            problem,
        };
        while trials.watches.len() > MAX_WATCHES {
            trials.watches.remove(0);
        }
        trials
    }

    /// Why the saved watches could not be read at open, if they could not.
    #[must_use]
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The watches now held, oldest first.
    #[must_use]
    pub fn watches(&self) -> &[Watch] {
        &self.watches
    }

    /// The watch for `board`, if one is held.
    #[must_use]
    pub fn watch(&self, board: NodeKey) -> Option<&Watch> {
        self.watches.iter().find(|watch| watch.board() == board)
    }

    /// The watch for `board`, mutable, if one is held.
    pub fn watch_mut(&mut self, board: NodeKey) -> Option<&mut Watch> {
        self.watches.iter_mut().find(|watch| watch.board() == board)
    }

    /// Starts following a trial of `slot` on `board`, which replaces any watch held for that
    /// board. The saved list is written at once.
    pub fn begin(&mut self, board: NodeKey, slot: &str, started_ms: u64) -> Begun {
        self.watches.retain(|watch| watch.board() != board);
        self.watches.push(Watch::begin(board, slot, started_ms));
        let evicted = if self.watches.len() > MAX_WATCHES {
            let oldest = self
                .watches
                .iter()
                .enumerate()
                .min_by_key(|(_, watch)| watch.started_ms())
                .map(|(index, _)| index);
            oldest.map(|index| self.watches.remove(index))
        } else {
            None
        };
        let saved = self.save();
        Begun { evicted, saved }
    }

    /// Releases the watch for `board` if it has ended, and returns its final state, to be
    /// reported once. A watch still waiting is left as it is. The saved list is written when a
    /// watch is released.
    ///
    /// # Errors
    ///
    /// A message when the saved list cannot be written. The released state is still returned.
    pub fn release(&mut self, board: NodeKey) -> (Option<TrialState>, Result<(), String>) {
        let ended = self
            .watches
            .iter()
            .position(|watch| watch.board() == board && watch.finished());
        let Some(index) = ended else {
            return (None, Ok(()));
        };
        let watch = self.watches.remove(index);
        let saved = self.save();
        (Some(watch.state().clone()), saved)
    }

    fn save(&self) -> Result<(), String> {
        let saved: Vec<Saved> = self
            .watches
            .iter()
            .map(|watch| Saved {
                board: watch.board(),
                slot: watch.slot().to_owned(),
                started_ms: watch.started_ms(),
            })
            .collect();
        self.store.save(&saved)
    }
}
