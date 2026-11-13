//! Simulated devices, one in every [`DeviceState`](atlas_types::DeviceState), behind a driver
//! the UI can list.
//!
//! - [`device`]: the devices, the fleet they sit in, and the driver that offers them. Devices
//!   can be taken off the link and brought back.
//!
//! The simulated Orion node (what a board publishes, its actions, slots and files) is a
//! testing crate of its own: `atlas-sim-node`, under `crates/testing/sim-node`. A driver's
//! tests use it as a dev-dependency; this crate needs nothing from it.

pub mod device;

pub use device::{MockDevice, MockDriver, MockFleet, SIM_FAMILY, SIM_LINK};
