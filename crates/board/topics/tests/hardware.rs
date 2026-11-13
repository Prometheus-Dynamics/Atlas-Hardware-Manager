//! The hardware reader against the simulated node: a device the crate has never heard of shows
//! its readings with their units; a session drop is stale and its return says everything again;
//! a republish that withdraws the devices does not remove them; logs are followed.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use atlas_session::{Handle, Pause, Presence, Sessions};
use atlas_sim_node::readings::Change;
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_topics::description::DescriptionState;
use atlas_topics::{
    Availability, Connection, Contract, Control, Device, DeviceState, Freshness, Hardware,
    ItemKind, Range, Reading, ReadingKind, Reported, Scalar, Snapshot, UnitState, Vector,
};
use atlas_types::LinkId;
use orion::Address;

/// A pause that does not wait: the failure rule's delays are tested on the session side. It
/// only yields, so the monitor thread does not spin while the node is gone.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("topics-{name}"))
}

fn start(dir: &Path, port: Option<u16>) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
    start_again(dir, port)
}

/// The same node in the same directory: its key is kept, so it is the same board.
fn start_again(dir: &Path, port: Option<u16>) -> SimNode {
    let kind = Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    });
    SimNode::start_with(dir, &kind, port).expect("the simulated node starts")
}

fn address(port: u16) -> Address {
    Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap()
}

fn link() -> LinkId {
    LinkId::new("usbnet:test")
}

/// Waits until the handle's monitor reports the node connected.
async fn connected(handle: &Handle) {
    let mut presence = handle.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor is running");
}

/// Reads the hardware until `check` says the snapshot is what the test waits for.
async fn read_until(
    hardware: &Hardware,
    what: &str,
    check: impl Fn(&Snapshot) -> bool,
) -> Snapshot {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let snapshot = hardware.snapshot().await;
        if check(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn device<'a>(snapshot: &'a Snapshot, name: &str) -> Option<&'a Device> {
    snapshot.devices.iter().find(|device| device.name == name)
}

/// Every device the node lists is here, listed, and every item has its value.
fn all_read(snapshot: &Snapshot) -> bool {
    snapshot.devices.len() == 5
        && snapshot.devices.iter().all(|device| {
            device.listed
                && !device.readings.is_empty()
                && device.readings.iter().all(|item| item.value.is_some())
        })
}

fn item<'a>(device: &'a Device, name: &str) -> &'a Reading {
    device
        .readings
        .iter()
        .find(|item| item.name == name)
        .unwrap_or_else(|| panic!("{} has no item {name}", device.name))
}

#[tokio::test]
async fn a_device_the_crate_has_never_heard_of_shows_its_readings_with_their_units() {
    let dir = scratch("unknown");
    let node = start(&dir, None);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());

    let snapshot = read_until(&hardware, "every reading", all_read).await;

    assert_eq!(snapshot.connection, Connection::Live);
    let identity = snapshot.identity.as_ref().expect("the node's identity");
    assert_eq!(identity.model.as_deref(), Some("test-model"));

    // The vane: its description (label, bus, address, state, kind, contract, generation), and its
    // health from the status topic.
    let vane = device(&snapshot, "lemnos.vane0").expect("the vane is listed");
    assert!(matches!(vane.description, DescriptionState::Described(_)));
    assert_eq!(vane.shown_name(), "Wind vane");
    assert_eq!(vane.bus, Reported::Value("i2c".into()));
    assert_eq!(vane.address, Reported::Value(0x44));
    assert_eq!(vane.state, Reported::Value(DeviceState::Running));
    assert_eq!(vane.kind, Reported::Value("sensor".into()));
    assert_eq!(
        vane.location,
        Reported::NotReported,
        "the description gives no location"
    );
    assert_eq!(vane.generation, Reported::Value(1));
    assert_eq!(
        vane.lease_seconds,
        Reported::NotReported,
        "no lease period at e5c0ff0e"
    );
    assert_eq!(
        vane.contracts,
        vec![Contract {
            index: 0,
            tag: Reported::Value("sim.vane/1".into()),
            kind: Reported::Value("sensor".into()),
            hash: Reported::Value(0x51a0),
        }]
    );
    assert_eq!(vane.availability, Reported::Value(Availability::Ready));
    assert_eq!(vane.health, Reported::Value("ok".into()));
    assert_eq!(vane.health_reason, Reported::NotReported);
    assert_eq!(vane.state_reason, Reported::NotReported);

    // Each reading kind the description names, with its unit state, unit, range, step and limits.
    let angle = item(vane, "angle");
    assert_eq!(angle.reading, Reported::Value(ReadingKind::Number));
    assert_eq!(angle.item_kind, Reported::Value(ItemKind::Value));
    assert_eq!(angle.unit_state, UnitState::Known);
    assert_eq!(angle.unit, Reported::Value("deg".into()));
    assert_eq!(
        angle.range,
        Reported::Value(Range {
            min: 0.0,
            max: 360.0
        })
    );
    match angle.value {
        Some(Scalar::Number(degrees)) => assert!((0.0..360.0).contains(&degrees), "{degrees}"),
        ref other => panic!("the angle is a number: {other:?}"),
    }
    assert_eq!(angle.settable, Reported::Value(false), "a value is not set");

    // A control: the exponent scales its figures, and its setter is the method it calls.
    let speed = item(vane, "speed");
    assert_eq!(speed.item_kind, Reported::Value(ItemKind::Control));
    assert_eq!(speed.reading, Reported::Value(ReadingKind::Range));
    assert_eq!(
        speed.range,
        Reported::Value(Range {
            min: 0.0,
            max: 30.0
        }),
        "integer range 0 to 300 at exponent -1"
    );
    assert_eq!(speed.step, Reported::Value(0.1));
    assert_eq!(speed.warn_high, Reported::Value(25.0));
    assert_eq!(
        speed.warn_low,
        Reported::NotReported,
        "no lower limit is given"
    );
    assert_eq!(speed.settable, Reported::Value(true));
    assert_eq!(speed.setter, Reported::Value("speed".into()));
    assert_eq!(
        vane.controls,
        vec![
            Control {
                name: "speed".into(),
                takes: "value:f64".into()
            },
            Control {
                name: "heater".into(),
                takes: "on:bool".into()
            },
        ]
    );

    // A switch has no unit: it is dimensionless, which is not the same as an unknown unit.
    let heater = item(vane, "heater");
    assert_eq!(heater.reading, Reported::Value(ReadingKind::Switch));
    assert_eq!(heater.unit_state, UnitState::Dimensionless);
    assert_eq!(heater.unit, Reported::NotReported);
    assert!(matches!(heater.value, Some(Scalar::Switch(_))));

    // A value whose unit the description does not give is unknown, not dimensionless and not "".
    let level = item(vane, "level");
    assert_eq!(level.unit_state, UnitState::Unknown);
    assert_eq!(level.unit, Reported::NotReported);
    assert_eq!(level.value, Some(Scalar::Number(0.5)));

    // The gauge has no label, so it is shown by its name. It is retrying: a state of the device
    // with its reason, not a fault. Retrying is current.
    let gauge = device(&snapshot, "lemnos.gauge1").expect("the gauge is listed");
    assert_eq!(gauge.shown_name(), "lemnos.gauge1");
    assert_eq!(gauge.label, Reported::NotReported);
    assert_eq!(gauge.address, Reported::NotReported);
    assert_eq!(gauge.state, Reported::Value(DeviceState::Retrying));
    assert_eq!(
        gauge.state_reason,
        Reported::Value("sensor drifting".into())
    );
    assert_eq!(gauge.health, Reported::Value("degraded".into()));
    assert_eq!(
        gauge.freshness,
        Freshness::Live,
        "retrying is current, not stale"
    );
    assert_eq!(gauge.availability, Reported::Value(Availability::Failed));
    let pressure = item(gauge, "pressure");
    assert_eq!(pressure.unit, Reported::Value("kPa".into()));
    assert_eq!(pressure.warn_high, Reported::Value(105.0));
    assert_eq!(item(gauge, "temperature").warn_low, Reported::Value(5.0));

    // The unit: a three-axis group (its parts, in the description's order), a colour array, a
    // range with real values that win over the integer one, and text.
    let unit = device(&snapshot, "lemnos.unit3").expect("the unit is listed");
    assert_eq!(unit.shown_name(), "Motion unit");
    assert_eq!(unit.address, Reported::Value(104));
    assert_eq!(
        unit.vectors,
        vec![Vector {
            name: "accel".into(),
            members: vec!["accel_x".into(), "accel_y".into(), "accel_z".into()],
        }]
    );
    assert_eq!(
        item(unit, "accel_x").reading,
        Reported::Value(ReadingKind::ThreeAxes)
    );
    assert_eq!(item(unit, "accel_x").group, Reported::Value("accel".into()));
    let colour = item(unit, "colour");
    assert_eq!(colour.reading, Reported::Value(ReadingKind::ColourArray));
    assert_eq!(colour.unit_state, UnitState::Dimensionless);
    assert_eq!(colour.value, Some(Scalar::Numbers(vec![0.2, 0.4, 0.6])));
    let position = item(unit, "position");
    assert_eq!(
        position.range,
        Reported::Value(Range {
            min: 0.5,
            max: 99.5
        }),
        "the real range wins over 0 to 100"
    );
    assert_eq!(
        position.step,
        Reported::Value(0.5),
        "the real step wins over 5"
    );
    assert_eq!(position.settable, Reported::Value(true));
    assert_eq!(position.setter, Reported::Value("position".into()));
    assert_eq!(item(unit, "note").value, Some(Scalar::Text("ready".into())));

    // The off device: the node lists it, but a device that is off has no description (the bridge
    // refuses describe: "the device isn't running"). Its readings are still shown.
    let off = device(&snapshot, "lemnos.off0").expect("the off device is listed");
    assert_eq!(
        off.description,
        DescriptionState::NotAvailable("the device isn't running".into())
    );
    assert_eq!(off.state, Reported::NotReported);
    assert_eq!(off.availability, Reported::Value(Availability::Pending));
    assert_eq!(item(off, "count").unit_state, UnitState::Unknown);
    assert!(item(off, "count").value.is_some());

    // The bare device: its description gives every optional field as unknown or empty. It is
    // listed, its value reads, and every optional field is not reported.
    let bare = device(&snapshot, "lemnos.bare0").expect("the bare device is listed");
    assert!(bare.listed);
    assert_eq!(bare.freshness, Freshness::Live);
    assert!(matches!(bare.description, DescriptionState::Described(_)));
    assert_eq!(bare.label, Reported::NotReported);
    assert_eq!(bare.bus, Reported::NotReported);
    assert_eq!(bare.state, Reported::NotReported);
    assert_eq!(bare.state_reason, Reported::NotReported);
    assert_eq!(bare.address, Reported::NotReported);
    assert_eq!(bare.kind, Reported::NotReported);
    assert!(bare.contracts.is_empty());
    assert_eq!(bare.health, Reported::NotReported);
    let count = item(bare, "count");
    assert_eq!(count.reading, Reported::NotReported);
    assert_eq!(count.unit_state, UnitState::Unknown);
    assert_eq!(count.range, Reported::NotReported);
    assert_eq!(count.step, Reported::NotReported);
    assert_eq!(count.warn_high, Reported::NotReported);
    assert_eq!(
        count.settable,
        Reported::Value(false),
        "kind value: not set"
    );
    assert_eq!(count.value, Some(Scalar::Number(1.0)));
    assert_eq!(
        snapshot.connection,
        Connection::Live,
        "no device is a fault of the node"
    );
}

/// A description is asked for once when the connection first sees a device, and again only when
/// its generation moves: a state change asks once for that device, and nothing else asks.
#[tokio::test]
async fn a_description_is_asked_once_and_again_only_when_its_generation_moves() {
    let dir = scratch("generation");
    let node = start(&dir, None);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    read_until(&hardware, "every reading", all_read).await;
    let asked = |name: &str| node.runs_started(&format!("{name}.call"));
    assert_eq!(asked("lemnos.vane0"), 1, "asked once at connect");
    assert_eq!(asked("lemnos.gauge1"), 1);

    // Steady state: several reads, and no ask again.
    for _ in 0..5 {
        let _ = hardware.snapshot().await;
    }
    assert_eq!(asked("lemnos.vane0"), 1, "no ask on a read with no change");

    // The vane's state and reason change: its generation moves once, and it is asked once.
    node.change_device(
        "vane0",
        Change::State {
            state: 1,
            reason: "sensor drifting".into(),
        },
    );
    let changed = read_until(&hardware, "the vane's new description", |snapshot| {
        device(snapshot, "lemnos.vane0").is_some_and(|device| {
            device.state == Reported::Value(DeviceState::Retrying)
                && device.generation == Reported::Value(2)
        })
    })
    .await;
    assert_eq!(
        device(&changed, "lemnos.vane0")
            .expect("the vane")
            .state_reason,
        Reported::Value("sensor drifting".into())
    );
    assert_eq!(asked("lemnos.vane0"), 2, "one more ask for the one change");
    assert_eq!(
        asked("lemnos.gauge1"),
        1,
        "the other devices are not asked again"
    );
    for _ in 0..5 {
        let _ = hardware.snapshot().await;
    }
    assert_eq!(
        asked("lemnos.vane0"),
        2,
        "the change is asked once, not on every read"
    );
}

/// A board whose bridge does not answer `describe` (the lock at d6b9361d): every device is listed
/// and its readings are shown, with the description "not available", and nothing is asked again
/// until the app refreshes. The refresh asks every device again.
#[tokio::test]
async fn a_board_that_refuses_describe_shows_its_devices_as_not_available_until_a_refresh() {
    let dir = scratch("refused");
    let node = start(&dir, None);
    node.refuse_describe(true);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    let snapshot = read_until(
        &hardware,
        "the devices shown with their readings",
        |snapshot| {
            all_read(snapshot)
                && snapshot
                    .devices
                    .iter()
                    .all(|device| matches!(device.description, DescriptionState::NotAvailable(_)))
        },
    )
    .await;
    let vane = device(&snapshot, "lemnos.vane0").expect("the vane is listed");
    assert_eq!(
        vane.description,
        DescriptionState::NotAvailable("this board does not answer describe".into())
    );
    assert_eq!(vane.label, Reported::NotReported, "no labels stand in");
    assert_eq!(vane.kind, Reported::NotReported);
    assert_eq!(item(vane, "angle").unit_state, UnitState::Unknown);
    assert!(
        item(vane, "angle").value.is_some(),
        "the topics still show the reading"
    );
    // The first ask may come before this connection has the generation topic's first sample. A
    // refusal then holds no generation, and the sample arriving is a change: one more ask, and then
    // nothing, however many reads follow.
    let mut asked = node.runs_started("lemnos.vane0.call");
    assert!(
        (1..=2).contains(&asked),
        "asked once, and once more when its generation arrives: {asked}"
    );
    for _ in 0..5 {
        let _ = hardware.snapshot().await;
    }
    asked = node.runs_started("lemnos.vane0.call");
    for _ in 0..5 {
        let _ = hardware.snapshot().await;
    }
    assert_eq!(
        node.runs_started("lemnos.vane0.call"),
        asked,
        "no retry on a timer"
    );

    // The board answers again, and the app asks: every device is described.
    node.refuse_describe(false);
    hardware.refresh_descriptions();
    read_until(
        &hardware,
        "the vane described after a refresh",
        |snapshot| {
            device(snapshot, "lemnos.vane0")
                .is_some_and(|device| matches!(device.description, DescriptionState::Described(_)))
        },
    )
    .await;
    assert_eq!(
        node.runs_started("lemnos.vane0.call"),
        asked + 1,
        "one ask on the refresh"
    );
}

/// An off device comes back to running: its generation moves, and it is described again.
#[tokio::test]
async fn a_device_that_comes_back_from_off_is_described_again() {
    let dir = scratch("back");
    let node = start(&dir, None);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    read_until(&hardware, "every reading", all_read).await;
    node.change_device(
        "off0",
        Change::State {
            state: 0,
            reason: String::new(),
        },
    );
    let back = read_until(
        &hardware,
        "the off device running and described",
        |snapshot| {
            device(snapshot, "lemnos.off0")
                .is_some_and(|device| matches!(device.description, DescriptionState::Described(_)))
        },
    )
    .await;
    let off = device(&back, "lemnos.off0").expect("the device");
    assert_eq!(off.state, Reported::Value(DeviceState::Running));
    assert_eq!(off.availability, Reported::Value(Availability::Ready));
    assert_eq!(off.generation, Reported::Value(2));
}

#[tokio::test]
async fn a_session_drop_is_stale_since_its_last_read_and_the_return_says_everything_again() {
    let dir = scratch("drop");
    let node = start(&dir, None);
    let port = node.network().port();
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions.handle(&address(port)).expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    read_until(&hardware, "every reading", all_read).await;

    drop(node);
    let stale = read_until(&hardware, "the stale read", |snapshot| {
        matches!(snapshot.connection, Connection::Stale { .. })
    })
    .await;
    let Connection::Stale { since, reason } = stale.connection.clone() else {
        panic!("stale");
    };
    assert!(!reason.is_empty(), "the reason is said");
    assert_eq!(stale.devices.len(), 5, "a lost session removes nothing");
    for device in &stale.devices {
        assert_eq!(
            device.freshness,
            Freshness::Stale { since },
            "{} is stale since the last read",
            device.name
        );
    }

    // The node comes back on the same port: the session is made again, and everything is said
    // again. The readings are current again once the new session has read them. It is the same
    // node: its directory, and so its key, is kept.
    let node = start_again(&dir, Some(port));
    let back = read_until(
        &hardware,
        "the return, with every device current",
        |snapshot| {
            snapshot.connection == Connection::Live
                && all_read(snapshot)
                && snapshot
                    .devices
                    .iter()
                    .all(|device| device.freshness == Freshness::Live)
        },
    )
    .await;
    assert_eq!(back.devices.len(), 5);

    // A line written on the new node reaches a subscriber that has just been made. A line
    // written before the subscription is live is not sent to a network subscriber (see the
    // LOCK note in the simulated readings), so the line is written again until it arrives.
    let deadline = Instant::now() + DEADLINE;
    loop {
        node.model().log("after the return");
        let snapshot = hardware.snapshot().await;
        if snapshot
            .logs
            .iter()
            .any(|line| line.text == "after the return")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the log line never came: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_republish_that_withdraws_the_devices_is_held_not_faulted_and_not_removed() {
    let dir = scratch("blip");
    let node = start(&dir, None);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    read_until(&hardware, "every reading", all_read).await;

    node.hold_devices(true);
    let held = read_until(&hardware, "the devices held", |snapshot| {
        device(snapshot, "lemnos.vane0").is_some_and(|device| !device.listed)
    })
    .await;
    assert_eq!(held.devices.len(), 5, "the blip removes no device");
    let vane = device(&held, "lemnos.vane0").expect("still here");
    assert!(matches!(vane.freshness, Freshness::Stale { .. }));
    assert_eq!(
        vane.health,
        Reported::Value("ok".into()),
        "a blip is not a fault"
    );
    assert_eq!(
        held.connection,
        Connection::Live,
        "the node is still connected"
    );

    node.hold_devices(false);
    let back = read_until(&hardware, "the devices listed again", |snapshot| {
        device(snapshot, "lemnos.vane0").is_some_and(|device| device.listed) && all_read(snapshot)
    })
    .await;
    assert!(
        device(&back, "lemnos.vane0").is_some_and(|device| device.freshness == Freshness::Live)
    );
}

#[tokio::test]
async fn a_device_withdrawn_for_longer_than_the_grace_is_removed() {
    let dir = scratch("grace");
    let node = start(&dir, None);
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::with_grace(handle, link(), Duration::from_millis(300));
    read_until(&hardware, "every reading", all_read).await;

    node.hold_devices(true);
    read_until(
        &hardware,
        "the devices removed after the grace",
        |snapshot| snapshot.devices.is_empty(),
    )
    .await;
}

#[tokio::test]
async fn a_node_with_no_devices_is_read_without_devices() {
    let dir = scratch("plain");
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start_with(&dir, &Kind::Plain, None).expect("a plain node starts");
    let sessions = Sessions::with_call_wait(Arc::new(NoPause), Duration::from_secs(5));
    let handle = sessions
        .handle(&address(node.network().port()))
        .expect("the node is on");
    connected(&handle).await;
    let hardware = Hardware::new(handle, link());
    let snapshot = read_until(&hardware, "a read", |snapshot| {
        snapshot.connection == Connection::Live
    })
    .await;
    assert!(snapshot.devices.is_empty(), "a plain node lists no devices");
    assert_eq!(snapshot.connection, Connection::Live);
}
