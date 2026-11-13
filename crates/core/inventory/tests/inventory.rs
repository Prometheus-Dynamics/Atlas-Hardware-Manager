//! Proof tests: one record per board across links and states, and health
//! that ignores robot power cycles.

use std::collections::{BTreeMap, BTreeSet};

use atlas_inventory::{
    FAULT_AFTER_MS, Health, HistoryKind, Inventory, Outcome, POWER_CYCLE_LEAD_MS, RETURN_GRACE_MS,
    Robot,
};
use atlas_types::{
    CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Identity, LinkId, LinkKind,
};

const T0: u64 = 1_000_000;

fn key() -> DeviceKey {
    DeviceKey::new("sample-soc", "1000abcd")
}

fn identity(link: &str, address: &str, state: DeviceState, version: &str) -> Identity {
    Identity {
        key: key(),
        model: "sample-board".into(),
        state,
        lemnos: atlas_types::LemnosState::Absent,
        versions: BTreeMap::from([("os".to_string(), version.to_string())]),
        name: Some("front camera".into()),
        link: LinkId::new(link),
        address: address.into(),
        attributes: BTreeMap::new(),
    }
}

fn caps() -> CapabilitySet {
    CapabilitySet::from_kinds([CapabilityKind::Update, CapabilityKind::Logs])
}

fn gateway() -> LinkKind {
    LinkKind::Gateway {
        via: DeviceKey::new("sample-controller", "c0ffee"),
    }
}

/// One board, seen over USB, the network and behind a controller, is one
/// record with three links; the best live link is the one it follows.
#[test]
fn one_board_on_three_links_is_one_record() {
    let mut inventory = Inventory::new();
    let usb = inventory.observe(
        identity("usbnet:0", "172.31.250.2", DeviceState::Running, "1.0.0"),
        LinkKind::UsbNetwork,
        caps(),
        T0,
    );
    let network = inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0 + 1_000,
    );
    let behind = inventory.observe(
        identity("gw:controller/1", "10.0.9.4", DeviceState::Running, "1.0.0"),
        gateway(),
        caps(),
        T0 + 2_000,
    );

    assert_eq!(usb, Outcome::New);
    assert_eq!(network, Outcome::Unchanged);
    assert_eq!(behind, Outcome::Unchanged);
    assert_eq!(inventory.all().len(), 1, "one board, one record");

    let record = inventory.get(&key()).unwrap();
    assert_eq!(
        record.links.len(),
        3,
        "every link it was seen through is kept"
    );
    assert_eq!(record.followed_link(), &LinkId::new("usbnet:0"));
    assert_eq!(record.identity.address, "172.31.250.2");
    assert_eq!(record.last_seen_ms, T0 + 2_000);
}

/// A board in a boot ROM and then running is the same record: the state
/// changes, the record and its history stay.
#[test]
fn states_share_one_record_and_keep_history() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("usbboot:1", "port-1-2", DeviceState::BootRom, "0"),
        LinkKind::UsbBoot,
        CapabilitySet::new(),
        T0,
    );
    let outcome = inventory.observe(
        identity("usbnet:0", "172.31.250.2", DeviceState::Running, "1.0.0"),
        LinkKind::UsbNetwork,
        caps(),
        T0 + 60_000,
    );

    assert_eq!(outcome, Outcome::Updated, "a better link took over");
    assert_eq!(inventory.all().len(), 1);
    let record = inventory.get(&key()).unwrap();
    assert_eq!(record.identity.state, DeviceState::Running);
    let kinds: Vec<HistoryKind> = record.history.iter().map(|h| h.kind).collect();
    assert_eq!(
        kinds,
        vec![
            HistoryKind::Found,
            HistoryKind::LinkChanged,
            HistoryKind::Changed
        ]
    );
}

/// The followed link is kept while it answers; a worse link does not take
/// the record over.
#[test]
fn a_worse_link_does_not_take_over_a_live_one() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("usbnet:0", "172.31.250.2", DeviceState::Running, "1.0.0"),
        LinkKind::UsbNetwork,
        caps(),
        T0,
    );
    let outcome = inventory.observe(
        identity("gw:controller/1", "10.0.9.4", DeviceState::Running, "1.0.0"),
        gateway(),
        caps(),
        T0 + 1_000,
    );
    assert_eq!(outcome, Outcome::Unchanged);
    assert_eq!(
        inventory.get(&key()).unwrap().followed_link(),
        &LinkId::new("usbnet:0")
    );
}

/// When the followed link goes silent, the next link that answers takes
/// over, and the record's history says so.
#[test]
fn a_silent_link_hands_over_to_the_next() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("usbnet:0", "172.31.250.2", DeviceState::Running, "1.0.0"),
        LinkKind::UsbNetwork,
        caps(),
        T0,
    );
    let outcome = inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0 + 120_000,
    );
    assert_eq!(outcome, Outcome::Updated);
    assert_eq!(
        inventory.get(&key()).unwrap().followed_link(),
        &LinkId::new("eth:enp3s0")
    );
}

/// A new version is a history line, and the record shows the new version.
#[test]
fn a_version_change_is_recorded() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    let outcome = inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.1.0"),
        LinkKind::Ethernet,
        caps(),
        T0 + 5_000,
    );
    assert_eq!(outcome, Outcome::Updated);
    let record = inventory.get(&key()).unwrap();
    assert_eq!(record.version(), Some("1.1.0"));
    assert_eq!(
        record.history.last().unwrap().message,
        "version 1.0.0 to 1.1.0"
    );
}

/// Absence is a fault only when no robot power cycle explains it. A board
/// that disappears with its robot's power cycle and returns with it is not a
/// fault.
#[test]
fn a_power_cycle_is_not_a_fault() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    inventory.set_device_robot(&key(), Some("robot-a".into()), T0);

    let cut = T0 + 10_000;
    inventory.begin_power_cycle("robot-a", cut);
    inventory.mark_absent(&BTreeSet::new(), cut + 1_000);

    let long_after = cut + FAULT_AFTER_MS * 3;
    assert_eq!(
        inventory.health(&key(), long_after),
        Some(Health::Away),
        "still powered off: away, not a fault"
    );

    inventory.end_power_cycle("robot-a", cut + 20_000);
    let back = cut + 40_000;
    let outcome = inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        back,
    );
    assert_eq!(outcome, Outcome::Returned);
    assert_eq!(inventory.health(&key(), back), Some(Health::Ok));
    assert!(inventory.faults(back + FAULT_AFTER_MS * 10).is_empty());
}

/// The same absence with no power cycle is a fault once it lasts.
#[test]
fn an_unexplained_absence_is_a_fault() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    inventory.mark_absent(&BTreeSet::new(), T0 + 1_000);

    assert_eq!(inventory.health(&key(), T0 + 1_000), Some(Health::Away));
    assert_eq!(
        inventory.health(&key(), T0 + FAULT_AFTER_MS + 1_000),
        Some(Health::Fault)
    );
    assert_eq!(inventory.faults(T0 + FAULT_AFTER_MS + 1_000), vec![key()]);
}

/// A board that does not come back within the grace after its robot's power
/// returns is a fault again.
#[test]
fn a_board_that_never_returns_after_a_power_cycle_faults() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    inventory.set_device_robot(&key(), Some("robot-a".into()), T0);
    let cut = T0 + POWER_CYCLE_LEAD_MS / 2;
    inventory.begin_power_cycle("robot-a", cut);
    inventory.mark_absent(&BTreeSet::new(), cut);
    inventory.end_power_cycle("robot-a", cut + 10_000);

    let within = cut + 10_000 + RETURN_GRACE_MS;
    let after = within + 1;
    assert_eq!(inventory.health(&key(), within), Some(Health::Away));
    assert_eq!(inventory.health(&key(), after), Some(Health::Fault));
}

/// A later absence, after the power cycle is over and the board came back,
/// is not explained by that old cycle.
#[test]
fn an_old_power_cycle_does_not_hide_a_later_fault() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    inventory.set_device_robot(&key(), Some("robot-a".into()), T0);
    inventory.begin_power_cycle("robot-a", T0 + 1_000);
    inventory.end_power_cycle("robot-a", T0 + 2_000);
    let back = T0 + 3_000;
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        back,
    );
    inventory.mark_absent(&BTreeSet::new(), back + 1_000);
    assert_eq!(
        inventory.health(&key(), back + 1_000 + FAULT_AFTER_MS + 1),
        Some(Health::Fault)
    );
}

#[test]
fn robots_group_devices_and_robots_group_into_teams() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    let other = DeviceKey::new("sample-soc", "2000beef");
    let mut spare = identity("eth:enp3s0", "10.53.38.12", DeviceState::Running, "1.0.0");
    spare.key = other.clone();
    inventory.observe(spare, LinkKind::Ethernet, caps(), T0);

    inventory.put_robot(Robot {
        name: "robot-a".into(),
        team: Some("team-one".into()),
        notes: None,
    });
    assert!(inventory.set_device_robot(&key(), Some("robot-a".into()), T0));
    assert!(inventory.set_device_robot(&other, Some("robot-b".into()), T0));

    assert_eq!(inventory.robot_members("robot-a"), vec![key()]);
    assert_eq!(
        inventory.teams(),
        BTreeMap::from([("team-one".to_string(), vec!["robot-a".to_string()])])
    );
    assert!(inventory.robot("robot-b").is_some(), "created on join");

    inventory.remove_robot("robot-a");
    assert!(inventory.robot_members("robot-a").is_empty());
    assert!(inventory.teams().is_empty());
}

#[test]
fn history_keeps_the_newest_entries_only() {
    let mut inventory = Inventory::new();
    inventory.observe(
        identity("eth:enp3s0", "10.53.38.11", DeviceState::Running, "1.0.0"),
        LinkKind::Ethernet,
        caps(),
        T0,
    );
    for n in 0..300u64 {
        inventory.set_label(&key(), Some(format!("name {n}")), T0 + n);
    }
    let history = &inventory.get(&key()).unwrap().history;
    assert_eq!(history.len(), atlas_inventory::HISTORY_LIMIT);
    assert_eq!(history.last().unwrap().message, "renamed to name 299");
}

#[test]
fn unknown_devices_have_no_health() {
    let inventory = Inventory::new();
    assert_eq!(inventory.health(&key(), T0), None);
}
