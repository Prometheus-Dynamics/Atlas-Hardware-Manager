//! Slots, as data and rules: nothing here reaches a board or a disk.
//!
//! Slots are side-by-side root images on one board, one writer (Lemnos's `system.slots/1`
//! driver), no A/B. This crate holds what the rest of Atlas reads and decides:
//!
//! - [`list`]: the board's slot list as data. Its trial and resolved records (I5f) and the
//!   per-slot signature outcome (I5g) are read as the board writes them; an absent signature is
//!   "not reported", never verified.
//! - [`BoardOutcome`]: the signature outcome, the one table first flash and slot installs share
//!   (atlas-image, PR 56). Used directly; nothing here re-words it.
//! - [`policy`]: keep-last-N with starred versions, what an install would remove, and the guard
//!   on removal. The board has the last word on every removal.
//! - [`stage`]: the stages an install goes through, as the app shows them.
//! - [`SUPPORTED_CONTRACT`] and [`offers_slots`]: whether a board offers the slot contract at all
//!   (C10: capabilities come from `hello.offers`). A board without it has no slots: a normal state.
//!
//! Starred is the board's pin; N is the board's keep. See [`policy`] for why there is one notion
//! of each, and why the board's per-update `keep_old` is not used.

pub mod list;
pub mod policy;
pub mod stage;

pub use atlas_image::BoardOutcome;
pub use list::{
    Attempt, ListError, Reported, Resolution, Resolved, Slot, SlotList, Trial, TrialLast,
};
pub use policy::{
    RemoveRefusal, View, check_remove, view_of, with_install, would_remove, would_remove_on_install,
};
pub use stage::Stage;

/// The Lemnos contract name a board offers for slots. Read from `hello.offers`, which lists
/// contract ids separated by spaces.
pub const SUPPORTED_CONTRACT: &str = "system.slots/1";

/// Whether a board's `offers` text includes the slot contract. An absent offers text is "no".
#[must_use]
pub fn offers_slots(offers: &str) -> bool {
    offers
        .split_whitespace()
        .any(|offer| offer == SUPPORTED_CONTRACT)
}
