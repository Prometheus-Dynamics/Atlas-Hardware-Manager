//! The contract between Atlas and a device family.
//!
//! A driver tells Atlas three things: how to find its devices on a [`Link`],
//! who each device is ([`Identity`]), and which [`Capabilities`] it offers.
//! Everything above this crate works only with those types, so a new device
//! family is added by writing a driver, not by changing the core or the UI.

mod augment;
mod capability;
mod driver;
mod error;
mod hardware;
mod health;
mod observe;
mod registry;
mod selftest;
mod status;
mod types;

pub use augment::CapabilitySource;
pub use capability::{
    ActionsCapability, Artifact, Capabilities, CapabilityKind, Concurrency, DeviceAction,
    ProgressSink, ProgressUpdate, ReleaseRef, UpdateCapability, UpdateOutcome, UpdatePlan,
    UpdateStep,
};
pub use driver::{ChangeNotifier, Driver, DriverManifest, LinkSource};
pub use error::DriverError;
pub use hardware::{HardwareCapability, HardwareCommand};
pub use health::{HealthCheck, HealthStatus};
pub use observe::{
    LogLevel, LogLine, LogsCapability, Metric, TelemetryCapability, attributes, metric_ids,
};
pub use registry::DriverRegistry;
pub use selftest::{
    CheckStatus, SELFTEST_FORMAT, SelfTestCapability, SelfTestCheck, SelfTestReport,
};
pub use status::{
    BootInfo, DeviceEvent, DeviceStatus, Drift, DriftItem, EventSource, FanState, HardwareControl,
    HardwareDevice, HardwareReading, HardwareSnapshot, StatusCapability, Temperature, UpdateState,
};
pub use types::{
    Candidate, DeviceKey, DeviceMode, Family, Identity, Link, LinkId, LinkKind, Serial,
};

/// Cancellation handle passed to long-running driver operations.
pub use tokio_util::sync::CancellationToken;
