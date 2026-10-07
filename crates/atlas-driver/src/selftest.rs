//! A device's own hardware self-test.
//!
//! The device runs its checks (LEDs, fan, camera, sensors, ...) and reports
//! each one as ok, skipped or failed. The report is the device contract's
//! `selftest --json` output, format 1; Atlas keeps and shows it as given, so a
//! device can add checks without any change here.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{DriverError, Identity};

/// The report format this Atlas reads.
pub const SELFTEST_FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Skip,
    Fail,
    /// A status a newer device reports that this Atlas doesn't know.
    #[serde(other)]
    Unknown,
}

/// One check: `{id, status, message, data}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfTestCheck {
    /// For example `fan` or `camera`.
    pub id: String,
    pub status: CheckStatus,
    /// One sentence saying what was found.
    #[serde(default)]
    pub message: String,
    /// What the check measured, as the device reports it.
    #[serde(default)]
    pub data: serde_json::Value,
}

/// The whole report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfTestReport {
    /// The report format, [`SELFTEST_FORMAT`] today.
    #[serde(default)]
    pub version: u32,
    /// The board serial as the device reads it (full hex).
    #[serde(default)]
    pub board_serial: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The device package version on the board.
    #[serde(default)]
    pub package_version: Option<String>,
    /// When it ran, Unix seconds on the device's clock.
    #[serde(default)]
    pub at: Option<u64>,
    /// Whether an operator answered questions at the device.
    #[serde(default)]
    pub interactive: bool,
    /// No check failed.
    pub ok: bool,
    #[serde(default)]
    pub checks: Vec<SelfTestCheck>,
}

impl SelfTestReport {
    /// Reads the report from a command's output: the last line that is a
    /// JSON object, so anything printed before it is ignored.
    pub fn parse(output: &str) -> Result<Self, String> {
        let line = output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with('{'))
            .ok_or_else(|| "the self-test printed no report".to_string())?;
        serde_json::from_str(line).map_err(|error| format!("unreadable self-test report: {error}"))
    }

    pub fn count(&self, status: CheckStatus) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == status)
            .count()
    }

    /// One line, for example `5 of 6 checks passed; camera failed`.
    pub fn summary(&self) -> String {
        let total = self.checks.len();
        let passed = self.count(CheckStatus::Ok);
        let failed: Vec<&str> = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .map(|check| check.id.as_str())
            .collect();
        let skipped = self.count(CheckStatus::Skip);
        let mut text = format!("{passed} of {total} checks passed");
        if !failed.is_empty() {
            text.push_str(&format!("; {} failed", failed.join(", ")));
        }
        if skipped > 0 {
            text.push_str(&format!("; {skipped} skipped"));
        }
        text
    }
}

/// Runs a device's self-test. Granted only to devices that have one.
#[async_trait]
pub trait SelfTestCapability: Send + Sync {
    /// Runs every non-interactive check and returns the report. The device
    /// puts anything it changed (fan, LEDs) back before it answers.
    async fn run_selftest(&self, device: &Identity) -> Result<SelfTestReport, DriverError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{"version":1,"board_serial":"10000000a317bcbe","model":"raze","package_version":"1.0.7","at":1791347363,"interactive":false,"ok":false,"checks":[{"id":"leds","status":"ok","message":"takes frames","data":{"count":16}},{"id":"camera","status":"fail","message":"no ov9782","data":{}},{"id":"i2c","status":"skip","message":"no i2cdetect","data":{}},{"id":"future","status":"maybe","message":""}]}"#;

    #[test]
    fn the_device_report_parses() {
        let report = SelfTestReport::parse(&format!("warming up\n{REPORT}\n")).unwrap();
        assert_eq!(report.version, SELFTEST_FORMAT);
        assert!(!report.ok);
        assert_eq!(report.checks[0].data["count"], 16);
        assert_eq!(report.checks[3].status, CheckStatus::Unknown);
        assert_eq!(
            report.summary(),
            "1 of 4 checks passed; camera failed; 1 skipped"
        );
    }

    #[test]
    fn no_report_is_an_error() {
        assert!(SelfTestReport::parse("selftest: run it as root\n").is_err());
        assert!(SelfTestReport::parse("{ not json").is_err());
    }
}
