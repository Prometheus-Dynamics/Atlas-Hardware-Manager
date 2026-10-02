//! Live views of a device and the fleet history.

use atlas_driver::{DeviceKey, LogLine, Metric};

use crate::{ActivityEntry, Atlas, CoreError};

impl Atlas {
    /// Current readings, for devices whose driver offers telemetry.
    pub async fn telemetry(&self, key: &DeviceKey) -> Result<Vec<Metric>, CoreError> {
        let (live, record) = self.live_device(key)?;
        let telemetry = live.capabilities.telemetry.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "telemetry",
        })?;
        Ok(telemetry.read(&record.identity).await?)
    }

    /// The last `lines` log lines, oldest first, for devices that offer logs.
    pub async fn logs(&self, key: &DeviceKey, lines: usize) -> Result<Vec<LogLine>, CoreError> {
        let (live, record) = self.live_device(key)?;
        let logs = live.capabilities.logs.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "logs",
        })?;
        Ok(logs.tail(&record.identity, lines.clamp(1, 2000)).await?)
    }

    /// Fleet history, newest first, at most `limit` entries.
    pub fn activity(&self, limit: usize) -> Vec<ActivityEntry> {
        self.inner
            .state()
            .activity
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}
