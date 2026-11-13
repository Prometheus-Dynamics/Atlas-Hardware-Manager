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
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_topics::{
    Availability, Connection, Device, Freshness, Hardware, ReadingKind, Reported, Scalar, Snapshot,
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

/// Every device the node lists is here, listed, and every reading has its value.
fn all_read(snapshot: &Snapshot) -> bool {
    snapshot.devices.len() == 2
        && snapshot.devices.iter().all(|device| {
            device.listed
                && !device.readings.is_empty()
                && device
                    .readings
                    .iter()
                    .all(|reading| reading.value.is_some())
        })
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
    assert_eq!(identity.model, "test-model");

    // The vane: its driver, its health, and each reading with its unit.
    let vane = device(&snapshot, "lemnos.vane0").expect("the vane is listed");
    assert_eq!(vane.driver, Reported::Value("sim-vane".into()));
    assert_eq!(vane.availability, Reported::Value(Availability::Ready));
    assert_eq!(vane.state, Reported::Value("running".into()));
    assert_eq!(vane.health, Reported::Value("ok".into()));
    assert_eq!(
        vane.reason,
        Reported::NotReported,
        "a healthy device gives no reason"
    );
    assert_eq!(
        vane.label,
        Reported::NotReported,
        "no human name in the description"
    );
    assert_eq!(
        vane.group,
        Reported::NotReported,
        "no grouping in the description"
    );

    let angle = vane
        .readings
        .iter()
        .find(|r| r.name == "angle")
        .expect("the angle");
    assert_eq!(angle.kind, ReadingKind::Number);
    assert_eq!(angle.unit, Reported::Value("deg".into()));
    match angle.value {
        Some(Scalar::Number(degrees)) => assert!((0.0..360.0).contains(&degrees), "{degrees}"),
        ref other => panic!("the angle is a number: {other:?}"),
    }

    let heater = vane
        .readings
        .iter()
        .find(|r| r.name == "heater")
        .expect("the heater");
    assert_eq!(heater.kind, ReadingKind::Switch);
    assert!(matches!(heater.value, Some(Scalar::Switch(_))));

    // A value whose unit the description does not give reads as not reported, not as "".
    let level = vane
        .readings
        .iter()
        .find(|r| r.name == "level")
        .expect("the level");
    assert_eq!(level.unit, Reported::NotReported);
    assert_eq!(level.value, Some(Scalar::Number(0.5)));

    // The gauge says why it is not healthy.
    let gauge = device(&snapshot, "lemnos.gauge1").expect("the gauge is listed");
    assert_eq!(gauge.health, Reported::Value("degraded".into()));
    assert_eq!(gauge.reason, Reported::Value("sensor drifting".into()));
    let pressure = gauge
        .readings
        .iter()
        .find(|r| r.name == "pressure")
        .expect("pressure");
    assert_eq!(pressure.unit, Reported::Value("kPa".into()));
    assert_eq!(vane.freshness, Freshness::Live);
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
    assert_eq!(stale.devices.len(), 2, "a lost session removes nothing");
    for device in &stale.devices {
        assert_eq!(
            device.freshness,
            Freshness::Stale { since },
            "{} is stale since the last read",
            device.name
        );
    }

    // The node comes back on the same port: the session is made again, and everything is said
    // again. The readings are current again once the new session has read them.
    let node = start(&dir, Some(port));
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
    assert_eq!(back.devices.len(), 2);

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
    assert_eq!(held.devices.len(), 2, "the blip removes no device");
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
