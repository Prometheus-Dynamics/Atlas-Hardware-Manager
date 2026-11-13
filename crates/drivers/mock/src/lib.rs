//! Simulated devices and the simulated node, for tests and UI work.
//!
//! - [`device`]: one simulated device in every `DeviceState`, behind a driver the UI lists.
//!   Devices can be taken off the link and brought back.
//! - [`node`]: a simulated board. It is an Orion comm core in this process (Orion's own
//!   `orion-stubs`), with node logic that answers the board's actions, topics and files as
//!   a real board does. Linux only, as Orion is.
//! - [`slots`]: the root slot rules of Lemnos's slot driver; [`bundle`]: the slot bundle.
//! - [`model`]: the board's state (slots, data partition, clock, readings).
//! - [`script`]: scripted failures: refuse, fail, stall, and link faults.
//! - [`names`]: the one table of names the node answers to.
//!
//! `testing/sim-node/README.md` maps each C-series brief to the names it uses.

pub mod bundle;
pub mod device;
pub mod model;
pub mod names;
#[cfg(target_os = "linux")]
pub mod node;
pub mod script;
pub mod slots;

pub use device::{MockDevice, MockDriver, MockFleet, SIM_FAMILY, SIM_LINK};
#[cfg(target_os = "linux")]
pub use node::{NODE_NAME, SimNode, call};
pub use script::{Behaviour, LinkFault, Script};
pub use slots::{Slots, Status};
