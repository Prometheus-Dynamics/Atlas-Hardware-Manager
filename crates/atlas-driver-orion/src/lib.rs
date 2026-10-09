//! Orion as a capability source.
//!
//! Orion is the management agent on running devices (control protocol v4).
//! Atlas doesn't treat an Orion node as a device of its own: the board is
//! found and keyed by its identity driver (for a Raze, the board driver over
//! mDNS), and when an Orion node reports the same board serial, this crate
//! adds what Orion offers to that device: live readings from the node's
//! status lane, the actions the on-device agent claims (`locate`, `reboot`),
//! A/B updates (see docs/ota.md), and the board's state (boot, readings,
//! the `update.*` keys) for boards that don't report it themselves. Without Orion, devices keep everything
//! else; Orion is never the only way in.
//!
//! The transport is a trait. [`RemoteTransport`] implements it with Orion's
//! remote operator client (signed orion+tcp).

mod actions;
mod directory;
mod metrics;
mod remote;
mod status;
mod transport;
mod update;

pub use directory::OrionDirectory;
pub use orion_client::remote::OperatorIdentity;
pub use remote::{OrionConnection, RemoteTransport};
pub use transport::{BundleHost, OrionTransport};
