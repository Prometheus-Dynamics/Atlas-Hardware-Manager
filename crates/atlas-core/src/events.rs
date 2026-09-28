use atlas_driver::{DeviceKey, UpdateStep};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{DeviceJobStatus, DeviceRecord, JobId, JobSummary, ScanReport};

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
    /// A robot profile was created, changed, or deleted.
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
        summary: JobSummary,
    },
}

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
