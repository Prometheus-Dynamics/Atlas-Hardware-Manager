//! Persistence: round trip, older files, unusable files set aside, and a
//! crash that leaves a temp file behind.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_inventory::{Inventory, JsonFileStore, LoadOutcome, Presence};
use atlas_types::{CapabilitySet, DeviceKey, Identity, LinkId, LinkKind};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A scratch directory unique to one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "atlas-inventory-{label}-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("inventory.json")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sample() -> Inventory {
    let mut inventory = Inventory::new();
    let identity = Identity {
        key: DeviceKey::new("sample-soc", "1000abcd"),
        model: "sample-board".into(),
        state: atlas_types::DeviceState::Running,
        versions: BTreeMap::from([("os".to_string(), "1.0.0".to_string())]),
        name: None,
        link: LinkId::new("eth:enp3s0"),
        address: "10.53.38.11".into(),
        attributes: BTreeMap::new(),
    };
    inventory.observe(identity, LinkKind::Ethernet, CapabilitySet::new(), 1_000);
    inventory.set_label(
        &DeviceKey::new("sample-soc", "1000abcd"),
        Some("cam".into()),
        2_000,
    );
    inventory
}

fn only_device(inventory: &Inventory) -> &atlas_inventory::DeviceRecord {
    let all = inventory.all();
    assert_eq!(all.len(), 1);
    all[0]
}

#[test]
fn a_saved_inventory_loads_back() {
    let scratch = Scratch::new("roundtrip");
    let store = JsonFileStore::new(scratch.file());
    store.save(&sample()).unwrap();

    let (loaded, outcome) = store.load();
    assert!(
        matches!(
            outcome,
            LoadOutcome::Restored {
                skipped_records: 0,
                dropped_values: 0
            }
        ),
        "{outcome:?}"
    );
    let record = only_device(&loaded);
    assert_eq!(record.label.as_deref(), Some("cam"));
    assert_eq!(
        record.presence,
        Presence::Offline,
        "nothing is online until seen"
    );
    assert_eq!(record.links.len(), 1);
    assert!(
        !scratch.0.join("inventory.json.tmp").exists(),
        "temp file renamed away"
    );
}

#[test]
fn a_missing_file_is_a_fresh_start() {
    let scratch = Scratch::new("fresh");
    let (inventory, outcome) = JsonFileStore::new(scratch.file()).load();
    assert_eq!(outcome, LoadOutcome::Fresh);
    assert!(inventory.all().is_empty());
}

#[test]
fn an_older_file_without_newer_fields_loads() {
    let scratch = Scratch::new("older");
    fs::write(scratch.file(), br#"{"version":0,"devices":[]}"#).unwrap();
    let (inventory, outcome) = JsonFileStore::new(scratch.file()).load();
    assert!(
        matches!(
            outcome,
            LoadOutcome::Restored {
                skipped_records: 0,
                dropped_values: 0
            }
        ),
        "{outcome:?}"
    );
    assert!(inventory.all().is_empty());
}

#[test]
fn a_corrupt_file_is_set_aside_and_the_app_starts_empty() {
    let scratch = Scratch::new("corrupt");
    fs::write(scratch.file(), b"{ this is not json").unwrap();

    let (inventory, outcome) = JsonFileStore::new(scratch.file()).load();
    assert!(inventory.all().is_empty());
    let LoadOutcome::Reset { aside, .. } = outcome else {
        panic!("a corrupt file must be reset, got {outcome:?}");
    };
    let aside = aside.expect("moved aside");
    assert!(
        !scratch.file().exists(),
        "the bad file no longer blocks the next save"
    );
    assert_eq!(fs::read(aside).unwrap(), b"{ this is not json");
}

#[test]
fn a_file_from_a_newer_atlas_loads_what_this_build_understands() {
    let scratch = Scratch::new("newer");
    fs::write(
        scratch.file(),
        br#"{"version":99,"devices":[],"future-field":true}"#,
    )
    .unwrap();
    let (inventory, outcome) = JsonFileStore::new(scratch.file()).load();
    assert!(inventory.all().is_empty());
    assert_eq!(
        outcome,
        LoadOutcome::Restored {
            skipped_records: 0,
            dropped_values: 0
        }
    );
    assert!(scratch.file().exists(), "a readable file is not set aside");
}

#[test]
fn a_crash_mid_save_leaves_the_last_good_file() {
    let scratch = Scratch::new("crash");
    let store = JsonFileStore::new(scratch.file());
    store.save(&sample()).unwrap();
    // A save that died after writing the temp file, before the rename.
    fs::write(scratch.0.join("inventory.json.tmp"), b"partial").unwrap();

    let (loaded, outcome) = store.load();
    assert!(
        matches!(
            outcome,
            LoadOutcome::Restored {
                skipped_records: 0,
                dropped_values: 0
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(only_device(&loaded).label.as_deref(), Some("cam"));

    store.save(&loaded).unwrap();
    assert!(!Path::new(&scratch.0.join("inventory.json.tmp")).exists());
}

#[test]
fn the_power_cycle_survives_a_restart() {
    let scratch = Scratch::new("cycles");
    let store = JsonFileStore::new(scratch.file());
    let mut inventory = sample();
    inventory.begin_power_cycle("robot-a", 5_000);
    inventory.end_power_cycle("robot-a", 6_000);
    store.save(&inventory).unwrap();

    let (loaded, _) = store.load();
    assert_eq!(loaded.power_cycles("robot-a").len(), 1);
    assert_eq!(loaded.power_cycles("robot-a")[0].ended_ms, Some(6_000));
}

/// Saves a sample, then edits its JSON, the way a newer Atlas would have
/// written it.
fn save_then_edit(scratch: &Scratch, edit: impl FnOnce(&mut serde_json::Value)) {
    let store = JsonFileStore::new(scratch.file());
    let mut inventory = sample();
    inventory.observe(
        Identity {
            key: DeviceKey::new("sample-soc", "2000beef"),
            model: "sample-board".into(),
            state: atlas_types::DeviceState::Running,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId::new("eth:enp3s0"),
            address: "10.53.38.12".into(),
            attributes: BTreeMap::new(),
        },
        LinkKind::Ethernet,
        CapabilitySet::new(),
        1_000,
    );
    store.save(&inventory).unwrap();
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(scratch.file()).unwrap()).unwrap();
    edit(&mut json);
    fs::write(scratch.file(), serde_json::to_vec(&json).unwrap()).unwrap();
}

#[test]
fn unknown_values_inside_a_record_drop_only_those_values() {
    let scratch = Scratch::new("unknown-values");
    save_then_edit(&scratch, |json| {
        let device = &mut json["devices"][0];
        device["capabilities"]["kinds"] = serde_json::json!(["update", "from-the-future"]);
        device["links"][0]["kind"] = serde_json::json!("from-the-future");
        device["history"][0]["kind"] = serde_json::json!("from-the-future");
    });

    let (loaded, outcome) = JsonFileStore::new(scratch.file()).load();
    assert_eq!(
        outcome,
        LoadOutcome::Restored {
            skipped_records: 0,
            dropped_values: 3
        }
    );
    let record = loaded
        .get(&DeviceKey::new("sample-soc", "1000abcd"))
        .unwrap();
    assert!(
        record
            .capabilities
            .grants(atlas_types::CapabilityKind::Update)
    );
    assert!(
        record.links.is_empty(),
        "the one link was of an unknown kind"
    );
    assert_eq!(
        record.history.len(),
        1,
        "only the unknown history entry is dropped"
    );
    assert_eq!(
        record.label.as_deref(),
        Some("cam"),
        "the rest of the record is kept"
    );
    assert_eq!(loaded.all().len(), 2, "the file is not set aside");
}

#[test]
fn an_unreadable_record_costs_only_that_record() {
    let scratch = Scratch::new("unknown-state");
    save_then_edit(&scratch, |json| {
        json["devices"][1]["identity"]["state"] = serde_json::json!("warp-drive");
    });

    let (loaded, outcome) = JsonFileStore::new(scratch.file()).load();
    assert_eq!(
        outcome,
        LoadOutcome::Restored {
            skipped_records: 1,
            dropped_values: 0
        }
    );
    assert!(
        loaded
            .get(&DeviceKey::new("sample-soc", "1000abcd"))
            .is_some()
    );
    assert!(
        loaded
            .get(&DeviceKey::new("sample-soc", "2000beef"))
            .is_none()
    );
    assert!(scratch.file().exists(), "a readable file is not set aside");
}

#[test]
fn an_unknown_robot_or_alias_value_costs_only_itself() {
    let scratch = Scratch::new("unknown-alias");
    save_then_edit(&scratch, |json| {
        json["aliases"] = serde_json::json!([{"alias": 5}]);
        json["robots"] = serde_json::json!([{"name": 7}]);
    });
    let (loaded, outcome) = JsonFileStore::new(scratch.file()).load();
    assert_eq!(
        outcome,
        LoadOutcome::Restored {
            skipped_records: 0,
            dropped_values: 2
        }
    );
    assert_eq!(loaded.all().len(), 2);
}
