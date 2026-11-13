//! Folding: one board seen under a boot ROM key and a running key is one
//! record, the fold survives a restart, and a later sighting under the alias
//! updates the same record.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use atlas_inventory::{HistoryKind, Inventory, JsonFileStore, Outcome};
use atlas_types::{
    CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Identity, LinkId, LinkKind,
};

const T0: u64 = 1_000_000;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn running() -> DeviceKey {
    DeviceKey::new("sample-soc", "1000abcd")
}

fn boot_rom() -> DeviceKey {
    DeviceKey::new("sample-soc", "port-1-2")
}

fn identity(
    key: &DeviceKey,
    link: &str,
    address: &str,
    state: DeviceState,
    version: &str,
) -> Identity {
    Identity {
        key: key.clone(),
        model: "sample-board".into(),
        state,
        lemnos: atlas_types::LemnosState::Absent,
        versions: BTreeMap::from([("os".to_string(), version.to_string())]),
        name: None,
        link: LinkId::new(link),
        address: address.into(),
        attributes: BTreeMap::new(),
    }
}

fn caps() -> CapabilitySet {
    CapabilitySet::from_kinds([CapabilityKind::Update])
}

/// A boot ROM record (USB boot) and the running board (Ethernet), then folded.
fn folded() -> Inventory {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity(
            &boot_rom(),
            "usbboot:1",
            "port-1-2",
            DeviceState::BootRom,
            "0",
        ),
        LinkKind::UsbBoot,
        CapabilitySet::new(),
        T0,
    );
    inventory.observe(
        identity(
            &running(),
            "eth:enp3s0",
            "10.53.38.11",
            DeviceState::Running,
            "1.0.0",
        ),
        LinkKind::Ethernet,
        caps(),
        T0 + 60_000,
    );
    assert!(inventory.fold(&boot_rom(), &running(), T0 + 61_000));
    inventory
}

#[test]
fn a_boot_rom_record_folded_into_a_running_board_is_one_record() {
    let inventory = folded();
    assert_eq!(inventory.all().len(), 1);
    assert!(
        inventory.get(&boot_rom()).is_some(),
        "the alias finds the record"
    );
    assert_eq!(inventory.get(&boot_rom()).unwrap().key, running());

    let record = inventory.get(&running()).unwrap();
    assert_eq!(record.links.len(), 2, "both links are kept");
    assert_eq!(record.first_seen_ms, T0, "first seen is the earlier one");
    assert!(
        record
            .history
            .iter()
            .any(|entry| entry.kind == HistoryKind::Folded)
    );
    assert_eq!(inventory.aliases(), vec![(&boot_rom(), &running())]);
}

#[test]
fn the_fold_survives_save_and_load() {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("atlas-inventory-fold-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let store = JsonFileStore::new(dir.join("inventory.json"));
    store.save(&folded()).unwrap();

    let (mut loaded, _) = store.load();
    assert_eq!(loaded.all().len(), 1);
    assert_eq!(loaded.aliases(), vec![(&boot_rom(), &running())]);

    // A sighting under the alias after the restart lands on the same record.
    let outcome = loaded.observe(
        identity(
            &boot_rom(),
            "usbnet:0",
            "172.31.250.2",
            DeviceState::Running,
            "1.1.0",
        ),
        LinkKind::UsbNetwork,
        caps(),
        T0 + 120_000,
    );
    assert_eq!(
        outcome,
        Outcome::Returned,
        "every record is offline after a restart"
    );
    assert_eq!(loaded.all().len(), 1);
    let record = loaded.get(&running()).unwrap();
    assert_eq!(record.version(), Some("1.1.0"));
    assert_eq!(record.key, running());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_later_sighting_under_the_alias_updates_the_same_record() {
    let mut inventory = folded();
    let outcome = inventory.observe(
        identity(
            &boot_rom(),
            "eth:enp3s0",
            "10.53.38.11",
            DeviceState::Running,
            "1.2.0",
        ),
        LinkKind::Ethernet,
        caps(),
        T0 + 200_000,
    );
    assert_eq!(outcome, Outcome::Updated);
    assert_eq!(inventory.all().len(), 1, "no new record under the alias");
    let record = inventory.get(&running()).unwrap();
    assert_eq!(
        record.identity.key,
        running(),
        "the identity is the record's own key"
    );
    assert_eq!(record.version(), Some("1.2.0"));
    assert!(
        record
            .history
            .iter()
            .any(|entry| entry.kind == HistoryKind::Changed)
    );
}

#[test]
fn an_alias_that_is_seen_as_absent_marks_the_record_absent() {
    let mut inventory = folded();
    let seen = BTreeSet::from([boot_rom()]);
    assert!(
        inventory.mark_absent(&seen, T0 + 300_000).is_empty(),
        "seen under the alias"
    );
    let gone = inventory.mark_absent(&BTreeSet::new(), T0 + 400_000);
    assert_eq!(gone, vec![running()]);
}

#[test]
fn unfold_makes_the_alias_a_new_record_again() {
    let mut inventory = folded();
    assert!(inventory.unfold(&boot_rom(), T0 + 500_000));
    assert!(!inventory.unfold(&boot_rom(), T0 + 500_000), "only once");

    let outcome = inventory.observe(
        identity(
            &boot_rom(),
            "usbboot:1",
            "port-1-2",
            DeviceState::BootRom,
            "0",
        ),
        LinkKind::UsbBoot,
        CapabilitySet::new(),
        T0 + 600_000,
    );
    assert_eq!(outcome, Outcome::New);
    assert_eq!(inventory.all().len(), 2);
    assert_eq!(inventory.get(&boot_rom()).unwrap().links.len(), 1);
}

#[test]
fn fold_refuses_what_it_cannot_do() {
    let mut inventory = folded();
    let stranger = DeviceKey::new("sample-soc", "9999ffff");
    assert!(!inventory.fold(&stranger, &running(), T0), "unknown alias");
    assert!(!inventory.fold(&running(), &running(), T0), "into itself");
    assert!(
        !inventory.fold(&boot_rom(), &running(), T0),
        "already folded"
    );
    assert!(
        !inventory.fold(&boot_rom(), &stranger, T0),
        "unknown target"
    );
    assert_eq!(inventory.all().len(), 1);
}

#[test]
fn folds_chain_to_the_final_record() {
    let mut inventory = folded();
    let third = DeviceKey::new("sample-soc", "3000cafe");
    inventory.observe(
        identity(
            &third,
            "usbnet:9",
            "172.31.250.9",
            DeviceState::Running,
            "1.0.0",
        ),
        LinkKind::UsbNetwork,
        caps(),
        T0 + 70_000,
    );
    // Folding the running record into the third one re-points the boot ROM key too.
    assert!(inventory.fold(&running(), &third, T0 + 80_000));
    assert_eq!(inventory.all().len(), 1);
    assert_eq!(inventory.resolve(&boot_rom()), third);
    assert_eq!(inventory.resolve(&running()), third);
}

#[test]
fn forgetting_a_record_forgets_its_aliases() {
    let mut inventory = folded();
    assert!(inventory.forget(&running()).is_some());
    assert!(inventory.aliases().is_empty());
    assert!(inventory.get(&boot_rom()).is_none());
}
