//! The actions a board offers: listed from the board, run on its node, followed and cancelled.
//!
//! Nothing here names an action. What a board offers is read from the board ([`Catalog`]): the
//! node's resource list names its devices and their `call` action, and each device's description
//! names its methods, with their arguments and shapes and whether each is a long job. A field the
//! board does not give is [`Reported::NotReported`], and the app shows that. A method Atlas has
//! never heard of is listed and runs from its entry alone.
//!
//! - [`BoardActions`] runs them through the node's session ([`atlas_session::Handle`]): read the
//!   list ([`BoardActions::catalog`]), run a short action ([`BoardActions::run`]), or start one
//!   detached and follow it ([`BoardActions::start`], [`BoardActions::follow`],
//!   [`BoardActions::cancel`]).
//! - [`ActionJobs`] runs a long action as a job of [`atlas_jobs`]: it has progress, a cancel that
//!   checks the board stopped, and it re-attaches to the run after a lost session instead of
//!   starting it again.
//!
//! A refusal, failure or cancel from the board is returned with the board's own words
//! ([`ActionError::board_reason`]). Arguments are checked against the board's description before
//! they are sent, where the board gives one.
//!
//! This crate depends on Orion (the client) and the session, jobs and driver contract. It is not
//! a driver, and it knows no family, board or product.

mod args;
mod board;
mod catalog;
mod error;
mod jobs;

pub use board::{
    BoardActions, DIRECT_WAIT, Done, Followed, Issued, LEASE_ENDED_PREFIX, POLL, Progress,
    any_answer, lease_ended,
};
pub use catalog::{Action, Arg, ArgKind, Catalog, Reported, Route};
pub use error::{ActionError, BUSY_CODE};
pub use jobs::{ActionJobs, CANCEL_CONFIRM, CancelState, KIND};
