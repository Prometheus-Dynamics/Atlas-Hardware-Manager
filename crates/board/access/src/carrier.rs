//! The carrier: the one place bytes reach the node's shell and its key list.
//!
//! // Orion at dbd5fc08 has the calls this trait needs (`Orion::shell` for a terminal, Orion C27,
//! // and `Orion::enroll` for an operator key, Orion D13). No production carrier is built here yet:
//! // the trait is `Send + Sync` and blocking, while Orion's `Shell` and enrol live on the client
//! // handle, which is not `Send` and stays on the session thread. A production carrier needs a
//! // long operation on that thread (`Handle::long`) that pumps the shell and takes its input from a
//! // queue. That is a PR of its own. Until then the only carrier is the simulated one in the tests.
//!
//! Calls on a carrier block. The crate runs them on a blocking thread, never on the async
//! runtime. A carrier is never asked to run a command: it opens an interactive shell or
//! installs a key, and nothing else.

use std::fmt;
use std::sync::Arc;

use crate::key::PublicKey;
use crate::kind::ShellKind;

/// What a shell is opened as, and the terminal it is opened on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellRequest {
    /// Whether this is the normal shell or the recovery shell, as the node said.
    pub kind: ShellKind,
    /// The terminal's width in characters.
    pub cols: u16,
    /// The terminal's height in characters.
    pub rows: u16,
    /// The terminal's kind, as `TERM` names it.
    pub term: String,
}

/// The way in to a node's shell and key list.
pub trait Carrier: Send + Sync + fmt::Debug {
    /// Opens an interactive shell as `request` says. Blocking.
    ///
    /// # Errors
    ///
    /// The words of why the node or the link refused.
    fn open_shell(&self, request: &ShellRequest) -> Result<Channel, String>;

    /// Installs one public key on the node. Blocking. Called only with a key that
    /// [`PublicKey::parse`] accepted.
    ///
    /// # Errors
    ///
    /// The words of why the node refused.
    fn install_key(&self, key: &PublicKey) -> Result<(), String>;
}

/// An open shell: the way to send to it, and its output.
#[derive(Debug)]
pub struct Channel {
    /// Sends to the shell and resizes it, and hangs it up.
    pub control: Arc<dyn Control>,
    /// Its output, read until it ends.
    pub output: Box<dyn Output>,
}

/// What a person does to an open shell. Implementations do not block for long: a call on a
/// shell whose link is wedged may return an error instead of waiting.
pub trait Control: Send + Sync + fmt::Debug {
    /// Sends bytes to the shell's input.
    ///
    /// # Errors
    ///
    /// The words of why the bytes could not be sent.
    fn send(&self, bytes: &[u8]) -> Result<(), String>;

    /// Tells the shell its terminal is now this size.
    ///
    /// # Errors
    ///
    /// The words of why the size could not be sent.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;

    /// Ends the link to the shell. Output that is read afterwards ends.
    fn hang_up(&self);
}

/// What an open shell's output says next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputEvent {
    /// Bytes the shell printed.
    Data(Vec<u8>),
    /// The shell ended with this status.
    Exit(i32),
    /// The link broke, with its words. No more output follows.
    Broke(String),
}

/// The output of an open shell, read blocking, one event at a time.
pub trait Output: Send + fmt::Debug {
    /// The next event. Blocks until there is one. After [`OutputEvent::Exit`] or
    /// [`OutputEvent::Broke`] it is not called again.
    fn next(&mut self) -> OutputEvent;
}
