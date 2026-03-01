use super::*;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum UpdaterMode {
    Flash,
    Mount,
    Ota,
    Other(String),
}

impl UpdaterMode {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Flash => "flash",
            Self::Mount => "mount",
            Self::Ota => "ota",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl From<&str> for UpdaterMode {
    fn from(value: &str) -> Self {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "flash" | "" => Self::Flash,
            "mount" => Self::Mount,
            "ota" => Self::Ota,
            _ => Self::Other(normalized),
        }
    }
}

impl From<String> for UpdaterMode {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl fmt::Display for UpdaterMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for UpdaterMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdaterMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from(raw))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum UpdaterStatus {
    Running,
    Success,
    Error,
    Info,
    Other(String),
}

impl UpdaterStatus {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Error => "error",
            Self::Info => "info",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl From<&str> for UpdaterStatus {
    fn from(value: &str) -> Self {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "running" => Self::Running,
            "success" => Self::Success,
            "error" => Self::Error,
            "info" => Self::Info,
            _ => Self::Other(normalized),
        }
    }
}

impl From<String> for UpdaterStatus {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl fmt::Display for UpdaterStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for UpdaterStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdaterStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from(raw))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum UpdaterStep {
    ResolveImage,
    ResolveTarget,
    Rpiboot,
    Flash,
    Verify,
    Finalize,
    OtaUpload,
    OtaApply,
    OtaMonitor,
    CancelRequested,
    Complete,
    Info,
    Other(String),
}

impl UpdaterStep {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::ResolveImage => "resolve-image",
            Self::ResolveTarget => "resolve-target",
            Self::Rpiboot => "rpiboot",
            Self::Flash => "flash",
            Self::Verify => "verify",
            Self::Finalize => "finalize",
            Self::OtaUpload => "ota-upload",
            Self::OtaApply => "ota-apply",
            Self::OtaMonitor => "ota-monitor",
            Self::CancelRequested => "cancel-requested",
            Self::Complete => "complete",
            Self::Info => "info",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl From<&str> for UpdaterStep {
    fn from(value: &str) -> Self {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "resolve-image" => Self::ResolveImage,
            "resolve-target" => Self::ResolveTarget,
            "rpiboot" => Self::Rpiboot,
            "flash" => Self::Flash,
            "verify" => Self::Verify,
            "finalize" => Self::Finalize,
            "ota-upload" => Self::OtaUpload,
            "ota-apply" => Self::OtaApply,
            "ota-monitor" => Self::OtaMonitor,
            "cancel-requested" => Self::CancelRequested,
            "complete" => Self::Complete,
            "info" => Self::Info,
            _ => Self::Other(normalized),
        }
    }
}

impl From<String> for UpdaterStep {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl fmt::Display for UpdaterStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for UpdaterStep {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdaterStep {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from(raw))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseInstallRequest {
    pub(crate) release_download_url: Option<String>,
    pub(crate) local_image_path: Option<String>,
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) mount_only: Option<bool>,
    pub(crate) prefer_ota: Option<bool>,
    pub(crate) target_ip_address: Option<String>,
    pub(crate) selected_bootloader_id: Option<String>,
    pub(crate) run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MountRequest {
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) selected_bootloader_id: Option<String>,
    pub(crate) run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelUpdateRequest {
    pub(crate) run_id: Option<String>,
    pub(crate) mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeExistingOtaUpdateRequest {
    pub(crate) target_ip_addresses: Vec<String>,
    pub(crate) timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExistingOtaUpdateProbeResult {
    pub(crate) target_ip_address: String,
    pub(crate) update_id: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachExistingOtaUpdateRequest {
    pub(crate) run_id: String,
    pub(crate) target_ip_address: String,
    pub(crate) expected_update_id: Option<String>,
    pub(crate) timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdaterProgressEvent {
    pub(crate) run_id: Option<String>,
    pub(crate) mode: UpdaterMode,
    pub(crate) step: UpdaterStep,
    pub(crate) status: UpdaterStatus,
    pub(crate) message: String,
    pub(crate) timestamp_epoch_ms: u64,
    pub(crate) stdout: Option<String>,
    pub(crate) stderr: Option<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) image_path: Option<String>,
    pub(crate) target_path: Option<String>,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) bytes_written: Option<u64>,
    pub(crate) bytes_total: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogListRequest {
    pub(crate) ip_address: String,
}

#[derive(Debug, Serialize, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogEntry {
    pub(crate) id: String,
    pub(crate) file_name: String,
    pub(crate) remote_path: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_epoch_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogListResult {
    pub(crate) success: bool,
    pub(crate) fetched_at_epoch_ms: u64,
    pub(crate) entries: Vec<WpilibLogEntry>,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogSelection {
    pub(crate) file_name: String,
    pub(crate) remote_path: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogsActionRequest {
    pub(crate) ip_address: String,
    pub(crate) selected_logs: Vec<WpilibLogSelection>,
    pub(crate) download_directory: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogActionItemResult {
    pub(crate) file_name: String,
    pub(crate) remote_path: String,
    pub(crate) local_path: Option<String>,
    pub(crate) success: bool,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WpilibLogActionResult {
    pub(crate) success: bool,
    pub(crate) processed_count: usize,
    pub(crate) success_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) completed_at_epoch_ms: u64,
    pub(crate) download_directory: Option<String>,
    pub(crate) results: Vec<WpilibLogActionItemResult>,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseInstallResult {
    pub(crate) success: bool,
    pub(crate) mode: String,
    pub(crate) image_path: Option<String>,
    pub(crate) selected_target_path: Option<String>,
    pub(crate) rpiboot: OperationResult,
    pub(crate) flash: Option<OperationResult>,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseDownloadCacheClearResult {
    pub(crate) removed_files: u64,
    pub(crate) removed_bytes: u64,
    pub(crate) cache_directory: String,
    pub(crate) message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedUpdaterRecoveryState {
    pub(crate) run_id: String,
    pub(crate) mode: String,
    pub(crate) status: String,
    pub(crate) target_ip_address: Option<String>,
    pub(crate) expected_update_id: Option<String>,
    pub(crate) monitor_timeout_seconds: Option<u64>,
    pub(crate) started_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) note: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdaterRecoverySession {
    pub(crate) run_id: String,
    pub(crate) mode: String,
    pub(crate) target_ip_address: Option<String>,
    pub(crate) expected_update_id: Option<String>,
    pub(crate) started_at_epoch_ms: u64,
    pub(crate) resumed_at_epoch_ms: u64,
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlashRequest {
    pub(crate) image_path: String,
    pub(crate) device_path: String,
    pub(crate) confirm_flash: bool,
}

#[derive(Clone)]
pub(crate) struct FlashProgressContext {
    pub(crate) app: tauri::AppHandle,
    pub(crate) run_id: Option<String>,
    pub(crate) target_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPrivilegeStrategy {
    Direct,
    SudoNoPrompt,
    PkexecPrompt,
    #[allow(dead_code)]
    SudoPrompt,
}

pub(crate) struct TimedCommandOutput {
    pub(crate) output: Output,
    pub(crate) timed_out: bool,
    pub(crate) canceled: bool,
    pub(crate) duration_ms: u64,
}
