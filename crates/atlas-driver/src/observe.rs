//! Optional live views of a running device: metrics and logs.
//!
//! Both are capabilities a driver grants only when the device offers them.
//! Atlas shows whatever a device reports and nothing more, so a device can
//! start with one metric and grow without any change to Atlas.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{DriverError, Identity};

/// Identity attributes with a shared meaning. Drivers set them when the
/// device provides the thing; the UI offers the matching control only then.
pub mod attributes {
    /// A web page for managing the device, opened in the user's browser.
    pub const WEB_UI: &str = "manage_url";
    /// A live camera stream the UI can show (MJPEG or a still-image URL).
    pub const CAMERA_STREAM: &str = "camera_stream";
    /// The device's network hostname.
    pub const HOSTNAME: &str = "hostname";
    /// The physical board's own serial, the same whichever way a driver
    /// sees it (boot ROM, recovery gadget, running OS). Lets Atlas tell that
    /// a recovery device and a running device are one board.
    pub const BOARD_SERIAL: &str = "board_serial";
    /// How far the device's clock is from this computer's, in seconds
    /// (negative: behind), when it is noticeably off. Absent when it's right.
    pub const CLOCK_OFFSET_S: &str = "clock_offset_s";

    /// Atlas's board-serial rule: the last 8 hex digits, lowercase. A
    /// Raspberry Pi's boot ROM reports just those; the running OS (device
    /// tree `serial-number`, often NUL-terminated) and Orion report the full
    /// serial. `None` when the serial isn't hex.
    pub fn normalize_board_serial(raw: &str) -> Option<String> {
        let serial = raw.trim_matches(|c: char| c.is_whitespace() || c == '\0');
        let tail = serial.get(serial.len().checked_sub(8)?..)?;
        serial
            .chars()
            .all(|c| c.is_ascii_hexdigit())
            .then(|| tail.to_ascii_lowercase())
    }

    #[cfg(test)]
    mod tests {
        use super::normalize_board_serial;

        #[test]
        fn board_serials_normalize_to_their_last_8_hex_digits() {
            assert_eq!(
                normalize_board_serial("a317bcbee5226d57").as_deref(),
                Some("e5226d57")
            );
            assert_eq!(
                normalize_board_serial("10000000ABCDEF01\0\n").as_deref(),
                Some("abcdef01")
            );
            assert_eq!(
                normalize_board_serial("e5226d57").as_deref(),
                Some("e5226d57")
            );
            assert_eq!(normalize_board_serial("short"), None);
            assert_eq!(normalize_board_serial("not-a-hex-serial"), None);
        }
    }
}

/// Metric ids with a shared meaning. The UI gives these a gauge and uses
/// them in robot summaries; any other id is still shown, as a plain value.
pub mod metric_ids {
    pub const CPU: &str = "cpu";
    /// One core's busy share, `cpu.core.<n>` (kernel CPU order).
    pub const CPU_CORE_PREFIX: &str = "cpu.core.";
    pub const TEMPERATURE: &str = "temp";
    pub const FAN: &str = "fan";
    pub const UPTIME: &str = "uptime";
    pub const MEMORY: &str = "memory";
    pub const VOLTAGE: &str = "voltage";
    pub const CURRENT: &str = "current";
    pub const FPS: &str = "fps";
}

/// One reading, for example `{"id":"temp","label":"Temperature","value":54.5,"unit":"°C"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    pub label: String,
    pub value: f64,
    #[serde(default)]
    pub unit: Option<String>,
    /// The top of the normal range, for drawing a gauge.
    #[serde(default)]
    pub max: Option<f64>,
    /// Above this the UI shows a warning.
    #[serde(default)]
    pub warn_above: Option<f64>,
    /// The raw numbers behind the value, as one line: `1.2 of 4.0 GiB`,
    /// `load 0.42 · 0.38 · 0.30`.
    #[serde(default)]
    pub detail: Option<String>,
}

impl Metric {
    pub fn new(id: &str, label: &str, value: f64, unit: Option<&str>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value,
            unit: unit.map(Into::into),
            max: None,
            warn_above: None,
            detail: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn range(mut self, max: f64, warn_above: Option<f64>) -> Self {
        self.max = Some(max);
        self.warn_above = warn_above;
        self
    }

    pub fn is_warning(&self) -> bool {
        self.warn_above.is_some_and(|limit| self.value > limit)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// Unix milliseconds, when the device reports a time.
    #[serde(default)]
    pub at_ms: Option<u64>,
    #[serde(default)]
    pub level: LogLevel,
    /// The service or unit that wrote the line.
    #[serde(default)]
    pub source: Option<String>,
    pub message: String,
}

/// Live readings such as CPU load, temperature, and fan speed.
#[async_trait]
pub trait TelemetryCapability: Send + Sync {
    async fn read(&self, device: &Identity) -> Result<Vec<Metric>, DriverError>;
}

/// Recent log lines from the device, oldest first.
#[async_trait]
pub trait LogsCapability: Send + Sync {
    async fn tail(&self, device: &Identity, lines: usize) -> Result<Vec<LogLine>, DriverError>;
}
