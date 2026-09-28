//! Atlas product logic, shared by the desktop app and the `atlas` CLI.
//!
//! [`Atlas`] owns the inventory of known devices, scans links through the
//! registered drivers, runs update jobs, and publishes every state change on
//! one typed [`Event`] stream. Hosts are thin: they send intents and render
//! events.

mod atlas;
mod error;
mod events;
mod inventory;
mod job_runner;
mod jobs;
mod manage;
mod robots;
mod scan;
mod store;
mod time;

pub use atlas::{Atlas, AtlasBuilder, AtlasOptions};
pub use error::CoreError;
pub use events::Event;
pub use inventory::{DeviceRecord, Presence};
pub use jobs::{
    DeviceJobState, DeviceJobStatus, JobId, JobPlan, JobRecord, JobState, JobSummary,
    PlannedDevice, ReleaseTarget, StagedRollout, UpdateRequest,
};
pub use robots::{RobotProfile, RobotRole, RobotState, RobotStatus, RoleStatus};
pub use scan::ScanReport;
pub use store::{InventoryStore, JsonFileStore, MemoryStore, Snapshot, StoreError};
