//! Hardware drivers compiled into the app.

use std::path::PathBuf;
use std::sync::Arc;

use atlas_core::{AtlasBuilder, install};
use atlas_devices::DeviceCatalog;
use atlas_driver_board::{
    CatalogHealth, NetworkLinks, SshAccess, UsbGadgetLinks, drivers_for_catalog,
};
use atlas_driver_rpi::{RpiConfig, RpiDriver, SshKeys, UsbBootLinks};

use crate::settings::AppPaths;

/// The app's own folders for device packages (`devices/<model>/`): the
/// bundled ones and the data folder's.
fn device_dirs(paths: &AppPaths) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = install::bundled_device_dirs();
    dirs.push(paths.data_dir.join("devices"));
    dirs
}

/// Registers every hardware driver and link source this build includes.
pub fn register_hardware(
    builder: AtlasBuilder,
    paths: &AppPaths,
    ssh_keys: &SshKeys,
    ssh_access: &SshAccess,
    warnings: &mut Vec<String>,
) -> AtlasBuilder {
    let own = device_dirs(paths);
    let catalog = Arc::new(DeviceCatalog::load(&atlas_devices::default_search_dirs(
        &own,
    )));
    let searched = atlas_devices::search_report(&own);
    warnings.extend(catalog.warnings().iter().cloned());

    let mut boot_file_dirs = install::bundled_usbboot_dirs();
    boot_file_dirs.push(paths.data_dir.join("usbboot"));
    let mut builder = builder
        .driver(Arc::new(RpiDriver::new(RpiConfig {
            boot_file_dirs,
            catalog: catalog.clone(),
            ssh_keys: ssh_keys.clone(),
            known_hosts: ssh_access.get().known_hosts,
        })))
        .link_source(Arc::new(UsbBootLinks))
        .link_source(Arc::new(NetworkLinks))
        .link_source(Arc::new(UsbGadgetLinks))
        .link_source(Arc::new(CatalogHealth::new(&catalog, &searched)));
    for driver in drivers_for_catalog(&catalog, ssh_access) {
        builder = builder.driver(driver);
    }
    // The robot network's controller and radio (behind a board's USB route
    // they are its children).
    for driver in atlas_driver_robot::drivers() {
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
