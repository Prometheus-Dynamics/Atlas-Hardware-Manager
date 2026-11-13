//! A node's hardware, readings and logs, read over its session as Orion resources and topics.
//!
//! What the app shows is what the node describes. A device's description is the text of the
//! bridge's `call describe` (Lemnos `describe 1`, see [`description`]): the device record (label,
//! bus, state and reason, address, location, kind, generation), its contracts, its methods and
//! its items (reading, unit and unit state, range, step, warning limits, group, control). It is
//! read whole and refused whole when it is not a description this crate understands. Anything the
//! description leaves out is [`Reported::NotReported`]: never a guess, never an empty string, and
//! never a reason to hide the device. A board that does not give a description is a state of the
//! device ([`description::DescriptionState::NotAvailable`]), not a fault of the node.
//!
//! - [`Hardware`] is the reader: one per node, built on the node's [`atlas_session::Handle`].
//!   [`Hardware::snapshot`] makes a read and returns a [`Snapshot`]; [`Hardware::refresh_descriptions`]
//!   asks every description again on the next read.
//! - A description is asked for once per connection and again only when the device's description
//!   generation moves (see [`describe`]).
//! - A read that cannot be made is a [`Connection::Stale`] with its reason; the devices shown are
//!   stale since the last read that was made. A device the node stops listing for a moment is
//!   held for [`DEVICE_GRACE`].
//! - A device that is retrying is a state of the device with its reason: it stays listed and
//!   current, and is not a fault of the node.
//! - The identity facts are [`atlas_types::Identity`], as the board driver makes them.
//!
//! This crate is for Linux, as Orion's topics are.

#![cfg(target_os = "linux")]

pub mod describe;
pub mod description;
pub mod device;
pub mod hardware;
pub mod logs;
mod read;
pub mod value;

pub use describe::DESCRIBE_WAIT;
pub use description::{
    Description, DescriptionState, MAX_DESCRIPTION_BYTES, Refusal, VERSION_LINE,
};
pub use device::{
    Availability, Contract, Control, Device, DeviceState, Freshness, Range, Reading, Vector,
};
pub use hardware::{Connection, DEVICE_GRACE, Hardware, Snapshot};
pub use logs::{LOG_CAPACITY, LOG_TOPIC, LogLine};
pub use value::{ItemKind, ReadingKind, Reported, Scalar, UnitState};
