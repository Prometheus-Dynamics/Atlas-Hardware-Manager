use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HealthStatus {
    Ok,
    /// Works, but something is degraded or missing for some flows.
    Warning,
    /// A flow that depends on this will fail.
    Error,
}

/// One host readiness check, such as USB access or a required boot file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Stable id, for example `usbboot.files`.
    pub id: String,
    pub label: String,
    pub status: HealthStatus,
    /// What was found, in one sentence.
    pub detail: String,
    /// What the user can do about it, when there is something.
    pub fix: Option<String>,
}

impl HealthCheck {
    pub fn ok(id: &str, label: &str, detail: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: HealthStatus::Ok,
            detail: detail.into(),
            fix: None,
        }
    }

    pub fn warning(
        id: &str,
        label: &str,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: HealthStatus::Warning,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }

    pub fn error(id: &str, label: &str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: HealthStatus::Error,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}
