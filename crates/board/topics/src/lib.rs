//! A node's hardware, readings and logs, read over its session as Orion resources and topics.
//!
//! What the app shows is what the node describes, in Lemnos V1a's shape (see [`device`]): a label,
//! a bus, a state (running, retrying or off) with its reason, an address, a kind per contract, and
//! per item a reading kind, a unit, a range, a step, warning limits, a group (a three-axis vector),
//! and whether it is settable with its setter. Nothing here knows a kind of device or a product.
//! Anything the description leaves out is [`Reported::NotReported`]: never a guess, never an empty
//! string, and never a reason to hide the device.
//!
//! - [`Hardware`] is the reader: one per node, built on the node's [`atlas_session::Handle`].
//!   [`Hardware::snapshot`] makes a read and returns a [`Snapshot`].
//! - A read that cannot be made is a [`Connection::Stale`] with its reason; the devices shown are
//!   stale since the last read that was made. A device the node stops listing for a moment is
//!   held for [`DEVICE_GRACE`].
//! - A device that is retrying is a state of the device with its reason: it stays listed and
//!   current, and is not a fault of the node.
//! - The identity facts are [`atlas_types::Identity`], as the board driver makes them.
//!
//! This crate is for Linux, as Orion's topics are.

#![cfg(target_os = "linux")]

pub mod device;
pub mod hardware;
pub mod logs;
mod read;
pub mod value;

pub use device::{
    Availability, Contract, Control, Device, DeviceState, Freshness, Range, Reading, Vector,
};
pub use hardware::{Connection, DEVICE_GRACE, Hardware, Snapshot};
pub use logs::{LOG_CAPACITY, LOG_TOPIC, LogLine};
pub use value::{ReadingKind, Reported, Scalar};
