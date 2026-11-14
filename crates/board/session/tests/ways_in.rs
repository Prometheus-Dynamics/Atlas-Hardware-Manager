//! The network way in of a node is bound to one address: a session reaches it there, and not at
//! another local address. And Atlas's parse of `orion+tcp://` takes an IPv6 literal, with or
//! without a scope.

#![cfg(target_os = "linux")]

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use atlas_session::{Pause, Presence, Seed, Sessions};
use atlas_sim_node::{DEADLINE, Hello, Kind, Mode, SimNode};
use atlas_types::LinkId;
use orion::Address;

/// A pause that does not wait: the monitor only yields while the node is not there.
#[derive(Debug)]
struct NoPause;

impl Pause for NoPause {
    fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
        std::thread::yield_now();
    }
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ways-in-{name}"))
}

fn kind() -> Kind {
    Kind::WithLemnos(Hello {
        family: "test-family".into(),
        serial: "0001".into(),
        mode: Mode::Running,
        model: "test-model".into(),
        ..Hello::default()
    })
}

#[tokio::test]
async fn the_way_in_answers_on_its_bound_address_and_not_on_another_local_one() {
    let dir = scratch("bound");
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start_with(&dir, &kind(), None).expect("the simulated node starts");
    let bound = node.network();
    assert!(
        bound.ip().is_loopback() && !bound.ip().is_unspecified(),
        "the way in is bound to one address, not a wildcard: {bound}"
    );
    let port = bound.port();

    // A session at the bound address is answered.
    let sessions = Sessions::new(Arc::new(NoPause));
    let there = sessions
        .handle(&Address::from_str(&format!("orion+tcp://127.0.0.1:{port}")).unwrap())
        .unwrap();
    let mut presence = there.watch();
    tokio::time::timeout(
        DEADLINE,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected at its bound address")
    .expect("the monitor is running");

    // The same port at another local address is refused at once: nothing listens there.
    let other = SocketAddr::from(([127, 0, 0, 2], port));
    let refused = TcpStream::connect_timeout(&other, Duration::from_secs(2))
        .expect_err("a connection at another local address must not be answered");
    assert_eq!(refused.kind(), ErrorKind::ConnectionRefused, "{refused}");

    // And a session pointed there never finds the node.
    let elsewhere = sessions
        .handle(&Address::from_str(&format!("orion+tcp://127.0.0.2:{port}")).unwrap())
        .unwrap();
    let mut presence = elsewhere.watch();
    let found = tokio::time::timeout(
        Duration::from_millis(500),
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await;
    assert!(
        found.is_err(),
        "a session at another local address reached the node"
    );
}

#[test]
fn atlas_takes_the_ipv6_ways_in_as_written() {
    let key = "ab".repeat(32);
    let cases = [
        (format!("orion+tcp://[::1]:9200#{key}"), "[::1]:9200"),
        (format!("orion+tcp://[::1]#{key}"), "[::1]:9200"),
        (
            format!("orion+tcp://[fe80::1%25lo]:9200#{key}"),
            "[fe80::1%25lo]:9200",
        ),
        (
            format!("orion+tcp://[fe80::1%lo]:9200#{key}"),
            "[fe80::1%lo]:9200",
        ),
    ];
    for (text, host) in cases {
        let seed = Seed::new(LinkId::new("usbnet:a"), &text)
            .unwrap_or_else(|error| panic!("{text} is refused: {error}"));
        match seed.address {
            Address::Network {
                host: got,
                key: Some(_),
            } => assert_eq!(got, host, "{text}"),
            other => panic!("{text} parsed as {other:?}"),
        }
    }
}
