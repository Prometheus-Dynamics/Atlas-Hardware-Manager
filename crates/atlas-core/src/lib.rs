//! Atlas product logic, shared by the desktop app and the `atlas` CLI.
//!
//! [`Atlas`] owns the inventory of known devices, scans links through the
//! registered drivers, runs update jobs, and publishes every state change on
//! one typed [`Event`] stream. Hosts are thin: they send intents and render
//! events.

mod activity;
mod atlas;
mod clock;
mod error;
mod events;
mod history;
pub mod install;
mod inventory;
mod job_runner;
mod jobs;
mod lineage;
mod manage;
mod observe;
mod push;
mod robots;
mod scan;
mod selftest;
mod store;
mod time;
mod watch;

pub use activity::{ActivityEntry, ActivityKind, ActivityLevel};
pub use atlas::{Atlas, AtlasBuilder, AtlasOptions};
pub use clock::ClockSync;
pub use error::CoreError;
pub use events::Event;
pub use history::{BOARD_EVENTS_LIMIT, BoardEventLog, HistoryEntry, HistoryOrigin};
pub use inventory::{DeviceRecord, Presence};
pub use jobs::{
    DeviceJobState, DeviceJobStatus, JobId, JobPlan, JobRecord, JobState, JobSummary,
    PlannedDevice, ReleaseTarget, StagedRollout, UpdateRequest,
};
pub use robots::{RobotProfile, RobotRole, RobotState, RobotStatus, RoleStatus};
pub use scan::ScanReport;
pub use selftest::{SelfTestRecord, SelfTestTrigger};
pub use store::{InventoryStore, JsonFileStore, MemoryStore, Snapshot, StoreError};
pub use watch::{DiscoveryStatus, WatchOptions};
