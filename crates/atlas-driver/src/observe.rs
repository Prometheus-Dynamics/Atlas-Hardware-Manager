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
}

/// Metric ids with a shared meaning. The UI gives these a gauge and uses
/// them in robot summaries; any other id is still shown, as a plain value.
pub mod metric_ids {
    pub const CPU: &str = "cpu";
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
        }
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
