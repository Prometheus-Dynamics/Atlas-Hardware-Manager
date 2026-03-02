use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolSource {
    Bundled,
    SystemPath,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedTool {
    pub(crate) path: PathBuf,
    pub(crate) source: ToolSource,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolStatus {
    pub(crate) available: bool,
    pub(crate) path: Option<String>,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostSetupCheck {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) required: bool,
    pub(crate) ready: bool,
    pub(crate) detail: String,
    pub(crate) detected: Option<String>,
    pub(crate) fix_hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostSetupStatus {
    pub(crate) platform: String,
    pub(crate) arch: String,
    pub(crate) ready: bool,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) checks: Vec<HostSetupCheck>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostSetupRepairResult {
    pub(crate) status: HostSetupStatus,
    pub(crate) attempted_actions: Vec<String>,
    pub(crate) applied_actions: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsbDevice {
    pub(crate) bus: String,
    pub(crate) device: String,
    pub(crate) usb_path: Option<String>,
    pub(crate) vendor_id: String,
    pub(crate) product_id: String,
    pub(crate) description: String,
    pub(crate) is_bootloader: bool,
    pub(crate) is_helios_candidate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FirmwareImage {
    pub(crate) path: String,
    pub(crate) file_name: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_epoch_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlashTarget {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) size_bytes: u64,
    pub(crate) model: String,
    pub(crate) transport: Option<String>,
    pub(crate) removable: bool,
    pub(crate) hotplug: bool,
    pub(crate) mounted: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeliosDiscoverySnapshot {
    pub(crate) workspace_path: String,
    pub(crate) workspace_exists: bool,
    pub(crate) generated_at_epoch_ms: u64,
    pub(crate) rpiboot: ToolStatus,
    pub(crate) sudo: ToolStatus,
    pub(crate) bootloader_present: bool,
    pub(crate) usb_devices: Vec<UsbDevice>,
    pub(crate) flash_targets: Vec<FlashTarget>,
    pub(crate) firmware_images: Vec<FirmwareImage>,
    pub(crate) network_neighbors: Vec<NetworkNeighbor>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationResult {
    pub(crate) success: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: u64,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) message: String,
    pub(crate) timed_out: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseImageOption {
    pub(crate) release_tag: String,
    pub(crate) release_name: String,
    pub(crate) prerelease: bool,
    pub(crate) asset_name: String,
    pub(crate) download_url: String,
    pub(crate) size_bytes: u64,
    pub(crate) published_at: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientUpdateStatus {
    pub(crate) current_version: String,
    pub(crate) latest_version: Option<String>,
    pub(crate) latest_tag: Option<String>,
    pub(crate) latest_name: Option<String>,
    pub(crate) prerelease: bool,
    pub(crate) update_available: bool,
    pub(crate) download_asset_name: Option<String>,
    pub(crate) download_url: Option<String>,
    pub(crate) release_page_url: Option<String>,
    pub(crate) published_at: Option<String>,
    pub(crate) checked_at_epoch_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientSelfUpdateResult {
    pub(crate) success: bool,
    pub(crate) started: bool,
    pub(crate) message: String,
    pub(crate) current_version: String,
    pub(crate) latest_version: Option<String>,
    pub(crate) download_asset_name: Option<String>,
    pub(crate) download_url: Option<String>,
    pub(crate) release_page_url: Option<String>,
    pub(crate) installer_path: Option<String>,
}
