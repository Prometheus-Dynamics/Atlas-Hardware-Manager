//! Orion as a capability source.
//!
//! Orion is the management agent on running devices (control protocol v4).
//! Atlas doesn't treat an Orion node as a device of its own: the board is
//! found and keyed by its identity driver (for a Raze, the PD driver over
//! mDNS), and when an Orion node reports the same board serial, this crate
//! adds what Orion offers to that device: live readings from the node's
//! status lane, the actions the on-device agent claims (`locate`, `reboot`),
//! and A/B updates (see docs/ota.md). Without Orion, devices keep everything
//! else; Orion is never the only way in.
//!
//! The transport is a trait. Orion's remote operator client (signed
//! orion+tcp) is the intended implementation; until it lands, only tests use
//! one, and the app registers no directory.

mod actions;
mod directory;
mod metrics;
mod transport;
mod update;

pub use directory::OrionDirectory;
pub use transport::{BundleHost, OrionTransport};
