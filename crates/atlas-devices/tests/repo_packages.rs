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
    // Gen 1 boards carry no revision marker, so an unmarked board is gen1.
    let unmarked = raze.manifest.revision_for(&BTreeMap::new()).unwrap();
    assert_eq!(unmarked.id, "gen1");
    assert_eq!(raze.manifest.revision_label("gen1"), "Gen 1");
}

/// Every `device-package.env` under a package reports the manifest's
/// `package_version`, so the identity document never claims a stale one.
#[test]
fn device_package_env_matches_the_manifest_version() {
    fn env_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                env_files(&path, found);
            } else if path
                .file_name()
                .is_some_and(|name| name == "device-package.env")
            {
                found.push(path);
            }
        }
    }

    for package in repo_catalog().packages() {
        let expected = package
            .manifest
            .package_version
            .as_deref()
            .expect("package_version is set");
        let mut found = Vec::new();
        env_files(&package.dir, &mut found);
        for file in found {
            let text = std::fs::read_to_string(&file).unwrap();
            let reported = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("PD_DEVICE_PACKAGE_VERSION="))
                .map(|value| value.trim().trim_matches('"'));
            assert_eq!(
                reported,
                Some(expected),
                "{} reports a different package version than manifest.json",
                file.display()
            );
        }
    }
}

/// A value from a pd-device `*.env` or `lib.sh` file: `KEY=value`, unquoted.
fn shell_value(file: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).unwrap();
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix(key)?.strip_prefix('=')?;
        Some(value.trim().trim_matches('"').to_string())
    })
}

#[test]
fn raze_names_its_gadget_address() {
    let raze = repo_catalog();
    let raze = raze.by_model("raze").expect("raze package");
    let scripts = raze.dir.join("gaia/assets/rootfs/usr/lib/pd-device");
    assert_eq!(
        raze.manifest.legacy_gadget_address().as_deref(),
        Some("172.31.250.1")
    );
    assert_eq!(
        shell_value(&scripts.join("lib.sh"), "PD_GADGET_LEGACY_ADDRESS").as_deref(),
        Some("172.31.250.1/24")
    );

    // The device computes its address from usb-gadget.env and Atlas from the
    // manifest: they must describe the same scheme.
    let addressing = raze.manifest.gadget_addressing().expect("addressing");
    assert_eq!(addressing.scheme, atlas_devices::SERIAL_HASH_V1);
    let env = scripts.join("usb-gadget.env");
    let value = |key: &str| shell_value(&env, key).unwrap_or_default();
    assert_eq!(value("USB_GADGET_ADDRESS"), "AUTO");
    assert_eq!(value("USB_GADGET_ADDR_BASE"), addressing.base);
    assert_eq!(
        value("USB_GADGET_ADDR_PREFIX"),
        addressing.prefix.to_string()
    );
    assert_eq!(
        value("USB_GADGET_ADDR_EXCLUDE"),
        addressing.exclude.join(" ")
    );
    assert_eq!(
        addressing.subnet_for("e5226d57").map(|s| s.to_string()),
        Some("172.31.209.217/29".into())
    );
}
