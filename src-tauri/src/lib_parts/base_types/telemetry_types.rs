use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceLogRequest {
    pub(crate) ip_address: String,
    pub(crate) runtime_product: Option<String>,
    pub(crate) max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceTelemetryStreamRequest {
    pub(crate) stream_id: String,
    pub(crate) ip_address: String,
    pub(crate) runtime_product: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceTelemetryStopRequest {
    pub(crate) stream_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceTelemetryEvent {
    pub(crate) stream_id: String,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) timestamp_epoch_ms: u64,
    pub(crate) runtime_product: Option<String>,
    pub(crate) os_version: Option<String>,
    pub(crate) telemetry_summary: Option<String>,
    pub(crate) source_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceLogResult {
    pub(crate) success: bool,
    pub(crate) source_url: Option<String>,
    pub(crate) line_count: usize,
    pub(crate) truncated: bool,
    pub(crate) fetched_at_epoch_ms: u64,
    pub(crate) log_text: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HeliosRuntimeInfo {
    pub(crate) hostname: Option<String>,
    pub(crate) runtime_product: Option<String>,
    pub(crate) firmware_version: Option<String>,
    pub(crate) os_version: Option<String>,
    pub(crate) telemetry_summary: Option<String>,
}

impl HeliosRuntimeInfo {
    pub(crate) fn has_data(&self) -> bool {
        self.hostname.is_some()
            || self.runtime_product.is_some()
            || self.firmware_version.is_some()
            || self.os_version.is_some()
            || self.telemetry_summary.is_some()
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.runtime_product.is_some()
            && self.firmware_version.is_some()
            && self.os_version.is_some()
            && self.telemetry_summary.is_some()
    }

    pub(crate) fn merge_missing(&mut self, other: HeliosRuntimeInfo) {
        if self.hostname.is_none() {
            self.hostname = other.hostname;
        }
        if self.runtime_product.is_none() {
            self.runtime_product = other.runtime_product;
        }
        if self.firmware_version.is_none() {
            self.firmware_version = other.firmware_version;
        }
        if self.os_version.is_none() {
            self.os_version = other.os_version;
        }
        if self.telemetry_summary.is_none() {
            self.telemetry_summary = other.telemetry_summary;
        }
    }
}
