//! Orion as a capability source.
//!
//! Orion is the management agent on running devices (control protocol v5).
//! Atlas doesn't treat an Orion node as a device of its own: the board is
//! found and keyed by its identity driver (for a Raze, the board driver over
//! mDNS), and when an Orion node reports the same board serial, this crate
//! adds what Orion offers to that device: live readings from the node's
//! status lane, the actions the on-device agent claims (`locate`, `reboot`),
//! A/B updates (see docs/ota.md), and the board's state (boot, readings,
//! the `update.*` keys), the board's devices (Lemnos's `lemnos.device`
//! resources: readings, and commands to their controls) for boards that don't report it themselves. Without Orion, devices keep everything
//! else; Orion is never the only way in.
//!
//! [`LiveStreams`] follows a board's live readings, pushed by the board's
//! own orion-node (board-agent's `board.live` stream); given to the
//! directory, it also opens the board's status push channel there.
//!
//! The transport is a trait. [`RemoteTransport`] implements it with Orion's
//! remote operator client (signed orion+tcp).

mod actions;
mod board_nodes;
mod control;
mod directory;
mod hardware;
mod live;
mod metrics;
mod operator;
mod peer_tunnel;
mod remote;
mod slots;
mod status;
mod transport;
mod update;

pub use board_nodes::BoardNodes;
pub use directory::OrionDirectory;
pub use live::{LiveFrame, LiveRow, LiveStreams};
pub use operator::operator_identity;
pub use orion_client::remote::OperatorIdentity;
pub use remote::{OrionConnection, RemoteTransport};
pub use transport::{BundleHost, OrionTransport};
