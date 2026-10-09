use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    DeviceMode, DriverError, Family, Identity, LogsCapability, SelfTestCapability,
    StatusCapability, TelemetryCapability,
};

/// Everything a device can offer. The UI shows a tab or action only for the
/// kinds a device reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    Info,
    Update,
    Recover,
    Config,
    Logs,
    Telemetry,
    Actions,
    Gateway,
    OpenUi,
    /// The device can check its own hardware.
    SelfTest,
    /// The device reports its state (boot, health, update, drift) and,
    /// usually, an event log.
    Status,
}

/// The capabilities a driver grants one device. Absent means unsupported.
#[derive(Clone, Default)]
pub struct Capabilities {
    pub update: Option<Arc<dyn UpdateCapability>>,
    pub actions: Option<Arc<dyn ActionsCapability>>,
    pub telemetry: Option<Arc<dyn TelemetryCapability>>,
    pub logs: Option<Arc<dyn LogsCapability>>,
    pub selftest: Option<Arc<dyn SelfTestCapability>>,
    pub status: Option<Arc<dyn StatusCapability>>,
}

impl Capabilities {
    /// The capability kinds this device supports, `Info` always first.
    pub fn kinds(&self) -> Vec<CapabilityKind> {
        self.kinds_for(DeviceMode::Normal)
    }

    /// Like [`kinds`](Self::kinds), but a device in recovery mode reports
    /// `Recover` instead of `Update`: the same capability, a different promise.
    pub fn kinds_for(&self, mode: DeviceMode) -> Vec<CapabilityKind> {
        let mut kinds = vec![CapabilityKind::Info];
        if self.update.is_some() {
            kinds.push(match mode {
                DeviceMode::Normal => CapabilityKind::Update,
                DeviceMode::Recovery => CapabilityKind::Recover,
            });
        }
        if self.actions.is_some() {
            kinds.push(CapabilityKind::Actions);
        }
        if self.telemetry.is_some() {
            kinds.push(CapabilityKind::Telemetry);
        }
        if self.logs.is_some() {
            kinds.push(CapabilityKind::Logs);
        }
        if self.selftest.is_some() && mode == DeviceMode::Normal {
            kinds.push(CapabilityKind::SelfTest);
        }
        if self.status.is_some() && mode == DeviceMode::Normal {
            kinds.push(CapabilityKind::Status);
        }
        kinds
    }
}

impl fmt::Debug for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Capabilities")
            .field("kinds", &self.kinds())
            .finish()
    }
}

/// A local file to install, already checked against its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// File name shown to users, for example `helios-2026.3.1-cm5.img.xz`.
    pub name: String,
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the file at `path`.
    pub sha256: String,
    pub size_bytes: u64,
}

/// A release a device can be moved to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseRef {
    pub family: Family,
    pub version: String,
    /// The file to install. Drivers that write images or firmware require
    /// it; drivers whose devices fetch their own updates may ignore it.
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

/// The named steps every update reports, whatever the device family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateStep {
    /// Checks that change nothing: compatibility, free space, battery, mode.
    Preflight,
    /// Moving the artifact to the device.
    Transfer,
    /// Writing or staging the new version on the device.
    Apply,
    /// Restarting into the new version.
    Reboot,
    /// Waiting for the device to report the new version as healthy.
    Confirm,
}

impl UpdateStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Transfer => "transfer",
            Self::Apply => "apply",
            Self::Reboot => "reboot",
            Self::Confirm => "confirm",
        }
    }
}

/// Whether an update can run alongside others.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "resource")]
pub enum Concurrency {
    /// The device updates itself; any number can run at once.
    Parallel,
    /// Needs a host resource one device at a time, for example a USB controller.
    Exclusive(String),
}

/// What an update will do, shown to the user before they confirm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePlan {
    pub steps: Vec<UpdateStep>,
    pub concurrency: Concurrency,
    /// One sentence, for example `OTA 2026.2.4 to 2026.3.1; the device reboots`.
    pub summary: String,
}

/// How an update ended. A driver error means it failed before changing anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "result")]
pub enum UpdateOutcome {
    /// The device came back reporting the new version.
    Verified { version: String },
    /// The device returned to its previous version on its own.
    RolledBack { reason: String },
    /// The device is not usable and needs a recovery update.
    NeedsRecovery { reason: String },
}

/// A progress report from a running update.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum ProgressUpdate {
    StepStarted {
        step: UpdateStep,
    },
    /// `fraction` runs from 0.0 to 1.0 within the step.
    StepProgress {
        step: UpdateStep,
        fraction: f32,
    },
    Log {
        message: String,
    },
}

/// Where a driver sends progress. Cheap to clone.
#[derive(Clone)]
pub struct ProgressSink(Arc<dyn Fn(ProgressUpdate) + Send + Sync>);

impl ProgressSink {
    pub fn new(report: impl Fn(ProgressUpdate) + Send + Sync + 'static) -> Self {
        Self(Arc::new(report))
    }

    /// A sink that drops every report.
    pub fn discard() -> Self {
        Self::new(|_| {})
    }

    pub fn step_started(&self, step: UpdateStep) {
        (self.0)(ProgressUpdate::StepStarted { step });
    }

    pub fn step_progress(&self, step: UpdateStep, fraction: f32) {
        (self.0)(ProgressUpdate::StepProgress {
            step,
            fraction: fraction.clamp(0.0, 1.0),
        });
    }

    pub fn log(&self, message: impl Into<String>) {
        (self.0)(ProgressUpdate::Log {
            message: message.into(),
        });
    }
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProgressSink")
    }
}

/// Moves a device to a release, from a normal or a recovery state.
#[async_trait]
pub trait UpdateCapability: Send + Sync {
    /// Describes the update without touching the device.
    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError>;

    /// Runs the update. Must stop promptly when `cancel` fires, as long as
    /// nothing irreversible has started.
    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError>;
}

/// A named one-shot action such as locate, reboot, or factory reset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAction {
    pub id: String,
    pub label: String,
    /// Destructive actions need an explicit confirm in the UI.
    pub destructive: bool,
}

#[async_trait]
pub trait ActionsCapability: Send + Sync {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction>;

    /// Whether this source runs the actions it shares with the device's own
    /// driver (a management agent such as Orion), with the driver's as the
    /// fallback when it can't be reached. Default: the driver's run.
    fn preferred(&self) -> bool {
        false
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError>;
}
