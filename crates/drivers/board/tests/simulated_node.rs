//! The board driver against Orion's own comm core (`orion-stubs`) on a real socket and a real
//! network port. The node's hello facts are published the way a board's own program does it:
//! as topics `hello.<fact>`, from a client on the board's socket.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use atlas_board::{BoardDriver, Pause, Seed};
use atlas_driver::{Driver, DriverError};
use atlas_types::{
    Candidate, CapabilityKind, DeviceKey, DeviceState, Identity, Link, LinkId, LinkKind,
};
use orion::topics::{FieldId, Publisher, SampleTime, TopicDecl};
use orion::{Address, Orion, fields};
use orion_stubs::{Board, Security};

const LINK: &str = "usbnet:test0";
const WAIT: Duration = Duration::from_secs(10);

/// Records each delay the monitor asks for. It does not sleep for the delay, only briefly,
/// so the test runs at once; the delays themselves are what the failure rule is checked on.
#[derive(Debug, Default)]
struct Delays(Mutex<Vec<Duration>>);

impl Pause for Delays {
    fn pause(&self, delay: Duration, _stop: &AtomicBool) {
        self.0.lock().unwrap().push(delay);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A board in `dir`, with its hello facts published. Field order is the drop order: the
/// publishers and the client go before the comm core stops.
struct Host {
    _publishers: Vec<Publisher>,
    _client: Orion,
    board: Board,
}

fn host(dir: &Path, port: Option<u16>, facts: &[(&str, &str)]) -> Host {
    let board = Board::start_in(dir, Security::Off, port);
    let client = Orion::connect(&Address::Local(board.socket().to_path_buf()), "facts")
        .expect("the board's client");
    let mut publishers = Vec::new();
    for (name, value) in facts {
        let topic = format!("hello.{name}");
        let mut publisher = client
            .declare(&TopicDecl {
                name: &topic,
                fields: &fields![value: Text],
                history: 1,
                expiry_ms: 0,
                max_payload: 256,
            })
            .expect("a fact topic");
        publisher
            .publish(FieldId(0), SampleTime::from_nanos(1), value.as_bytes())
            .expect("the fact");
        publishers.push(publisher);
    }
    Host {
        _publishers: publishers,
        _client: client,
        board,
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-board-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn usb_link() -> Link {
    Link {
        id: LinkId::new(LINK),
        kind: LinkKind::UsbNetwork,
        label: "test USB network".into(),
    }
}

fn driver_for(port: u16, pause: Arc<dyn Pause>) -> BoardDriver {
    let seed = Seed::new(LinkId::new(LINK), &format!("orion+tcp://127.0.0.1:{port}")).unwrap();
    BoardDriver::with_pause(vec![seed], pause)
}

const RUNNING: &[(&str, &str)] = &[
    ("family", "test-family"),
    ("serial", "0001"),
    ("state", "running"),
    ("model", "test-model"),
    ("version.os", "1.0.0"),
    ("offers", "logs,update"),
];

/// Polls `discover` until the node is offered, or the wait runs out.
async fn offered(driver: &BoardDriver, link: &Link) -> Candidate {
    tokio::time::timeout(WAIT, async {
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
    tokio::time::timeout(WAIT, async {
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

#[tokio::test]
async fn a_board_is_found_identified_lost_and_found_again() {
    let dir = scratch("cycle");
    let first = host(&dir, None, RUNNING);
    let port = first.board.network().port();
    let pause = Arc::new(Delays::default());
    let driver = driver_for(port, pause.clone());
    let link = usb_link();

    // Found, and identified from the hello document.
    let candidate = offered(&driver, &link).await;
    assert_eq!(candidate.link, LinkId::new(LINK));
    let identity: Identity = driver.identify(&candidate).await.expect("identified");
    assert_eq!(identity.key, DeviceKey::new("test-family", "0001"));
    assert_eq!(identity.state, DeviceState::Running);
    assert_eq!(identity.model, "test-model");
    assert_eq!(
        identity.versions.get("os").map(String::as_str),
        Some("1.0.0")
    );
    assert_eq!(identity.address, candidate.address);
    let grants = driver.capabilities(&identity);
    assert!(grants.grants(CapabilityKind::Logs));
    assert!(grants.grants(CapabilityKind::Update));
    assert!(
        !grants.grants(CapabilityKind::Slots),
        "a kind the node does not offer is not granted"
    );

    // Lost: the comm core goes away, and the scan stops offering the node.
    drop(first);
    gone(&driver, &link).await;
    let lost = driver.identify(&candidate).await;
    assert!(
        matches!(lost, Err(DriverError::Unreachable(_))),
        "a lost node is unreachable, not identified: {lost:?}"
    );

    // Found again on the same port, with the same identity.
    let _second = host(&dir, Some(port), RUNNING);
    let back = offered(&driver, &link).await;
    let again = driver.identify(&back).await.expect("identified again");
    assert_eq!(again.key, identity.key);

    // The failure rule: while the node was gone the monitor waited, starting at a quarter
    // second, doubling, and never longer than the cap.
    let delays = pause.0.lock().unwrap().clone();
    assert!(!delays.is_empty(), "the monitor waited between tries");
    assert_eq!(delays[0], Duration::from_millis(250));
    assert!(delays.iter().all(|delay| *delay <= Duration::from_secs(5)));
    for pair in delays.windows(2) {
        assert!(
            pair[1] >= pair[0].min(Duration::from_secs(5)),
            "delays never shrink: {delays:?}"
        );
    }
}

#[tokio::test]
async fn one_path_identifies_running_recovery_and_freshly_loaded_nodes() {
    let cases = [
        ("running", DeviceState::Running),
        ("recovery", DeviceState::Recovery),
        ("flasher", DeviceState::Recovery),
    ];
    for (index, (state, expected)) in cases.into_iter().enumerate() {
        let dir = scratch(&format!("state{index}"));
        let host = host(
            &dir,
            None,
            &[
                ("family", "test-family"),
                ("serial", &format!("100{index}")),
                ("state", state),
                ("offers", "logs,recover"),
            ],
        );
        let driver = driver_for(host.board.network().port(), Arc::new(Delays::default()));
        let candidate = offered(&driver, &usb_link()).await;
        let identity = driver.identify(&candidate).await.expect("identified");
        assert_eq!(identity.state, expected, "state {state}");
        assert_eq!(
            identity.key,
            DeviceKey::new("test-family", format!("100{index}"))
        );
        let shown = driver.capabilities(&identity).shown(identity.state);
        assert!(shown.contains(&CapabilityKind::Info));
        assert_eq!(
            shown.contains(&CapabilityKind::Recover),
            expected == DeviceState::Recovery,
            "state {state} shows Recover"
        );
    }
}

#[tokio::test]
async fn a_node_that_does_not_say_who_it_is_is_refused_by_name() {
    let dir = scratch("anonymous");
    let host = host(&dir, None, &[("state", "running")]);
    let driver = driver_for(host.board.network().port(), Arc::new(Delays::default()));
    let candidate = offered(&driver, &usb_link()).await;
    match driver.identify(&candidate).await {
        Err(DriverError::Incompatible(message)) => {
            assert!(message.contains("hello.family"), "{message}");
        }
        other => panic!("expected a refusal naming the fact, got {other:?}"),
    }
}

#[tokio::test]
async fn a_turned_off_node_is_not_offered_until_turned_on() {
    let dir = scratch("off");
    let host = host(&dir, None, RUNNING);
    let port = host.board.network().port();
    let address = format!("orion+tcp://127.0.0.1:{port}");
    let driver = driver_for(port, Arc::new(Delays::default()));
    let link = usb_link();
    offered(&driver, &link).await;
    driver.turn_off(&address);
    assert!(driver.discover(&link).await.unwrap().is_empty());
    driver.turn_on(&address);
    offered(&driver, &link).await;
}

#[tokio::test]
async fn a_node_on_another_kind_of_link_is_not_offered() {
    let dir = scratch("kind");
    let host = host(&dir, None, RUNNING);
    let driver = driver_for(host.board.network().port(), Arc::new(Delays::default()));
    let serial = Link {
        id: LinkId::new(LINK),
        kind: LinkKind::UsbSerial,
        label: "a serial port on the same name".into(),
    };
    assert!(driver.discover(&serial).await.unwrap().is_empty());
}
