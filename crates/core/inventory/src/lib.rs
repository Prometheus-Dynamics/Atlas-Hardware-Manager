//! The inventory: one record per board, however many states and links it is
//! seen through.
//!
//! - A board is keyed by its [`DeviceKey`]. Its running, recovery, boot ROM and
//!   removable-disk sightings all land on that one record, and so do its links
//!   (USB network, Ethernet, behind a controller). The record follows one link
//!   at a time (the best one that is live) and keeps every link it was seen on.
//! - Every change of state, link or version is written to the record's history.
//! - Robots group devices, and robots group into teams.
//! - Health is a reading of a record at a time: a device that disappears while
//!   its robot is power cycled, and returns with it, is not a fault.
//! - [`JsonFileStore`] writes the inventory crash-safely (temp file, sync,
//!   rename). A file that cannot be read or used is moved aside and the
//!   inventory starts empty, so the app always opens.
//!
//! Time is passed in as milliseconds by the caller, so nothing here reads a
//! clock and every rule is testable without waiting.

mod health;
mod history;
mod inventory;
mod persist;
mod record;
mod robots;

pub use health::{FAULT_AFTER_MS, Health, POWER_CYCLE_LEAD_MS, RETURN_GRACE_MS};
pub use history::{HISTORY_LIMIT, HistoryEntry, HistoryKind};
pub use inventory::{Inventory, Outcome, PowerCycle};
pub use persist::{FORMAT_VERSION, JsonFileStore, LoadOutcome};
pub use record::{DeviceRecord, LinkSighting, Presence};
pub use robots::Robot;
