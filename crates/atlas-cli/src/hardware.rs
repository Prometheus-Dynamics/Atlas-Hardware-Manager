//! The hardware drivers and link sources the CLI and the app both use.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_core::{AtlasBuilder, install};
use atlas_devices::DeviceCatalog;
use atlas_driver_board::{
    CatalogHealth, NetworkLinks, SshAccess, UsbGadgetLinks, drivers_for_catalog,
};
use atlas_driver_rpi::{RpiConfig, RpiDriver, SshKeys, UsbBootLinks};

/// Registers every hardware driver and link source this build includes.
/// `data_dir` (`<data>/atlas`) adds its `devices/` and `usbboot/` to the
/// bundled ones; device package warnings go to `warnings`.
pub fn register_hardware(
    builder: AtlasBuilder,
    data_dir: Option<&Path>,
    ssh_keys: &SshKeys,
    ssh_access: &SshAccess,
    warnings: &mut Vec<String>,
) -> AtlasBuilder {
    let with_data = |mut dirs: Vec<PathBuf>, name: &str| {
        dirs.extend(data_dir.map(|data| data.join(name)));
        dirs
    };
    let device_dirs = with_data(install::bundled_device_dirs(), "devices");
    let catalog = Arc::new(DeviceCatalog::load(&atlas_devices::default_search_dirs(
        &device_dirs,
    )));
    let searched = atlas_devices::search_report(&device_dirs);
    warnings.extend(catalog.warnings().iter().cloned());

    let mut builder = builder
        .driver(Arc::new(RpiDriver::new(RpiConfig {
            boot_file_dirs: with_data(install::bundled_usbboot_dirs(), "usbboot"),
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
