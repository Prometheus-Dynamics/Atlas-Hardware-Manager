//! The one board driver. A board is an Orion node, whatever state it is in: running its own
//! software, waiting in recovery, or freshly loaded with the flasher image. Atlas reaches all
//! of them the same way, through Orion's client: a node is detected by its Identify answer
//! (the signed hello document), never by a bare connect.
//!
//! - [`atlas_session::Seed`] says where a node is: a link and the Orion address on it.
//! - [`discovery::addresses_on`] is the one place a scan looks; it is the caller's addresses
//!   until Orion's discovery lands.
//! - [`BoardDriver`] is the [`atlas_driver::Driver`], serving every family. The connection to
//!   each node (its monitor, the failure rule, the manual off) is [`atlas_session`]'s; the
//!   driver only asks it for a node's handle.
//! - [`identity`] maps a hello document and Lemnos's state to an [`atlas_types::Identity`] and
//!   grants the capabilities the node offers. [`lemnos`] reads Lemnos's state into
//!   [`atlas_types::LemnosState`].
//!
//! The driver depends on no other driver crate. Its tests run against Orion's comm core and
//! the simulated node (`atlas-sim-node`) on a real socket and port.

pub mod discovery;
mod driver;
pub mod identity;
pub mod lemnos;

pub use driver::{BoardDriver, FAMILY, LINK_KINDS};
