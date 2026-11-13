//! The contract between Atlas and a device family.
//!
//! A driver tells Atlas three things: how to find its devices on a [`Link`],
//! who each device is ([`Identity`]), and which capabilities it grants
//! ([`CapabilitySet`]). Everything above this crate works only with those
//! types, so a new family is a new driver, not a change to the core.
//!
//! A driver serves one family, or every family ([`Serves::Any`]) for a
//! board-like driver that finds any device on its links. The registry says
//! which driver serves a device; see [`DriverRegistry::resolve`].
//!
//! No driver depends on another driver. The rule is checked by the
//! `driver_isolation` test.

mod driver;
mod error;
mod health;
mod registry;

pub use driver::{Driver, DriverManifest, Serves};
pub use error::DriverError;
pub use health::{HealthCheck, HealthStatus};
pub use registry::{DriverRegistry, RegisterError};
