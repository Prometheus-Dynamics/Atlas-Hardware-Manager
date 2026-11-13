//! The board driver against the simulated node (`atlas-sim-node`): Orion's own comm core on a
//! real socket and port, with the hello facts, Lemnos's surface and the stop and start the
//! node really has.

#![cfg(target_os = "linux")]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;

use atlas_board::BoardDriver;
use atlas_driver::{Driver, DriverError, Serves};
use atlas_session::{Pause, Seed};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_types::{
    Candidate, CapabilityKind, DeviceKey, DeviceState, Identity, LemnosState, Link, LinkId,
    LinkKind,
};

const LINK: &str = "usbnet:test0";

/// Records each delay the monitor asks for. It sleeps only briefly, so the tests run at once;
/// the delays are what the failure rule is checked on.
#[derive(Debug, Default)]
struct Delays(Mutex<Vec<Duration>>);

impl Pause for Delays {
    fn pause(&self, delay: Duration, _stop: &AtomicBool) {
        self.0.lock().unwrap().push(delay);
        thread::sleep(Duration::from_millis(2));
    }
}

/// Waits until the monitor has recorded more than `count` pauses: it has backed off that many
/// times. Only [`DEADLINE`] ends the wait.
async fn paused_beyond(pause: &Delays, count: usize) {
    tokio::time::timeout(DEADLINE, async {
        while pause.0.lock().unwrap().len() <= count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the monitor backed off");
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("board-{name}"))
}

fn start(dir: &Path, kind: &Kind, port: Option<u16>) -> SimNode {
    let _ = std::fs::remove_dir_all(dir);
    SimNode::start_with(dir, kind, port).expect("the simulated node starts")
}

fn usb_link() -> Link {
    Link {
        id: LinkId::new(LINK),
        kind: LinkKind::UsbNetwork,
        label: "test USB network".into(),
    }
}

fn driver_for(port: u16, pause: &std::sync::Arc<Delays>) -> BoardDriver {
    let seed = Seed::new(LinkId::new(LINK), &format!("orion+tcp://127.0.0.1:{port}")).unwrap();
    BoardDriver::with_pause(vec![seed], pause.clone())
}

fn running_board() -> Kind {
    Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

/// Polls `discover` until the node is offered, or the wait runs out.
async fn offered(driver: &BoardDriver, link: &Link) -> Candidate {
    tokio::time::timeout(DEADLINE, async {
        loop {
            if let Some(candidate) = driver.discover(link).await.unwrap().into_iter().next() {
                return candidate;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the node is offered by a scan")
}

/// Polls `discover` until the node is no longer offered.
async fn gone(driver: &BoardDriver, link: &Link) {
    tokio::time::timeout(DEADLINE, async {
        loop {
            if driver.discover(link).await.unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the node drops out of a scan");
}

/// Identifies `candidate`, waiting while its monitor catches up with the node's facts.
async fn identify(driver: &BoardDriver, candidate: &Candidate) -> Identity {
    driver.identify(candidate).await.expect("identified")
}

#[tokio::test]
async fn a_node_is_found_identified_lost_and_found_again() {
    let dir = scratch("cycle");
    let first = start(&dir, &running_board(), None);
    let port = first.network().port();
    let pause = std::sync::Arc::new(Delays::default());
    let driver = driver_for(port, &pause);
    let link = usb_link();

    // Found, and identified from the hello document and Lemnos.
    let candidate = offered(&driver, &link).await;
    assert_eq!(candidate.link, LinkId::new(LINK));
    let identity = identify(&driver, &candidate).await;
    assert_eq!(identity.key, DeviceKey::from_static("test-family", "0001"));
    assert_eq!(identity.state, DeviceState::Running);
    assert_eq!(identity.model, "test-model");
    assert!(identity.key.is_board());
    assert_eq!(identity.lemnos, LemnosState::Answering);
    let grants = driver.capabilities(&identity);
    assert!(
        grants.grants(CapabilityKind::Slots),
        "the node offers slots"
    );
    assert!(grants.grants(CapabilityKind::Logs));

    // Lost: the node goes away, and the scan stops offering it.
    let paused_before = pause.0.lock().unwrap().len();
    drop(first);
    gone(&driver, &link).await;
    let lost = driver.identify(&candidate).await;
    assert!(
        matches!(lost, Err(DriverError::Unreachable(_))),
        "a lost node is unreachable, not identified: {lost:?}"
    );
    // The monitor records its wait just after it marks the node lost. Wait for that record
    // before the node comes back, so the wait is seen whatever the machine's pace.
    paused_beyond(&pause, paused_before).await;

    // Found again on the same port, with the same identity.
    // The same board again: the same directory keeps its key.
    let _second = SimNode::start_with(&dir, &running_board(), Some(port)).expect("the node starts");
    let back = offered(&driver, &link).await;
    let again = identify(&driver, &back).await;
    assert_eq!(again.key, identity.key);

    // The failure rule: while the node was gone the monitor waited, starting at a quarter
    // second, doubling, never longer than the cap.
    let delays = pause.0.lock().unwrap().clone();
    assert!(!delays.is_empty(), "the monitor waited between tries");
    assert_eq!(delays[0], Duration::from_millis(250));
    assert!(delays.iter().all(|delay| *delay <= Duration::from_secs(5)));
}

#[tokio::test]
async fn a_node_with_lemnos_a_plain_node_and_a_faulted_lemnos_are_three_kinds_of_identity() {
    // With Lemnos: the board's own key, the Lemnos contracts it offers.
    let board_dir = scratch("kind-board");
    let board = start(&board_dir, &running_board(), None);
    let pause = std::sync::Arc::new(Delays::default());
    let driver = driver_for(board.network().port(), &pause);
    let link = usb_link();
    let board_id = identify(&driver, &offered(&driver, &link).await).await;
    assert_eq!(board_id.key, DeviceKey::from_static("test-family", "0001"));
    assert!(driver.capabilities(&board_id).grants(CapabilityKind::Slots));

    // Lemnos stopped on that same node: its facts are still published, so it is faulted and
    // restarting, not plain. Its key is still the board's.
    board.stop_lemnos();
    let faulted = identify(&driver, &offered(&driver, &link).await).await;
    assert!(
        matches!(&faulted.lemnos, LemnosState::NotAnswering { reason } if reason.starts_with("-4")),
        "{faulted:?}"
    );
    assert_eq!(faulted.key, DeviceKey::from_static("test-family", "0001"));
    assert!(faulted.key.is_board(), "a board key, not a plain one");

    // Lemnos answers again: back to answering.
    board.resume_lemnos();
    let back = identify(&driver, &offered(&driver, &link).await).await;
    assert_eq!(back.lemnos, LemnosState::Answering);

    // A plain Orion node: no hardware facts, no Lemnos. It is not refused; it is keyed by its
    // fingerprint and grants Orion's own services only.
    let plain_dir = scratch("kind-plain");
    let plain = start(&plain_dir, &Kind::Plain, None);
    let plain_driver = driver_for(plain.network().port(), &pause);
    let plain_id = identify(&plain_driver, &offered(&plain_driver, &link).await).await;
    assert!(
        matches!(plain_id.key, DeviceKey::Node { .. }),
        "keyed by the node's fingerprint"
    );
    assert_eq!(plain_id.lemnos, LemnosState::Absent);
    let plain_grants = plain_driver.capabilities(&plain_id);
    for kind in [
        CapabilityKind::Files,
        CapabilityKind::Logs,
        CapabilityKind::Actions,
        CapabilityKind::Shell,
    ] {
        assert!(plain_grants.grants(kind), "{kind:?} for a plain node");
    }
    assert!(
        !plain_grants.grants(CapabilityKind::Slots),
        "no slots without Lemnos"
    );
    assert!(!plain_grants.grants(CapabilityKind::Update));
}

#[tokio::test]
async fn a_bare_tcp_listener_is_never_offered_as_a_node() {
    // Something accepts TCP and says nothing Orion-like: a bare connect is not detection.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    let pause = std::sync::Arc::new(Delays::default());
    let driver = driver_for(port, &pause);
    let link = usb_link();
    // The first scan starts the monitor. Then wait for it to try and back off twice: a bare
    // connect must still offer nothing, whenever the scan is asked.
    assert!(driver.discover(&link).await.unwrap().is_empty());
    paused_beyond(&pause, 1).await;
    assert!(driver.discover(&link).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_node_that_does_not_publish_a_family_still_identifies_by_fingerprint() {
    let dir = scratch("anonymous");
    let board = start(
        &dir,
        &Kind::WithLemnos(Hello {
            family: String::new(),
            ..Hello::default()
        }),
        None,
    );
    let pause = std::sync::Arc::new(Delays::default());
    let driver = driver_for(board.network().port(), &pause);
    let identity = identify(&driver, &offered(&driver, &usb_link()).await).await;
    assert!(matches!(identity.key, DeviceKey::Node { .. }));
}

#[tokio::test]
async fn the_driver_serves_every_family_and_only_its_own_link_kinds() {
    let pause = std::sync::Arc::new(Delays::default());
    let driver = BoardDriver::with_pause(Vec::new(), pause);
    assert_eq!(driver.serves(), Serves::Any);
    let serial = Link {
        id: LinkId::new(LINK),
        kind: LinkKind::UsbSerial,
        label: "a serial port".into(),
    };
    assert!(driver.discover(&serial).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_turned_off_node_is_not_offered_until_turned_on() {
    let dir = scratch("off");
    let board = start(&dir, &running_board(), None);
    let port = board.network().port();
    let address = format!("orion+tcp://127.0.0.1:{port}");
    let pause = std::sync::Arc::new(Delays::default());
    let driver = driver_for(port, &pause);
    let link = usb_link();
    offered(&driver, &link).await;
    driver.turn_off(&address);
    assert!(driver.discover(&link).await.unwrap().is_empty());
    driver.turn_on(&address);
    offered(&driver, &link).await;
}
