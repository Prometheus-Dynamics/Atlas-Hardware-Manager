//! Sending a file to a board, through its Orion node: resumable, streamed, checked end to end,
//! and reported as a job.
//!
//! - [`submit`] starts a push of a [`PushSource`] to a destination name on the node behind a
//!   [`atlas_session::Handle`]. It returns a [`Push`]: its job, its [`Progress`], its
//!   [`Counters`], and [`Push::cancel`].
//! - A push runs as one step of a [`PushKind`] job, so it shows in the jobs like any other. Register
//!   the kind when the jobs are opened.
//!
//! Each attempt is one long operation of the node's session ([`atlas_session::Handle::long`]); the
//! file is sent in chunks and checked by the board against its hash. When the session breaks, the push
//! waits (never more than the failure rule's cap between tries, and never gives up) and begins
//! again: the board answers with how many bytes it already holds, so nothing it acknowledged is sent
//! again. A different file under the same name is a different transfer on the board, so it starts
//! from nothing and never adds to another file's bytes. A file that changed while it was being
//! pushed is refused before anything more is sent.
//!
//! Memory: the file is never held. The push keeps one chunk buffer and at most nine chunks of 32 KiB
//! unacknowledged at any moment (Orion's window), see [`Counters::max_in_flight`].
//!
//! Today's push holds the node's session while the file moves: the locked Orion rev has only a
//! blocking `Files::push`, so calls to that node made meanwhile wait until it ends. The session
//! already carries calls between the steps of a long operation; the stepped push of Orion R2 is what
//! lets a file and the calls share the node (see `transfer.rs`, feature `orion-r2`).
//!
//! Not here: a support bundle. Fetching one needs an Orion action that puts the bundle where a
//! receiver can take it, which the locked Orion rev does not have.

mod progress;
mod push;
mod source;
mod transfer;

pub use progress::{Progress, Stage};
pub use push::{KIND, PUSH_WAIT, Push, PushError, PushKind, submit};
pub use source::{Counters, PushSource};
