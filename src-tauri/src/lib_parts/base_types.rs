pub(crate) use rusb::UsbContext;
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use serde_json::Value;
pub(crate) use std::cmp::Reverse;
pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use std::env;
pub(crate) use std::fs;
pub(crate) use std::io::{self, Read, Write};
pub(crate) use std::net::Ipv4Addr;
pub(crate) use std::net::SocketAddr;
pub(crate) use std::net::SocketAddrV4;
pub(crate) use std::net::TcpStream;
pub(crate) use std::net::ToSocketAddrs;
pub(crate) use std::net::UdpSocket;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::{Child, Command, Output, Stdio};
pub(crate) use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
pub(crate) use std::sync::{Mutex, OnceLock};
pub(crate) use std::thread;
pub(crate) use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub(crate) use tauri::{Emitter, Manager};
pub(crate) use tungstenite::stream::MaybeTlsStream;
pub(crate) use tungstenite::Message;
pub(crate) use tungstenite::WebSocket;

#[path = "base_types/host_types.rs"]
mod host_types;
#[path = "base_types/network_types.rs"]
mod network_types;
#[path = "base_types/telemetry_types.rs"]
mod telemetry_types;
#[path = "base_types/updater_types.rs"]
mod updater_types;

pub(crate) use host_types::*;
pub(crate) use network_types::*;
pub(crate) use telemetry_types::*;
pub(crate) use updater_types::*;

pub(crate) const DEFAULT_HELIOS_WORKSPACE: &str =
    "/run/media/sozo/bd1d96d9-fa81-4fac-b25e-193cfcac2dcb/Github/HeliOS";
pub(crate) const HELIOS_USB_ID_ALLOWLIST: [(&str, &str); 1] = [("1d6b", "0104")];
pub(crate) const HELIOS_RELEASES_API_URL: &str =
    "https://api.github.com/repos/Prometheus-Dynamics/HeliOS/releases?per_page=40";
pub(crate) const HELIOS_RUNTIME_API_PATHS: [&str; 22] = [
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
pub(crate) const ROBORIO_RUNTIME_API_PATHS: [&str; 23] = [
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
pub(crate) const ROBORIO_RUNTIME_TCP_PORTS: [u16; 4] = [80, 3580, 1250, 22];
pub(crate) const HELIOS_LOG_API_PATHS: [&str; 10] = [
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
pub(crate) const WPILIB_ROBORIO_LOG_DIRS: [&str; 2] = ["/home/lvuser/logs", "/u/logs"];
pub(crate) const PHOTONVISION_ARCHIVE_LOG_ZIP_PATH: &str = "/api/settings/photonvision_config.zip";
pub(crate) const PHOTONVISION_ARCHIVE_CACHE_TTL_MS: u64 = 60_000;
pub(crate) const MAX_IMAGE_RESULTS: usize = 120;
pub(crate) const MAX_SCAN_DEPTH: usize = 8;
pub(crate) const LINUX_FLASH_DEPENDENCIES: [&str; 5] = ["ip", "lsblk", "dd", "xz", "sync"];
pub(crate) const WINDOWS_RUNTIME_DEPENDENCIES: [&str; 2] = ["powershell", "arp"];
pub(crate) const MACOS_RUNTIME_DEPENDENCIES: [&str; 2] = ["diskutil", "arp"];
pub(crate) const UPDATER_PROGRESS_EVENT: &str = "updater-progress";
pub(crate) const DEVICE_TELEMETRY_EVENT: &str = "device-telemetry";
pub(crate) const NETWORK_DISCOVERY_PROGRESS_EVENT: &str = "network-discovery-progress";

#[derive(Clone)]
pub(crate) struct PhotonvisionArchiveLogCacheEntry {
    pub(crate) fetched_at_epoch_ms: u64,
    pub(crate) source_url: String,
    pub(crate) raw_log_text: String,
}

pub(crate) static PHOTONVISION_ARCHIVE_LOG_CACHE: OnceLock<
    Mutex<HashMap<String, PhotonvisionArchiveLogCacheEntry>>,
> = OnceLock::new();
pub(crate) static APP_RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub(crate) static UPDATER_JOB_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static FALLBACK_ICON_RGBA_32: &[u8] = include_bytes!("../../icons/32x32.rgba");
#[cfg(target_os = "linux")]
pub(crate) static FALLBACK_ICON_PNG_32: &[u8] = include_bytes!("../../icons/32x32.png");
#[cfg(target_os = "linux")]
pub(crate) static FALLBACK_ICON_PNG_128: &[u8] = include_bytes!("../../icons/128x128.png");
#[cfg(target_os = "linux")]
pub(crate) static FALLBACK_ICON_PNG_256: &[u8] = include_bytes!("../../icons/128x128@2x.png");
#[cfg(target_os = "linux")]
pub(crate) static FALLBACK_ICON_SVG: &[u8] = include_bytes!("../../../static/logo.svg");
