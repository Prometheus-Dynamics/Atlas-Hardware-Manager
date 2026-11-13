//! The retention policy: keep the last N, keep what is starred, and guard removal.
//!
//! Atlas exposes ONE notion of "starred" and one count, N.
//!
//! - **Starred** is the board's `pin` (Lemnos `pin`, and the `pin` argument of `install_file`).
//!   A starred version is never removed by keep-last-N, and never counts toward N. The board
//!   keeps the same promise: a pinned slot never counts and never goes (I5-actions.md,
//!   "Housekeeping").
//! - **N** is the board's `set_keep` (`keep`). Zero keeps all. There is no forced "previous"
//!   slot and no target count beyond N.
//!
//! Atlas does not send the board's per-update `keep_old`: the one retention concept Atlas has is
//! starred plus N, so an install always passes `keep_old = false` and the board's housekeeping
//! runs. The board's `keep_old` skips housekeeping for one update, which is a second notion of
//! keeping that this build does not offer.
//!
//! Inside the board's rule, the running, default and trial slots are protected and count
//! toward N (unless starred), then the newest unstarred slots fill the rest. The function
//! [`would_remove`] is the same rule over a view of the list, so the app can say "this will
//! remove X" before the person confirms.

use crate::list::SlotList;

/// One slot as the policy sees it: its id, whether it is starred, and whether it is protected
/// (running, default or trial). Views are newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: String,
    pub starred: bool,
    pub protected: bool,
}

/// The policy's view of a board's list, newest first.
#[must_use]
pub fn view_of(list: &SlotList) -> Vec<View> {
    list.slots
        .iter()
        .map(|slot| View {
            id: slot.id.clone(),
            starred: slot.starred(),
            protected: slot.has_flag("running")
                || slot.has_flag("default")
                || slot.has_flag("trial")
                || slot.id == list.trial_slot
                || slot.id == list.default
                || slot.id == list.running,
        })
        .collect()
}

/// The view after an install of `id`, starred or not: the new slot is the newest.
#[must_use]
pub fn with_install(views: &[View], id: &str, starred: bool) -> Vec<View> {
    let mut next = Vec::with_capacity(views.len() + 1);
    next.push(View {
        id: id.to_owned(),
        starred,
        protected: false,
    });
    next.extend(views.iter().filter(|view| view.id != id).cloned());
    next
}

/// The ids keep-last-N removes, newest-first rule, given a view and N. Starred slots never go and
/// never count. Protected unstarred slots count. Zero keeps everything.
#[must_use]
pub fn would_remove(views: &[View], keep: u32) -> Vec<String> {
    if keep == 0 {
        return Vec::new();
    }
    let keep = keep as usize;
    let mut count = views
        .iter()
        .filter(|view| view.protected && !view.starred)
        .count();
    let mut removed = Vec::new();
    for view in views
        .iter()
        .filter(|view| !view.protected && !view.starred)
    {
        if count < keep {
            count += 1;
        } else {
            removed.push(view.id.clone());
        }
    }
    removed
}

/// What an install of `id` (starred or not) would remove under keep N, given the board's list
/// now. The new slot is counted as installed, so the result is the board's answer, not a guess.
#[must_use]
pub fn would_remove_on_install(list: &SlotList, keep: u32, id: &str, starred: bool) -> Vec<String> {
    let views = with_install(&view_of(list), id, starred);
    would_remove(&views, keep)
}

/// Why a slot cannot be removed. Atlas refuses these before asking the board; the board still has
/// the last word and its own words are shown when it refuses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveRefusal {
    /// The board does not list the slot.
    Missing(String),
    /// The running slot is never removed.
    Running(String),
    /// The default slot is never removed.
    Default(String),
    /// A pending trial is never removed.
    Trial(String),
    /// The only slot on the board is never removed.
    OnlySlot(String),
    /// A starred slot is removed only when the person says so explicitly.
    Starred(String),
}

impl std::fmt::Display for RemoveRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(id) => write!(f, "there is no slot {id} on this board"),
            Self::Running(id) => write!(f, "{id} is the running slot, and is never removed"),
            Self::Default(id) => write!(f, "{id} is the default slot; make another slot the default first"),
            Self::Trial(id) => write!(f, "{id} is on trial; it is never removed while the trial is pending"),
            Self::OnlySlot(id) => write!(f, "{id} is the only slot on this board"),
            Self::Starred(id) => write!(f, "{id} is starred; remove it only if you say so explicitly"),
        }
    }
}

/// Checks that `id` may be removed. `allow_starred` is the person's explicit choice.
///
/// # Errors
///
/// The first [`RemoveRefusal`] that applies.
pub fn check_remove(list: &SlotList, id: &str, allow_starred: bool) -> Result<(), RemoveRefusal> {
    let slot = list
        .get(id)
        .ok_or_else(|| RemoveRefusal::Missing(id.to_owned()))?;
    if slot.has_flag("running") || list.running == id {
        return Err(RemoveRefusal::Running(id.to_owned()));
    }
    if slot.has_flag("default") || list.default == id {
        return Err(RemoveRefusal::Default(id.to_owned()));
    }
    if slot.has_flag("trial") || list.trial_slot == id || list.trial.as_ref().is_some_and(|t| t.slot == id) {
        return Err(RemoveRefusal::Trial(id.to_owned()));
    }
    if list.slots.len() == 1 {
        return Err(RemoveRefusal::OnlySlot(id.to_owned()));
    }
    if slot.starred() && !allow_starred {
        return Err(RemoveRefusal::Starred(id.to_owned()));
    }
    Ok(())
}
