//! Simulated PD devices for tests, CI, and UI work without hardware.
//!
//! A [`MockFleet`] holds the simulated devices and their state. It hands out
//! one [`MockDriver`] per family and a [`LinkSource`](atlas_driver::LinkSource)
//! for a single simulated link. Devices can sit behind a simulated gateway,
//! start in recovery mode, and fail updates in chosen ways.

mod driver;
mod fleet;

pub use driver::MockDriver;
pub use fleet::{MockBehavior, MockDevice, MockFleet, SIM_LINK_ID};

/// Family name used for simulated HeliOS-like gateway devices.
pub const SIM_HELIOS: &str = "sim-helios";
/// Family name used for simulated microcontroller boards.
pub const SIM_MCU: &str = "sim-mcu";
