//! The contract between Atlas and a device family.
//!
//! A driver tells Atlas three things: how to find its devices on a [`Link`],
//! who each device is ([`Identity`]), and which [`Capabilities`] it offers.
//! Everything above this crate works only with those types, so a new device
//! family is added by writing a driver, not by changing the core or the UI.

mod augment;
mod bundle;
mod capability;
pub mod controller_peer;
mod driver;
mod error;
mod hardware;
mod health;
mod install;
mod observe;
mod registry;
pub mod robot_net;
mod selftest;
mod slots;
mod status;
mod trial;
mod types;
pub mod util;

pub use augment::CapabilitySource;
pub use bundle::{BundleHost, download_failed};
pub use capability::{
    ActionsCapability, Artifact, Capabilities, CapabilityKind, Concurrency, DeviceAction,
    ProgressSink, ProgressUpdate, ReleaseRef, UpdateCapability, UpdateOutcome, UpdatePlan,
    UpdateStep,
};
pub use driver::{ChangeNotifier, Driver, DriverManifest, LinkSource};
pub use error::DriverError;
pub use hardware::{
    CalibrationPart, CalibrationStatus, CalibrationStep, FrameSink, HardwareCapability,
    HardwareCommand, HardwareFrame, LiveChannel, LiveDevice,
};
pub use health::{HealthCheck, HealthStatus};
pub use install::{
    ArtifactKind, ChangeTracker, DeviceChannel, InstallFailure, InstallMethod, InstallMethodId,
    InstallOptions, InstallPlan, InstallRequest,
};
pub use observe::{
    LogLevel, LogLine, LogsCapability, Metric, TelemetryCapability, attributes, metric_ids,
};
pub use registry::DriverRegistry;
pub use selftest::{
    CheckStatus, SELFTEST_FORMAT, SelfTestCapability, SelfTestCheck, SelfTestReport, SelfTestStep,
};
pub use slots::{
    LayerPush, LayerPushed, PushMode, Slot, SlotCommand, SlotInstall, SlotLayer, SlotSet,
    SlotsCapability, refusal,
};
pub use status::{
    BootInfo, DeviceEvent, DeviceStatus, Drift, DriftItem, EventPage, EventQuery, EventSource,
    FanState, HardwareControl, HardwareDevice, HardwareReading, HardwareSnapshot, KernelRecord,
    PushSink, StatusCapability, StatusPush, Temperature, UpdateState,
};
pub use trial::{TrialStop, trial_stopped};
pub use types::{
    Candidate, DeviceKey, DeviceMode, Family, Identity, Link, LinkId, LinkKind, Serial,
};

/// Cancellation handle passed to long-running driver operations.
pub use tokio_util::sync::CancellationToken;
