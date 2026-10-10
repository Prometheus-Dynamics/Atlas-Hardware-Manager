use atlas_driver::{DeviceKey, SelfTestStep, UpdateStep};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{
    ActivityEntry, DeviceJobStatus, DeviceRecord, JobId, JobState, JobSummary, ScanReport,
    SelfTestRecord,
};

/// Every state change in Atlas. Hosts render these; nothing else is needed
/// to keep a UI in sync.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Event {
    ScanStarted,
    /// A device was found or its details changed. `new` is true the first
    /// time this device is ever seen.
    DeviceSeen {
        record: Box<DeviceRecord>,
        new: bool,
    },
    DeviceOffline {
        key: DeviceKey,
    },
    /// A device was removed from the remembered inventory.
    DeviceForgotten {
        key: DeviceKey,
    },
    /// A robot profile was created, changed, or deleted, or a device in a
    /// robot changed version or presence. Re-read robot statuses.
    RobotsChanged,
    ScanWarning {
        message: String,
    },
    ScanFinished {
        report: ScanReport,
    },
    JobStarted {
        job: JobId,
        devices: Vec<DeviceKey>,
    },
    JobDevice {
        job: JobId,
        device: DeviceKey,
        status: DeviceJobStatus,
    },
    JobStep {
        job: JobId,
        device: DeviceKey,
        step: UpdateStep,
    },
    JobProgress {
        job: JobId,
        device: DeviceKey,
        step: UpdateStep,
        fraction: f32,
    },
    JobLog {
        job: JobId,
        device: DeviceKey,
        message: String,
    },
    JobFinished {
        job: JobId,
        state: JobState,
        summary: JobSummary,
    },
    /// Something worth a line in the fleet history.
    Activity {
        entry: ActivityEntry,
    },
    /// A self-test finished (or couldn't run); the board's latest result.
    SelfTest {
        record: Box<SelfTestRecord>,
    },
    /// A running self-test's progress: which checks, one starting, one done.
    SelfTestProgress {
        key: DeviceKey,
        #[serde(flatten)]
        step: SelfTestStep,
    },
    /// New events from the device's board: re-read its history.
    DeviceHistory {
        key: DeviceKey,
    },
    /// The device said its state changed (its push channel): re-read its
    /// status now.
    DeviceStatus {
        key: DeviceKey,
    },
}

#[derive(Clone)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self { sender }
    }

    pub(crate) fn emit(&self, event: Event) {
        // No subscribers is normal, for example in a one-shot CLI command.
        let _ = self.sender.send(event);
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}
