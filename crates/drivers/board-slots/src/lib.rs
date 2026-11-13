//! A board's slots, through its Orion node.
//!
//! Slots are side-by-side root images on one board, written by one writer: Lemnos's
//! `system.slots/1` driver. Atlas makes the slot bundle from the first-flash `.img.xz` on the
//! computer and asks the board to take it. Nothing runs on the board from Atlas: every step is
//! an action or a file on the node.
//!
//! - [`BoardSlots`]: list, set the default, start a trial, remove (guarded here, the board has
//!   the last word), pin, and set the keep count. Calls go through [`atlas_actions::BoardActions`]
//!   (short calls, refusals verbatim).
//! - [`SlotInstalls`]: the install, as a job with stages ([`atlas_slots::Stage`]). See [`install`]
//!   for how each step is made safe to repeat.
//!
//! A board whose offers do not include `system.slots/1` has no slots: [`SlotError::NoSlots`],
//! which is a normal state, not a fault.
//!
//! Starred and keep: one notion each. See `atlas_slots::policy`. Starred is the board's `pin`;
//! N is the board's `keep`. The install always passes `keep_old = false`.
//!
//! Trials: Atlas starts a trial and reads its record ([`atlas_slots::Trial`],
//! [`atlas_slots::Resolved`]). It never confirms one: the board confirms itself. Following the
//! trial across the reboot is C6.
//!
//! Names are Lemnos's contract method names (`system.slots.*`, the ones the bridge answers).
//! // LOCK: Lemnos L1: every list/accept/install call here needs the out-buffer and long-job
//! // forms at the L1 rev; today's lock answers the plain-argument calls and not `list`.

pub mod calls;
pub mod install;

use atlas_actions::ActionError;
use atlas_bundle::BundleError;
use atlas_jobs::JobError;
use atlas_slots::{ListError, RemoveRefusal};
use thiserror::Error;

pub use calls::BoardSlots;
pub use install::{InstallRequest, SlotInstalls, Source};

/// Why a slot operation gave no result. The board's own words are inside [`SlotError::Board`]
/// and print verbatim.
#[derive(Debug, Error)]
pub enum SlotError {
    /// The board does not offer the slot contract. Normal: "this board has no slots".
    #[error("this board has no slots")]
    NoSlots,
    /// The board refused, failed, or did not answer. Its words are shown as they are.
    #[error(transparent)]
    Board(#[from] ActionError),
    /// The board's slot list cannot be read.
    #[error("the board's slot list cannot be read: {0}")]
    BadList(#[from] ListError),
    /// The bundle is for another board. Refused on the computer, before anything is sent.
    #[error(
        "this slot is built for board {bundle}, and this is board {board}: every image is built for one board, so it was not sent"
    )]
    BoardMismatch { bundle: String, board: String },
    /// The removal is refused here, before the board is asked.
    #[error(transparent)]
    Removal(#[from] RemoveRefusal),
    /// The bundle could not be made or read.
    #[error("the slot bundle could not be made: {0}")]
    Bundle(String),
    /// The job list refused the install.
    #[error(transparent)]
    Job(#[from] JobError),
    /// The bundle error, when the bundle is built from an image.
    #[error(transparent)]
    Image(#[from] BundleError),
}
