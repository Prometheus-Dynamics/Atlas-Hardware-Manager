//! Live views of a device, commands to its board's devices, and the fleet
//! history.

use atlas_driver::{DeviceKey, FrameSink, HardwareCommand, LogLine, Metric};

use crate::{ActivityEntry, ActivityKind, ActivityLevel, Atlas, CoreError};

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

    /// Streams the board devices `wanted` (id, period ms) live into `sink`
    /// until the board ends the stream (`Ok(true)`) or the caller drops
    /// this; `Ok(false)` when the board has no live stream.
    pub async fn stream_hardware(
        &self,
        key: &DeviceKey,
        wanted: &[(String, u32)],
        sink: FrameSink,
    ) -> Result<bool, CoreError> {
        let (live, record) = self.live_device(key)?;
        let Some(status) = live.capabilities.status else {
            return Ok(false);
        };
        Ok(status
            .stream_hardware(&record.identity, wanted, sink)
            .await?)
    }

    /// Runs `command` on the board device `hardware` (its id in the device's
    /// hardware snapshot) and logs it. A set answers the applied value.
    pub async fn control_hardware(
        &self,
        key: &DeviceKey,
        hardware: &str,
        command: HardwareCommand,
    ) -> Result<Option<f64>, CoreError> {
        let (live, record) = self.live_device(key)?;
        let control = live.capabilities.hardware.ok_or(CoreError::Unsupported {
            device: key.clone(),
            what: "hardware controls",
        })?;
        let what = format!("{hardware}: {}", command.describe());
        let result = control.control(&record.identity, hardware, command).await;
        let name = record.display_name();
        self.inner.record_activity(vec![match &result {
            Ok(_) => ActivityEntry::about(
                &record,
                ActivityKind::ActionRun,
                ActivityLevel::Info,
                format!("{what} on {name}"),
            ),
            Err(error) => ActivityEntry::about(
                &record,
                ActivityKind::ActionRun,
                ActivityLevel::Error,
                format!("{what} on {name} failed: {error}"),
            ),
        }]);
        self.inner.persist();
        Ok(result?)
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
