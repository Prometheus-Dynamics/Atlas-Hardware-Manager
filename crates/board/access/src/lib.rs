//! SSH and the recovery shell, for a person.
//!
//! Two things a person can do on a board, through the node's Orion session ([`atlas_session`]):
//!
//! - [`open`] a terminal: an interactive byte stream with resize and an exit status. It is the
//!   normal shell of a node that runs its own software, or the recovery shell of a node in
//!   recovery or a flasher image. Which one is read from the node's own hello `state` fact, and
//!   a node that says nothing about its state gets no shell.
//! - [`install_key`]: install one OpenSSH ed25519 public key on the node. The line is checked in
//!   form first; a private key is never read.
//!
//! The board is identified by its Orion node key from the signed hello. A terminal or a key
//! install never asks a question about a known-hosts file: a node whose key is not the one the
//! caller knew is refused, and never accepted silently.
//!
//! No board job is ever done through this crate. There is no call that runs a command and
//! returns its output: a terminal is the only interactive stream, and it is for a person.
//!
//! The way to the shell and the key list is a [`Carrier`]. Orion has the calls for either now (see
//! [`carrier`]); the crate ships no production carrier yet, and tests drive it through the
//! simulated node.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use atlas_access::{Carrier, Event, OpenRequest, open};
//! use atlas_session::Handle;
//!
//! # async fn demo(handle: &Handle, carrier: Arc<dyn Carrier>) -> Result<(), atlas_access::AccessError> {
//! let mut terminal = open(handle, carrier, OpenRequest::new(120, 40)).await?;
//! println!("shell on {} ({:?})", terminal.identity().fingerprint, terminal.kind());
//! terminal.send(b"uptime\n").await?;
//! while let Some(event) = terminal.recv().await {
//!     match event {
//!         Event::Output(bytes) => print!("{}", String::from_utf8_lossy(&bytes)),
//!         Event::Ended(end) => println!("\nended: {end:?}"),
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod carrier;
mod error;
mod install;
mod key;
mod kind;
mod terminal;

pub use carrier::{Carrier, Channel, Control, Output, OutputEvent, ShellRequest};
pub use error::AccessError;
pub use install::{Installed, install_key};
pub use key::PublicKey;
pub use kind::{Identity, NodeState, ShellKind, state_of};
pub use terminal::{End, Event, OpenRequest, Terminal, open};
