//! NetworkTables for Atlas, on Orion's NT4 crate (`orion-nt4`).
//!
//! * [`Viewer`] connects to an NT4 server (a robot's, a camera's) and keeps its topics: names,
//!   types, [`Properties`] (the `persistent` and `retained` flags) and last [`Sample`]s. Each
//!   change comes out of [`Viewer::next`] as an [`Update`]. [`Viewer::edit`] writes a value to an
//!   announced topic, and refuses one whose type does not fit it.
//! * [`LocalServer`] is the NT4 server Atlas runs itself, for bench use: its topics are the
//!   host's, and what clients write comes out of [`LocalServer::next_event`] as a [`ServerUpdate`].
//!
//! Every call that does I/O is async and runs on the caller's Tokio runtime; nothing blocks.
//! A viewer reconnects on its own, with backoff capped at [`RECONNECT_MAX`], until it is dropped.
//! Dropping a [`Viewer`] or a [`LocalServer`] stops its tasks and closes its connections.

mod error;
mod server;
mod target;
mod topic;
mod viewer;

pub use error::Error;
pub use orion_nt4::{DEFAULT_PORT, Properties, Value};
pub use server::{LocalServer, LocalServerConfig, ServerUpdate};
pub use target::team_host;
pub use topic::{Sample, Topic, Update};
pub use viewer::{RECONNECT_MAX, Viewer, ViewerConfig};
