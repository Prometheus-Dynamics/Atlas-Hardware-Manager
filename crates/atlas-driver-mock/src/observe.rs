//! Simulated telemetry and logs: smooth, plausible numbers that move a
//! little on every read, so the UI has something live to show.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use atlas_driver::{
    DriverError, Identity, LogLevel, LogLine, LogsCapability, Metric, TelemetryCapability,
    metric_ids,
};

use crate::{MockDevice, MockFleet, SIM_HELIOS};

/// A simulated camera view: a test pattern any `<img>` can show.
pub(crate) const TEST_PATTERN: &str = "data:image/svg+xml;utf8,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 320 180'%3E%3Crect width='320' height='180' fill='%23111'/%3E%3Cg opacity='.85'%3E%3Crect x='0' width='46' height='120' fill='%23ddd'/%3E%3Crect x='46' width='46' height='120' fill='%23dd0'/%3E%3Crect x='92' width='46' height='120' fill='%230dd'/%3E%3Crect x='138' width='46' height='120' fill='%230d0'/%3E%3Crect x='184' width='46' height='120' fill='%23d0d'/%3E%3Crect x='230' width='46' height='120' fill='%23d00'/%3E%3Crect x='276' width='44' height='120' fill='%2300d'/%3E%3C/g%3E%3Ctext x='160' y='156' fill='%23aaa' font-family='sans-serif' font-size='14' text-anchor='middle'%3ESimulated camera%3C/text%3E%3C/svg%3E";

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// A stable 0..1 phase per device, so devices do not move in lockstep.
fn phase(device: &MockDevice) -> f64 {
    let sum: u32 = device.key.serial.0.bytes().map(u32::from).sum();
    f64::from(sum % 97) / 97.0
}

/// A slow wave between `low` and `high`, with a little jitter.
fn wave(device: &MockDevice, period_s: f64, low: f64, high: f64) -> f64 {
    let t = now_ms() as f64 / 1000.0;
    let x = (t / period_s + phase(device)) * std::f64::consts::TAU;
    let jitter = ((t * 7.3 + phase(device) * 13.0).sin()) * 0.06;
    let unit = ((x.sin() + 1.0) / 2.0 + jitter).clamp(0.0, 1.0);
    low + (high - low) * unit
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

pub(crate) fn metrics(device: &MockDevice) -> Vec<Metric> {
    let uptime = now_ms().saturating_sub(device.booted_ms) as f64 / 1000.0;
    if device.key.family.as_str() == SIM_HELIOS {
        let cpu = wave(device, 23.0, 18.0, 71.0);
        let temp = 42.0 + cpu * 0.32 + wave(device, 61.0, 0.0, 3.0);
        vec![
            Metric::new(metric_ids::CPU, "CPU", round(cpu, 0), Some("%")).range(100.0, Some(90.0)),
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                round(temp, 1),
                Some("°C"),
            )
            .range(85.0, Some(75.0)),
            Metric::new(
                metric_ids::FAN,
                "Fan",
                round(1800.0 + (temp - 40.0) * 95.0, -1),
                Some("rpm"),
            )
            .range(6000.0, None),
            Metric::new(
                metric_ids::MEMORY,
                "Memory",
                round(wave(device, 90.0, 31.0, 44.0), 0),
                Some("%"),
            )
            .range(100.0, Some(90.0)),
            Metric::new(
                metric_ids::FPS,
                "Vision",
                round(wave(device, 9.0, 52.0, 60.0), 0),
                Some("fps"),
            )
            .range(60.0, None),
            Metric::new(metric_ids::UPTIME, "Uptime", round(uptime, 0), Some("s")),
        ]
    } else {
        let current = wave(device, 7.0, 0.4, 3.2);
        vec![
            Metric::new(
                metric_ids::VOLTAGE,
                "Bus voltage",
                round(12.6 - current * 0.18, 2),
                Some("V"),
            )
            .range(14.0, None),
            Metric::new(metric_ids::CURRENT, "Current", round(current, 2), Some("A"))
                .range(20.0, Some(15.0)),
            Metric::new(
                metric_ids::TEMPERATURE,
                "Temperature",
                round(34.0 + current * 2.1, 1),
                Some("°C"),
            )
            .range(85.0, Some(70.0)),
            Metric::new(metric_ids::UPTIME, "Uptime", round(uptime, 0), Some("s")),
        ]
    }
}

const CAMERA_LINES: [(LogLevel, &str, &str); 8] = [
    (
        LogLevel::Info,
        "photonvision",
        "Pipeline \"apriltag\" running at 58 fps",
    ),
    (
        LogLevel::Info,
        "networktables",
        "Connected to robot at 10.0.0.2",
    ),
    (LogLevel::Debug, "camera", "Exposure adjusted to 12 ms"),
    (LogLevel::Info, "photonvision", "Target 7 acquired"),
    (LogLevel::Info, "systemd", "Started PhotonVision service"),
    (
        LogLevel::Warning,
        "thermal",
        "SoC above 70 °C; fan raised to 80%",
    ),
    (LogLevel::Info, "photonvision", "Target 7 lost"),
    (LogLevel::Debug, "camera", "Frame queue depth 2"),
];

const BOARD_LINES: [(LogLevel, &str, &str); 5] = [
    (LogLevel::Info, "can", "Heartbeat ok"),
    (LogLevel::Info, "motor", "Closed loop enabled"),
    (LogLevel::Debug, "adc", "Bus voltage sample 12.4 V"),
    (LogLevel::Warning, "can", "One frame retried"),
    (LogLevel::Info, "motor", "Setpoint reached"),
];

/// Lines spaced two seconds apart, ending now, so a poll shows new ones.
pub(crate) fn log_lines(device: &MockDevice, lines: usize) -> Vec<LogLine> {
    let templates: &[(LogLevel, &str, &str)] = if device.key.family.as_str() == SIM_HELIOS {
        &CAMERA_LINES
    } else {
        &BOARD_LINES
    };
    let step = 2000;
    let now = now_ms() / step * step;
    let booted = device.booted_ms;
    (0..lines as u64)
        .rev()
        .map(|back| now.saturating_sub(back * step))
        .filter(|at| *at >= booted)
        .map(|at| {
            let (level, source, message) = templates[((at / step) as usize) % templates.len()];
            LogLine {
                at_ms: Some(at),
                level,
                source: Some(source.into()),
                message: message.into(),
            }
        })
        .collect()
}

pub(crate) struct MockTelemetry {
    pub(crate) fleet: MockFleet,
}

#[async_trait]
impl TelemetryCapability for MockTelemetry {
    async fn read(&self, device: &Identity) -> Result<Vec<Metric>, DriverError> {
        let mock = self.fleet.online(&device.key)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(metrics(&mock))
    }
}

pub(crate) struct MockLogs {
    pub(crate) fleet: MockFleet,
}

#[async_trait]
impl LogsCapability for MockLogs {
    async fn tail(&self, device: &Identity, lines: usize) -> Result<Vec<LogLine>, DriverError> {
        let mock = self.fleet.online(&device.key)?;
        Ok(log_lines(&mock, lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SIM_MCU;

    #[test]
    fn cameras_and_boards_report_their_own_metrics() {
        let camera = MockDevice::new(SIM_HELIOS, "H-1");
        let ids: Vec<String> = metrics(&camera).into_iter().map(|m| m.id).collect();
        assert!(ids.contains(&"fps".to_string()));
        let board = MockDevice::new(SIM_MCU, "M-1");
        let ids: Vec<String> = metrics(&board).into_iter().map(|m| m.id).collect();
        assert!(ids.contains(&"voltage".to_string()));
        assert!(metrics(&camera).iter().all(|m| m.value.is_finite()));
    }

    #[test]
    fn logs_start_at_boot() {
        let mut device = MockDevice::new(SIM_HELIOS, "H-1");
        assert_eq!(log_lines(&device, 50).len(), 50);
        // Just restarted: at most the current line.
        device.booted_ms = now_ms();
        assert!(log_lines(&device, 50).len() <= 1);
    }
}
