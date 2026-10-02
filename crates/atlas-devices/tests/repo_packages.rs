//! The device packages in this repository must load cleanly: OS builds pin
//! them, and a package Atlas cannot read is a package Atlas cannot manage.

use std::collections::BTreeMap;
use std::path::Path;

use atlas_devices::{CONTRACT, DeviceCatalog};

fn repo_catalog() -> DeviceCatalog {
    let devices = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../devices");
    DeviceCatalog::load(&[devices])
}

#[test]
fn every_repo_package_loads_without_warnings() {
    let catalog = repo_catalog();
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    for package in catalog.packages() {
        assert_eq!(
            package.manifest.contract, CONTRACT,
            "{}",
            package.manifest.model
        );
    }
}

#[test]
fn raze_is_recognised_in_recovery_and_running() {
    let catalog = repo_catalog();
    let raze = catalog.by_model("raze").expect("devices/raze is present");

    assert_eq!(catalog.for_usb_boot(0x0a5c, 0x2712).len(), 1);
    assert!(
        catalog
            .for_gadget(Some("Prometheus Dynamics"), Some("Raze"))
            .is_some()
    );
    assert_eq!(
        raze.manifest.recovery.storage_target.as_deref(),
        Some("emmc")
    );
    assert!(!raze.manifest.recovery.instructions.is_empty());
    assert!(raze.manifest.has_capability("camera"));
    assert!(raze.manifest.eeprom_file("pieeprom").is_some());
    assert!(raze.manifest.eeprom_file("pieeprom-sig").is_some());
    assert!(raze.eeprom_dir().is_some());
    // With no revision source yet, every board falls back to the default.
    assert!(raze.manifest.revision_for(&BTreeMap::new()).is_some());
}
