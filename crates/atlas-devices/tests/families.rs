//! Device families: a board names its family and inherits the family's
//! recovery method and download-mode steps, what the board says wins, a
//! board naming no family file is refused, and the families the repository
//! ships load and match their schema.

use std::path::{Path, PathBuf};
use std::process::Command;

use atlas_devices::{DeviceCatalog, FAMILY_SCHEMA, UsbId};

const FAMILY: &str = r#"{
  "schema": "urn:board:family:1",
  "family": "test-rom",
  "display_name": "Test ROM",
  "vendor": "Rockchip",
  "method": "rockusb",
  "usb_vendor": "2207",
  "entry": ["Hold the key.", "Connect the board."],
  "flash": { "stages": ["maskrom"], "sram": null },
  "host": { "linux": "udev vendor match", "windows": "WinUSB class" },
  "pid_chips": { "1234": "TEST1" }
}"#;

/// A board record: `top` is extra top-level members (ending in a comma),
/// `recovery` extra members of `recovery` (starting with a comma).
fn board(model: &str, top: &str, recovery: &str) -> String {
    format!(
        r#"{{
  "schema": "urn:board:manifest:1",
  "contract": 1,
  "model": "{model}",
  "package_version": "2026.1.0-dev",
  "display_name": "{model}",
  "record_only": true,
  "platform": {{ "soc": "TEST" }},
  {top}
  "recovery": {{
    "usb_boot": {{ "vid": "2207", "pid": "1234" }},
    "storage_target": "emmc",
    "status": "untested",
    "flash": {{ "loader": "test_loader_*.bin", "storage": "emmc", "image_offset": 0 }}{recovery}
  }}
}}"#
    )
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atlas-families-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A device root holding the test family and the given board records.
fn root_with(label: &str, boards: &[(&str, String)]) -> PathBuf {
    let root = scratch(label);
    put(&root.join("families/test-rom.json"), FAMILY);
    for (model, text) in boards {
        put(&root.join(model).join("manifest.json"), text);
    }
    root
}

fn load(root: &Path) -> DeviceCatalog {
    DeviceCatalog::load(std::slice::from_ref(&root.to_path_buf()))
}

#[test]
fn a_board_inherits_its_familys_method_and_steps() {
    let root = root_with(
        "inherit",
        &[(
            "inheritor",
            board("inheritor", r#""family": "test-rom","#, ""),
        )],
    );
    let catalog = load(&root);
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());

    let package = catalog.by_model("inheritor").expect("the board loads");
    assert_eq!(package.family_id(), Some("test-rom"));
    assert_eq!(
        package.family.as_ref().map(|family| family.display_name()),
        Some("Test ROM")
    );
    let recovery = &package.manifest.recovery;
    assert_eq!(recovery.method.as_deref(), Some("rockusb"));
    assert_eq!(
        recovery.instructions,
        ["Hold the key.", "Connect the board."]
    );
    // The board keeps its own facts.
    assert_eq!(recovery.storage_target.as_deref(), Some("emmc"));
    assert_eq!(
        recovery
            .flash
            .as_ref()
            .and_then(|flash| flash.loader.first())
            .map(String::as_str),
        Some("test_loader_*.bin")
    );

    // The family's data is on the catalog, not repeated per board.
    let family = catalog.family("test-rom").expect("the family loads");
    assert_eq!(family.usb_vendor, UsbId(0x2207));
    assert_eq!(family.chip_for_pid(0x1234), Some("TEST1"));
    assert_eq!(family.chip_for_pid(0x9999), None);
    assert!(family.flash.is_some());
    assert_eq!(
        family.host.get("windows").map(String::as_str),
        Some("WinUSB class")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn what_a_board_names_wins_over_its_family() {
    let root = root_with(
        "override",
        &[(
            "overrider",
            board(
                "overrider",
                r#""family": "test-rom","#,
                r#", "method": "fel", "instructions": ["Its own steps."]"#,
            ),
        )],
    );
    let catalog = load(&root);
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());

    let package = catalog.by_model("overrider").expect("the board loads");
    assert_eq!(package.manifest.recovery.method.as_deref(), Some("fel"));
    assert_eq!(package.manifest.recovery.instructions, ["Its own steps."]);
    // The family is still named, and its data still there.
    assert_eq!(package.family_id(), Some("test-rom"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_board_naming_no_family_file_is_refused_and_the_warning_names_both() {
    let root = root_with(
        "unknown",
        &[(
            "lost-board",
            board("lost-board", r#""family": "no-such-family","#, ""),
        )],
    );
    let catalog = load(&root);

    assert!(catalog.by_model("lost-board").is_none());
    assert!(
        catalog.family("test-rom").is_some(),
        "the other families still load"
    );
    let [warning] = catalog.warnings() else {
        panic!("one warning, got {:?}", catalog.warnings());
    };
    assert!(warning.contains("lost-board"), "{warning}");
    assert!(warning.contains("no-such-family"), "{warning}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn variants_are_exposed_as_the_record_gives_them() {
    let top = r#""family": "test-rom",
  "variants": [
    { "id": "wide", "display_name": "Wide lens", "data": { "lens": "fisheye", "fx_px": 320.5 } },
    { "id": "narrow" }
  ],"#;
    let root = root_with("variants", &[("camera", board("camera", top, ""))]);
    let catalog = load(&root);
    assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());

    let package = catalog.by_model("camera").expect("the board loads");
    let variants = package.variants();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].id, "wide");
    assert_eq!(variants[0].display_name.as_deref(), Some("Wide lens"));
    assert_eq!(variants[0].data["lens"], "fisheye");
    assert_eq!(variants[0].data["fx_px"], 320.5);
    assert_eq!(variants[1].id, "narrow");
    assert!(variants[1].data.is_null());

    let plain = root_with("no-variants", &[("plain", board("plain", "", ""))]);
    let plain = load(&plain);
    assert!(
        plain
            .by_model("plain")
            .expect("loads")
            .variants()
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_file_in_families_that_is_not_a_family_is_a_warning() {
    let root = root_with("not-family", &[]);
    put(
        &root.join("families/other.json"),
        r#"{ "schema": "urn:board:manifest:1", "family": "other", "display_name": "x",
  "vendor": "x", "method": "rockusb", "usb_vendor": "2207", "entry": [], "flash": {}, "host": {} }"#,
    );
    let catalog = load(&root);

    assert!(catalog.family("other").is_none());
    assert!(catalog.family("test-rom").is_some());
    assert!(
        catalog
            .warnings()
            .iter()
            .any(|w| w.contains("other.json") && w.contains(FAMILY_SCHEMA)),
        "{:?}",
        catalog.warnings()
    );
    let _ = std::fs::remove_dir_all(root);
}

fn repo_devices() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../devices")
}

#[test]
fn the_rockchip_family_names_the_chip_tags_each_soc_accepts() {
    let catalog = DeviceCatalog::load(&[repo_devices()]);
    let family = catalog
        .family("rockchip-maskrom")
        .expect("the Rockchip family loads");
    assert_eq!(
        family.loader_chips("RV1126B"),
        Some(vec!["110F".to_string(), "1126".to_string()])
    );
    assert_eq!(
        family.loader_chips("RK3588S2"),
        Some(vec!["3588".to_string()])
    );
    // A SoC the family names no list for has none (refused by the flash).
    assert_eq!(family.loader_chips("RK3399"), None);
}

#[test]
fn the_repository_families_load_with_their_data() {
    let catalog = DeviceCatalog::load(&[repo_devices()]);
    assert!(
        !catalog.warnings().iter().any(|w| w.contains("families")),
        "{:?}",
        catalog.warnings()
    );

    let rockchip = catalog
        .family("rockchip-maskrom")
        .expect("rockchip-maskrom.json");
    assert_eq!(rockchip.method, "rockusb");
    assert_eq!(rockchip.usb_vendor, UsbId(0x2207));
    assert_eq!(rockchip.chip_for_pid(0x350b), Some("RK3588"));
    assert_eq!(rockchip.chip_for_pid(0x350a), Some("RK3566/RK3568"));
    assert_eq!(rockchip.chip_for_pid(0x3510), None);

    let fel = catalog.family("allwinner-fel").expect("allwinner-fel.json");
    assert_eq!(fel.method, "fel");
    assert_eq!(fel.usb_vendor, UsbId(0x1f3a));
    assert!(
        fel.flash
            .as_ref()
            .is_some_and(|flash| flash["sram"].is_null())
    );

    let k230 = catalog.family("canaan-k230").expect("canaan-k230.json");
    assert_eq!(k230.method, "k230-usb");
    assert_eq!(k230.usb_vendor, UsbId(0x29f1));
    for family in catalog.families() {
        assert_eq!(
            family.schema.as_deref(),
            Some(FAMILY_SCHEMA),
            "{}",
            family.family
        );
        assert!(
            !family.entry.is_empty(),
            "{}: its download-mode steps",
            family.family
        );
    }
}

/// Validates `files` against `schema` with the repository's JSON Schema
/// checker (`devices/tools/board_lemnos.py`, `schema_errors`), returning the
/// errors it finds, one per line.
fn schema_errors(schema: &Path, files: &[PathBuf]) -> String {
    const CHECK: &str = r#"
import json, sys
sys.path.insert(0, sys.argv[1])
import board_lemnos
schema = json.load(open(sys.argv[2]))
for path in sys.argv[3:]:
    for error in board_lemnos.schema_errors(json.load(open(path)), schema, schema):
        print(f"{path}: {error}")
"#;
    let output = Command::new("python3")
        .args(["-I", "-B", "-c", CHECK])
        .arg(repo_devices().join("tools"))
        .arg(schema)
        .args(files)
        .output()
        .expect("python3 runs the schema check");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_families_and_a_family_board_validate_against_their_schemas() {
    let schemas = repo_devices().join("schema");
    let families: Vec<PathBuf> = ["rockchip-maskrom", "allwinner-fel", "canaan-k230"]
        .iter()
        .map(|id| repo_devices().join("families").join(format!("{id}.json")))
        .collect();
    let errors = schema_errors(&schemas.join("family.schema.json"), &families);
    assert!(errors.trim().is_empty(), "family files:\n{errors}");

    // A board naming a family, with a variant and no method or steps of its
    // own, is a valid record.
    let top = r#""family": "rockchip-maskrom",
  "variants": [{ "id": "wide", "data": { "lens": "fisheye" } }],"#;
    let root = root_with(
        "schema",
        &[("schema-board", board("schema-board", top, ""))],
    );
    let record = root.join("schema-board/manifest.json");
    let errors = schema_errors(&schemas.join("manifest.schema.json"), &[record]);
    assert!(errors.trim().is_empty(), "a family board:\n{errors}");
    let _ = std::fs::remove_dir_all(root);
}
