//! The one board driver. A board is an Orion node, whatever state it is in: running its own
//! software, waiting in recovery, or freshly loaded with the flasher image. Atlas reaches all
//! of them the same way, through Orion's client, and reads who each one is from its hello
//! document.
//!
//! - [`Seed`] says where a node is: a link and the Orion address on it.
//! - [`BoardDriver`] is the [`atlas_driver::Driver`]. A scan offers the nodes that are up; the
//!   driver starts one [`monitor::Monitor`] per node, which keeps its session on a thread of
//!   its own and follows the failure rule (see [`backoff`]).
//! - [`identity`] maps a hello document to an [`atlas_types::Identity`] and grants the
//!   capabilities the node offers.
//!
//! The driver depends on no other driver crate. Its tests run against Orion's own comm core
//! (`orion-stubs`) on a real socket and port.

pub mod backoff;
mod driver;
pub mod identity;
pub mod monitor;
mod reach;

pub use driver::{BoardDriver, FAMILY};
pub use monitor::{Handle, Pause, Presence, RealPause};
pub use reach::{LINK_KINDS, Seed};
