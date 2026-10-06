//! Hardware drivers compiled into the app.

use std::path::PathBuf;
use std::sync::Arc;

use atlas_core::AtlasBuilder;
use atlas_devices::DeviceCatalog;
use atlas_driver_pd::{NetworkLinks, SshAccess, UsbGadgetLinks, drivers_for_catalog};
use atlas_driver_rpi::{RpiConfig, RpiDriver, SshKeys, UsbBootLinks};

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

/// Folders that may hold device packages (`devices/<model>/`).
fn device_dirs(paths: &AppPaths) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = resource_dirs()
        .into_iter()
        .filter_map(|dir| dir.parent().map(|parent| parent.join("devices")))
        .collect();
    dirs.push(paths.data_dir.join("devices"));
    atlas_devices::default_search_dirs(&dirs)
}

/// Registers every hardware driver and link source this build includes.
pub fn register_hardware(
    builder: AtlasBuilder,
    paths: &AppPaths,
    ssh_keys: &SshKeys,
    ssh_access: &SshAccess,
    warnings: &mut Vec<String>,
) -> AtlasBuilder {
    let catalog = Arc::new(DeviceCatalog::load(&device_dirs(paths)));
    warnings.extend(catalog.warnings().iter().cloned());

    let mut boot_file_dirs = resource_dirs();
    boot_file_dirs.push(paths.data_dir.join("usbboot"));
    let mut builder = builder
        .driver(Arc::new(RpiDriver::new(RpiConfig {
            boot_file_dirs,
            catalog: catalog.clone(),
            ssh_keys: ssh_keys.clone(),
        })))
        .link_source(Arc::new(UsbBootLinks))
        .link_source(Arc::new(NetworkLinks))
        .link_source(Arc::new(UsbGadgetLinks));
    for driver in drivers_for_catalog(&catalog, ssh_access) {
        builder = builder.driver(driver);
    }
    builder
}

#[cfg(test)]
mod tests {
    /// The rule the Linux packages install must be the one the helper's
    /// one-click fix writes, so both paths give the same access.
    #[test]
    fn packaged_udev_rule_matches_the_helper() {
        assert_eq!(
            include_str!("../bundle/linux/60-atlas-usbboot.rules"),
            atlas_usbboot::LINUX_UDEV_RULES_FILE
        );
    }
}
