//! A node's hardware, readings and logs, read over its session as Orion resources and topics.
//!
//! What the app shows is what the node describes. A node lists each device as a resource with
//! labels and topics; its readings are the topics' fields, with their units; its health is its
//! status topic, with the reason. Nothing here knows a kind of device or a product: a device the
//! crate has never heard of shows its readings the same way. What the description does not give
//! (a human name, a unit, a group) is [`Reported::NotReported`], never a guess and never an empty
//! string.
//!
//! - [`Hardware`] is the reader: one per node, built on the node's [`atlas_session::Handle`].
//!   [`Hardware::snapshot`] makes a read and returns a [`Snapshot`].
//! - A read that cannot be made is a [`Connection::Stale`] with its reason; the devices shown are
//!   stale since the last read that was made. A device the node stops listing for a moment is
//!   held for [`DEVICE_GRACE`].
//! - The identity facts are [`atlas_types::Identity`], as the board driver makes them.
//!
//! This crate is for Linux, as Orion's topics are.

#![cfg(target_os = "linux")]

pub mod device;
pub mod hardware;
pub mod logs;
mod read;
pub mod value;

pub use device::{Availability, Control, Device, Freshness, Reading};
pub use hardware::{Connection, DEVICE_GRACE, Hardware, Snapshot};
pub use logs::{LOG_CAPACITY, LOG_TOPIC, LogLine};
pub use value::{ReadingKind, Reported, Scalar};
