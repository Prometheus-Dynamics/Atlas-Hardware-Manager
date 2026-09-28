//! Hardware drivers compiled into the app.

use std::path::PathBuf;
use std::sync::Arc;

use atlas_core::AtlasBuilder;
use atlas_driver_rpi::{RpiConfig, RpiDriver, UsbBootLinks};

use crate::settings::AppPaths;

/// Where installers put bundled resources relative to the executable:
/// next to it (Windows, dev), `../Resources` (macOS), `../lib/<app>`
/// (Linux packages).
fn resource_dirs() -> Vec<PathBuf> {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    else {
        return Vec::new();
    };
    let mut dirs = vec![
        exe_dir.join("usbboot"),
        exe_dir.join("resources").join("usbboot"),
        exe_dir.join("../Resources/usbboot"),
    ];
    for name in [
        "atlas-hardware-manager",
        "Atlas Hardware Manager",
        "atlas-app",
    ] {
        dirs.push(exe_dir.join("../lib").join(name).join("usbboot"));
    }
    dirs
}

/// Registers every hardware driver and link source this build includes.
pub fn register_hardware(
    builder: AtlasBuilder,
    paths: &AppPaths,
    _warnings: &mut Vec<String>,
) -> AtlasBuilder {
    let mut boot_file_dirs = resource_dirs();
    boot_file_dirs.push(paths.data_dir.join("usbboot"));
    builder
        .driver(Arc::new(RpiDriver::new(RpiConfig { boot_file_dirs })))
        .link_source(Arc::new(UsbBootLinks))
}
