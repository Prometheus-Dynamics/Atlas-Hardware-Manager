//! The simulated Orion node: what a board publishes, answered the way a board answers.
//!
//! - [`node`]: the node. It is an Orion comm core in this process (Orion's own `orion-stubs`),
//!   with node logic that publishes the hello facts a board publishes, registers the I5 slot
//!   and reboot surface, the board's own actions, topics and files, and answers each run. It
//!   can be stopped and started again on the same port. Linux only, as Orion is.
//! - [`slots`]: the root slot rules of Lemnos's slot driver (I5). [`bundle`]: the slot bundle
//!   and its signature check. [`model`]: the board's state (slots, data partition, clock,
//!   readings).
//! - [`script`]: scripted failures: refuse, fail, stall, restart, and link faults.
//! - [`names`]: the one table of names the node answers to and publishes.
//!
//! `testing/sim-node/README.md` says what the node does not do, and maps each C-series brief
//! to the names it uses. The mock devices that drivers list are in `atlas-mock`.

pub mod access;
pub mod bundle;
pub mod model;
pub mod names;
#[cfg(target_os = "linux")]
pub mod node;
pub mod script;
pub mod slots;

#[cfg(target_os = "linux")]
pub use node::{DEADLINE, Hello, Kind, Mode, NODE_NAME, SimNode, call};
pub use script::{Behaviour, LinkFault, Script};
pub use slots::{Slots, Status};
