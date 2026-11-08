//! The Rust USB id table (`atlas-usb-ids`) agrees with the data it is written
//! from (`devices/usb-ids.json`, `devices/ecosystem.json`), and with the files
//! generated from that data (`devices/tools/gen-usb-ids.py`). The generator's
//! `--check` is the exact comparison; these tests name the disagreement.

use std::path::PathBuf;

use atlas_devices::{DeviceCatalog, UsbId};
use atlas_usb_ids as ids;
use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn json(path: &str) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

fn id(value: &Value) -> u16 {
    u16::from_str_radix(value.as_str().expect("an id string"), 16).expect("hex")
}

/// Every download mode in the data: (name, vendor, product).
fn boot_modes() -> Vec<(String, u16, u16)> {
    json("devices/usb-ids.json")["boot_modes"]
        .as_array()
        .expect("boot_modes")
        .iter()
        .map(|mode| {
            (
                mode["name"].as_str().expect("name").to_string(),
                id(&mode["vid"]),
                id(&mode["pid"]),
            )
        })
        .collect()
}

#[test]
fn the_boot_mode_table_is_the_data() {
    let modes = boot_modes();
    let (pi, soc): (Vec<_>, Vec<_>) = modes
        .iter()
        .partition(|(_, vid, _)| *vid == ids::BROADCOM_VENDOR_ID);

    // The Raspberry Pi products, as a set.
    let mut pi_pids: Vec<u16> = pi.iter().map(|(_, _, pid)| *pid).collect();
    let mut rust_pi = ids::RPI_BOOT_PRODUCT_IDS.to_vec();
    pi_pids.sort_unstable();
    rust_pi.sort_unstable();
    assert_eq!(
        pi_pids, rust_pi,
        "RPI_BOOT_PRODUCT_IDS vs devices/usb-ids.json"
    );
    assert!(
        pi.iter()
            .all(|(name, _, _)| name == "Raspberry Pi USB boot")
    );

    // The SoC download modes, in the data's order, with their names.
    let rust_soc: Vec<(String, u16, u16)> = ids::SOC_BOOT_IDS
        .iter()
        .map(|(name, vid, pid)| (name.to_string(), *vid, *pid))
        .collect();
    let data_soc: Vec<(String, u16, u16)> = soc.into_iter().cloned().collect();
    assert_eq!(rust_soc, data_soc, "SOC_BOOT_IDS vs devices/usb-ids.json");
}

#[test]
fn the_storage_gadget_and_board_gadget_are_the_data() {
    let ids_data = json("devices/usb-ids.json");
    assert_eq!(
        id(&ids_data["storage_gadget"]["vid"]),
        ids::BROADCOM_VENDOR_ID
    );
    assert_eq!(
        id(&ids_data["storage_gadget"]["pid"]),
        ids::STORAGE_GADGET_PRODUCT_ID
    );

    let eco = json("devices/ecosystem.json");
    let gadget = &eco["identity"]["usb_gadget"];
    assert_eq!(id(&gadget["vid"]), ids::BOARD_GADGET_VENDOR_ID);
    assert_eq!(id(&gadget["pid"]), ids::BOARD_GADGET_PRODUCT_ID);
    assert_eq!(gadget["manufacturer"], ids::BOARD_GADGET_MANUFACTURER);
    assert_eq!(eco["flasher"]["product"], ids::FLASHER_PRODUCT);
}

#[test]
fn the_rules_file_grants_every_boot_mode_and_is_the_packaged_copy() {
    let rules = ids::LINUX_UDEV_RULES_FILE;
    assert_eq!(
        rules,
        read("apps/desktop/src-tauri/bundle/linux/60-atlas-usbboot.rules"),
        "the packaged rules differ from the generated ones; run devices/tools/gen-usb-ids.py"
    );
    assert!(
        rules.contains(ids::LINUX_UDEV_RULE),
        "the Pi rule is missing"
    );
    for (name, vid, pid) in boot_modes() {
        let vendor = format!("ATTR{{idVendor}}==\"{}\"", ids::hex(vid));
        let vendor_wide = rules
            .lines()
            .any(|line| line.contains(&vendor) && !line.contains("idProduct"));
        // A rule names several products as "2711|2712|...".
        let wanted = ids::hex(pid);
        let by_product = rules.lines().any(|line| {
            line.contains(&vendor)
                && line
                    .split("ATTR{idProduct}==\"")
                    .skip(1)
                    .filter_map(|rest| rest.split('"').next())
                    .any(|list| list.split('|').any(|id| id == wanted))
        });
        assert!(
            vendor_wide || by_product,
            "no udev rule grants {name} ({}:{})",
            ids::hex(vid),
            ids::hex(pid)
        );
    }
    let storage = format!(
        "ATTRS{{idVendor}}==\"{}\", ATTRS{{idProduct}}==\"{}\"",
        ids::hex(ids::BROADCOM_VENDOR_ID),
        ids::hex(ids::STORAGE_GADGET_PRODUCT_ID)
    );
    assert!(
        rules.contains(&storage),
        "the storage gadget's console rule is missing"
    );
}

#[test]
fn the_windows_installer_binds_every_boot_mode() {
    let hooks = read("apps/desktop/src-tauri/bundle/windows/hooks.nsh");
    for (name, vid, pid) in boot_modes() {
        let line = format!(
            "ATLAS_BIND_WINUSB \"{name}\" 0x{} 0x{}",
            ids::hex(vid),
            ids::hex(pid)
        );
        assert!(hooks.contains(&line), "hooks.nsh lacks: {line}");
    }
}

#[test]
fn the_raze_package_inherits_the_ecosystem_through_the_catalog() {
    let catalog = DeviceCatalog::load(&[repo().join("devices")]);
    let raze = catalog.by_model("raze").expect("the Raze package loads");
    let gadget = raze
        .manifest
        .identity
        .usb_gadget
        .as_ref()
        .expect("usb_gadget is inherited");
    assert_eq!(gadget.vid, Some(UsbId(ids::BOARD_GADGET_VENDOR_ID)));
    assert_eq!(gadget.pid, Some(UsbId(ids::BOARD_GADGET_PRODUCT_ID)));
    assert_eq!(
        gadget.manufacturer.as_deref(),
        Some(ids::BOARD_GADGET_MANUFACTURER)
    );
    assert_eq!(
        gadget.product.as_deref(),
        Some("Raze"),
        "the board keeps its product"
    );
    assert!(
        raze.manifest.identity.mac_rule.is_some(),
        "mac_rule is inherited"
    );
    assert!(
        raze.manifest.gadget_addressing().is_some(),
        "addressing is inherited"
    );
    assert!(
        catalog.ecosystem("atlas-board").is_some(),
        "devices/ecosystem.json loads"
    );
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
}
