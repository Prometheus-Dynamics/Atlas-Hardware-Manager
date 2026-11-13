//! The trial watch (C6): Atlas does not confirm an update. The board confirms itself after a
//! trial boot, or falls back, and records what it did. Atlas follows the board through the
//! restarts and shows the outcome.
//!
//! - [`watch`]: one board's trial, followed by events. [`watch::Watch`] takes what the session
//!   says ([`watch::Seen`]) and the board's records ([`watch::Read`]), and says what a person sees
//!   ([`watch::TrialState`]). The outcome comes only from the board's `resolved` record; the
//!   running version (`version.slot`) is compared with the trial's slot, and a disagreement shows
//!   both facts.
//! - [`ledger`]: the watches Atlas holds, saved through a [`ledger::WatchStore`] so a trial is
//!   followed after an Atlas restart. Bounded by [`ledger::MAX_WATCHES`]; released once reported.
//! - [`settling`]: the device list's predicate: "update still settling", and health that does not
//!   turn red while a trial is pending.
//!
//! No clock is read here and no deadline is set: a board on a robot takes what it takes. The
//! driver (`atlas-board-slots`, `trial` module) supplies the events and the store.

pub mod ledger;
pub mod settling;
pub mod watch;

pub use ledger::{Begun, MAX_WATCHES, Saved, Trials, WatchStore};
pub use settling::{health_while_settling, settling};
pub use watch::{
    Decided, Next, NodeKey, Outcome, RUNNING_FACT, Read, Running, Seen, Settling, TrialState,
    Watch, hex, key_from_hex,
};
