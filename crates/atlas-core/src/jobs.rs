use std::collections::BTreeMap;
use std::fmt;

use atlas_driver::{DeviceKey, Family, ReleaseRef, UpdatePlan, UpdateStep};
use serde::{Deserialize, Serialize};

/// Log lines kept per device in a job; older lines are dropped first.
pub(crate) const DEVICE_LOG_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl JobId {
    /// Accepts `job-3` or `3`.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.trim().strip_prefix("job-").unwrap_or(text.trim());
        digits.parse().ok().map(Self)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

/// Whether to update one device of a family first and stop if it fails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StagedRollout {
    /// On when a family has at least the configured threshold of devices.
    #[default]
    Auto,
    On,
    Off,
}

/// Update these devices to the chosen release of their family.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub devices: Vec<DeviceKey>,
    /// Target version per family.
    pub releases: BTreeMap<Family, String>,
    pub staged: StagedRollout,
}

/// One device's part of a job, as shown before the user confirms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedDevice {
    pub device: DeviceKey,
    pub name: String,
    pub from_version: Option<String>,
    pub release: ReleaseRef,
    pub plan: UpdatePlan,
    /// Updated alone first; the rest of its family waits for it to verify.
    pub canary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobPlan {
    pub devices: Vec<PlannedDevice>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum DeviceJobStatus {
    Queued,
    Running,
    Verified { version: String },
    RolledBack { reason: String },
    NeedsRecovery { reason: String },
    Failed { error: String },
    Skipped { reason: String },
    Cancelled,
}

impl DeviceJobStatus {
    pub fn is_finished(&self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }

    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Verified { .. } => "verified",
            Self::RolledBack { .. } => "rolled back",
            Self::NeedsRecovery { .. } => "needs recovery",
            Self::Failed { .. } => "failed",
            Self::Skipped { .. } => "skipped",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceJobState {
    pub device: DeviceKey,
    pub name: String,
    pub release: ReleaseRef,
    pub plan: UpdatePlan,
    pub canary: bool,
    pub status: DeviceJobStatus,
    pub step: Option<UpdateStep>,
    /// Progress within the current step, 0.0 to 1.0.
    pub fraction: f32,
    pub log: Vec<String>,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
}

impl DeviceJobState {
    pub(crate) fn push_log(&mut self, line: String) {
        if self.log.len() >= DEVICE_LOG_LIMIT {
            self.log.remove(0);
        }
        self.log.push(line);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobState {
    Running,
    Finished,
    Cancelled,
}

/// Counts of how each device ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub verified: usize,
    pub rolled_back: usize,
    pub needs_recovery: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cancelled: usize,
}

impl JobSummary {
    pub(crate) fn from_devices(devices: &[DeviceJobState]) -> Self {
        let mut summary = Self::default();
        for device in devices {
            match device.status {
                DeviceJobStatus::Verified { .. } => summary.verified += 1,
                DeviceJobStatus::RolledBack { .. } => summary.rolled_back += 1,
                DeviceJobStatus::NeedsRecovery { .. } => summary.needs_recovery += 1,
                DeviceJobStatus::Failed { .. } => summary.failed += 1,
                DeviceJobStatus::Skipped { .. } => summary.skipped += 1,
                DeviceJobStatus::Cancelled => summary.cancelled += 1,
                DeviceJobStatus::Queued | DeviceJobStatus::Running => {}
            }
        }
        summary
    }

    /// True when every device verified.
    pub fn all_verified(&self) -> bool {
        self.rolled_back + self.needs_recovery + self.failed + self.skipped + self.cancelled == 0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: JobId,
    pub state: JobState,
    pub created_ms: u64,
    pub finished_ms: Option<u64>,
    pub devices: Vec<DeviceJobState>,
    pub summary: Option<JobSummary>,
}

impl JobRecord {
    pub(crate) fn device_mut(&mut self, key: &DeviceKey) -> Option<&mut DeviceJobState> {
        self.devices.iter_mut().find(|device| &device.device == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_parses_both_forms() {
        assert_eq!(JobId::parse("job-7"), Some(JobId(7)));
        assert_eq!(JobId::parse("7"), Some(JobId(7)));
        assert_eq!(JobId::parse("job-x"), None);
        assert_eq!(JobId(7).to_string(), "job-7");
    }
}
