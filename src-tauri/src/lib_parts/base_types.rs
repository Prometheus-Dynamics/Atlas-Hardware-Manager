use rusb::UsbContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::TcpStream;
use std::net::ToSocketAddrs;
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;
use tungstenite::WebSocket;

const DEFAULT_HELIOS_WORKSPACE: &str =
    "/run/media/sozo/bd1d96d9-fa81-4fac-b25e-193cfcac2dcb/Github/HeliOS";
const HELIOS_USB_ID_ALLOWLIST: [(&str, &str); 1] = [("1d6b", "0104")];
const HELIOS_RELEASES_API_URL: &str =
    "https://api.github.com/repos/Prometheus-Dynamics/HeliOS/releases?per_page=40";
const HELIOS_RUNTIME_API_PATHS: [&str; 22] = [
    "/v1/health",
    "/v1/device/os",
    "/v1/device/metrics",
    "/v1/device/hostname",
    "/",
    "/api/status",
    "/api/system",
    "/api/info",
    "/api/settings",
    "/api/version",
    "/api/telemetry",
    "/api/v1/status",
    "/api/v1/system",
    "/api/v1/info",
    "/api/v1/settings",
    "/api/v1/version",
    "/json",
    "/results",
    "/photonvision",
    "/status",
    "/system",
    "/telemetry",
];
const ROBORIO_RUNTIME_API_PATHS: [&str; 23] = [
    "/status",
    "/api/status",
    "/api/v1/status",
    "/api/system",
    "/api/v1/system",
    "/api/metrics",
    "/api/v1/metrics",
    "/metrics",
    "/json",
    "/ni/status",
    "/ni/metrics",
    "/networktables.json",
    "/nt/status",
    "/nt/metrics",
    "/",
    "/?action=getstatus",
    "/?action=getversion",
    "/?action=getdevices",
    "/?action=getdiagnostics",
    "/?action=getcanstatus",
    "/?action=getnetwork",
    "/?action=getsystem",
    "/?action=gethealth",
];
const ROBORIO_RUNTIME_TCP_PORTS: [u16; 4] = [80, 3580, 1250, 22];
const HELIOS_LOG_API_PATHS: [&str; 10] = [
    "/v1/device/logs",
    "/api/logs",
    "/api/v1/logs",
    "/api/system/logs",
    "/api/v1/system/logs",
    "/api/utils/logs",
    "/api/utils/journal",
    "/api/journal",
    "/logs",
    "/journal",
];
const WPILIB_ROBORIO_LOG_DIRS: [&str; 2] = ["/home/lvuser/logs", "/u/logs"];
const PHOTONVISION_ARCHIVE_LOG_ZIP_PATH: &str = "/api/settings/photonvision_config.zip";
const PHOTONVISION_ARCHIVE_CACHE_TTL_MS: u64 = 60_000;
const MAX_IMAGE_RESULTS: usize = 120;
const MAX_SCAN_DEPTH: usize = 8;
const LINUX_FLASH_DEPENDENCIES: [&str; 5] = ["ip", "lsblk", "dd", "xz", "sync"];
const WINDOWS_RUNTIME_DEPENDENCIES: [&str; 2] = ["powershell", "arp"];
const MACOS_RUNTIME_DEPENDENCIES: [&str; 2] = ["diskutil", "arp"];
const UPDATER_PROGRESS_EVENT: &str = "updater-progress";
const DEVICE_TELEMETRY_EVENT: &str = "device-telemetry";
const NETWORK_DISCOVERY_PROGRESS_EVENT: &str = "network-discovery-progress";

#[derive(Clone)]
struct PhotonvisionArchiveLogCacheEntry {
    fetched_at_epoch_ms: u64,
    source_url: String,
    raw_log_text: String,
}

static PHOTONVISION_ARCHIVE_LOG_CACHE: OnceLock<
    Mutex<HashMap<String, PhotonvisionArchiveLogCacheEntry>>,
> = OnceLock::new();
static APP_RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
static UPDATE_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
static UPDATER_JOB_ACTIVE: AtomicBool = AtomicBool::new(false);
static FALLBACK_ICON_RGBA_32: &[u8] = include_bytes!("../../icons/32x32.rgba");
#[cfg(target_os = "linux")]
static FALLBACK_ICON_PNG_32: &[u8] = include_bytes!("../../icons/32x32.png");
#[cfg(target_os = "linux")]
static FALLBACK_ICON_PNG_128: &[u8] = include_bytes!("../../icons/128x128.png");
#[cfg(target_os = "linux")]
static FALLBACK_ICON_PNG_256: &[u8] = include_bytes!("../../icons/128x128@2x.png");
#[cfg(target_os = "linux")]
static FALLBACK_ICON_SVG: &[u8] = include_bytes!("../../../static/logo.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolSource {
    Bundled,
    SystemPath,
}

#[derive(Debug, Clone)]
struct ResolvedTool {
    path: PathBuf,
    source: ToolSource,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolStatus {
    available: bool,
    path: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostSetupCheck {
    id: String,
    label: String,
    required: bool,
    ready: bool,
    detail: String,
    detected: Option<String>,
    fix_hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostSetupStatus {
    platform: String,
    arch: String,
    ready: bool,
    generated_at_epoch_ms: u64,
    checks: Vec<HostSetupCheck>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HostSetupRepairResult {
    status: HostSetupStatus,
    attempted_actions: Vec<String>,
    applied_actions: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UsbDevice {
    bus: String,
    device: String,
    usb_path: Option<String>,
    vendor_id: String,
    product_id: String,
    description: String,
    is_bootloader: bool,
    is_helios_candidate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FirmwareImage {
    path: String,
    file_name: String,
    size_bytes: u64,
    modified_epoch_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlashTarget {
    path: String,
    name: String,
    size_bytes: u64,
    model: String,
    transport: Option<String>,
    removable: bool,
    hotplug: bool,
    mounted: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NetworkNeighbor {
    ip: String,
    mac: Option<String>,
    interface: Option<String>,
    state: Option<String>,
    is_helios_candidate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeliosDiscoverySnapshot {
    workspace_path: String,
    workspace_exists: bool,
    generated_at_epoch_ms: u64,
    rpiboot: ToolStatus,
    sudo: ToolStatus,
    bootloader_present: bool,
    usb_devices: Vec<UsbDevice>,
    flash_targets: Vec<FlashTarget>,
    firmware_images: Vec<FirmwareImage>,
    network_neighbors: Vec<NetworkNeighbor>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DiscoveredDevice {
    id: String,
    display_name: String,
    status: String,
    connection_chips: Vec<String>,
    ip_address: Option<String>,
    mac_address: Option<String>,
    interface_name: Option<String>,
    usb_location: Option<String>,
    vendor_product: Option<String>,
    runtime_product: Option<String>,
    firmware_version: Option<String>,
    os_version: Option<String>,
    telemetry_summary: Option<String>,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceDiscoverySnapshot {
    generated_at_epoch_ms: u64,
    devices: Vec<DiscoveredDevice>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeviceDiscoveryProgressEvent {
    generated_at_epoch_ms: u64,
    devices: Vec<DiscoveredDevice>,
    warnings: Vec<String>,
    in_progress: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionStatusSnapshot {
    connected: bool,
    route: String,
    label: String,
    detail: String,
    target_ip: Option<String>,
    interface_name: Option<String>,
    generated_at_epoch_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationResult {
    success: bool,
    exit_code: Option<i32>,
    duration_ms: u64,
    stdout: String,
    stderr: String,
    message: String,
    timed_out: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ReleaseImageOption {
    release_tag: String,
    release_name: String,
    prerelease: bool,
    asset_name: String,
    download_url: String,
    size_bytes: u64,
    published_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseInstallRequest {
    release_download_url: Option<String>,
    local_image_path: Option<String>,
    timeout_seconds: Option<u64>,
    mount_only: Option<bool>,
    prefer_ota: Option<bool>,
    target_ip_address: Option<String>,
    selected_bootloader_id: Option<String>,
    run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MountRequest {
    timeout_seconds: Option<u64>,
    selected_bootloader_id: Option<String>,
    run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelUpdateRequest {
    run_id: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeExistingOtaUpdateRequest {
    target_ip_addresses: Vec<String>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExistingOtaUpdateProbeResult {
    target_ip_address: String,
    update_id: Option<String>,
    stage: Option<String>,
    progress_percent: Option<f64>,
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachExistingOtaUpdateRequest {
    run_id: String,
    target_ip_address: String,
    expected_update_id: Option<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UpdaterProgressEvent {
    run_id: Option<String>,
    mode: String,
    step: String,
    status: String,
    message: String,
    timestamp_epoch_ms: u64,
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    image_path: Option<String>,
    target_path: Option<String>,
    progress_percent: Option<f64>,
    bytes_written: Option<u64>,
    bytes_total: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceLogRequest {
    ip_address: String,
    runtime_product: Option<String>,
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceTelemetryStreamRequest {
    stream_id: String,
    ip_address: String,
    runtime_product: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceTelemetryStopRequest {
    stream_id: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DeviceTelemetryEvent {
    stream_id: String,
    status: String,
    message: String,
    timestamp_epoch_ms: u64,
    runtime_product: Option<String>,
    os_version: Option<String>,
    telemetry_summary: Option<String>,
    source_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceLogResult {
    success: bool,
    source_url: Option<String>,
    line_count: usize,
    truncated: bool,
    fetched_at_epoch_ms: u64,
    log_text: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WpilibLogListRequest {
    ip_address: String,
}

#[derive(Debug, Serialize, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WpilibLogEntry {
    id: String,
    file_name: String,
    remote_path: String,
    size_bytes: u64,
    modified_epoch_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WpilibLogListResult {
    success: bool,
    fetched_at_epoch_ms: u64,
    entries: Vec<WpilibLogEntry>,
    message: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WpilibLogSelection {
    file_name: String,
    remote_path: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WpilibLogsActionRequest {
    ip_address: String,
    selected_logs: Vec<WpilibLogSelection>,
    download_directory: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WpilibLogActionItemResult {
    file_name: String,
    remote_path: String,
    local_path: Option<String>,
    success: bool,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WpilibLogActionResult {
    success: bool,
    processed_count: usize,
    success_count: usize,
    failed_count: usize,
    completed_at_epoch_ms: u64,
    download_directory: Option<String>,
    results: Vec<WpilibLogActionItemResult>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseInstallResult {
    success: bool,
    mode: String,
    image_path: Option<String>,
    selected_target_path: Option<String>,
    rpiboot: OperationResult,
    flash: Option<OperationResult>,
    message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PersistedUpdaterRecoveryState {
    run_id: String,
    mode: String,
    status: String,
    target_ip_address: Option<String>,
    expected_update_id: Option<String>,
    monitor_timeout_seconds: Option<u64>,
    started_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    note: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UpdaterRecoverySession {
    run_id: String,
    mode: String,
    target_ip_address: Option<String>,
    expected_update_id: Option<String>,
    started_at_epoch_ms: u64,
    resumed_at_epoch_ms: u64,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlashRequest {
    image_path: String,
    device_path: String,
    confirm_flash: bool,
}

#[derive(Clone)]
struct FlashProgressContext {
    app: tauri::AppHandle,
    run_id: Option<String>,
    target_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinuxPrivilegeStrategy {
    Direct,
    SudoNoPrompt,
    PkexecPrompt,
    SudoPrompt,
}

struct TimedCommandOutput {
    output: Output,
    timed_out: bool,
    canceled: bool,
    duration_ms: u64,
}

#[derive(Debug, Clone)]
struct RawNeighborEntry {
    ip: String,
    mac: Option<String>,
    interface: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Clone)]
struct IpDeviceCandidate {
    ip: String,
    mac: Option<String>,
    interface: String,
    state: Option<String>,
    is_usb_link: bool,
    usb_identity: Option<UsbIdentity>,
}

#[derive(Debug, Clone)]
struct UsbIdentity {
    vendor_id: String,
    product_id: String,
    manufacturer: Option<String>,
    product: Option<String>,
    serial: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct HeliosRuntimeInfo {
    hostname: Option<String>,
    runtime_product: Option<String>,
    firmware_version: Option<String>,
    os_version: Option<String>,
    telemetry_summary: Option<String>,
}

impl HeliosRuntimeInfo {
    fn has_data(&self) -> bool {
        self.hostname.is_some()
            || self.runtime_product.is_some()
            || self.firmware_version.is_some()
            || self.os_version.is_some()
            || self.telemetry_summary.is_some()
    }

    fn is_complete(&self) -> bool {
        self.runtime_product.is_some()
            && self.firmware_version.is_some()
            && self.os_version.is_some()
            && self.telemetry_summary.is_some()
    }

    fn merge_missing(&mut self, other: HeliosRuntimeInfo) {
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
