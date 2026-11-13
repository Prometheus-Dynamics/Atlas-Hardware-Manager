//! The actions a board offers: listed from the board, run on its node, followed and cancelled.
//!
//! Nothing here names an action. A board describes its actions as a list ([`Catalog`]): a name,
//! and where it gives them, a human label, a description, the arguments with their types, and
//! whether it is long-running. A field the board does not give is [`Reported::NotReported`], and
//! the app shows that. An action Atlas has never heard of is listed and runs from its entry alone.
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

pub use board::{BoardActions, CATALOG_ACTION, DIRECT_WAIT, Done, POLL, Progress};
pub use catalog::{Action, Arg, ArgKind, Catalog, CatalogError, Reported, Route};
pub use error::ActionError;
pub use jobs::{ActionJobs, CANCEL_CONFIRM, CancelState, KIND};
