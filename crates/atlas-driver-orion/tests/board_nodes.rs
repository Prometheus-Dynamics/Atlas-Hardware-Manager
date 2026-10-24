//! `BoardNodes` against a real orion-node on `orion+tcp`: a board whose
//! node enrolls operators on their hello from this network (the Raze's
//! `orion-env` with its security switch off) gets Orion's capabilities with
//! no Orion set up in Atlas and no key; one that doesn't (switch on) gets
//! none, and Atlas uses the board's own ways.

#![cfg(unix)]

use std::collections::BTreeMap;

use atlas_driver::{CapabilitySource, DeviceKey, DeviceMode, Identity, LinkId, LinkSource};
use atlas_driver_orion::{BoardNodes, OperatorIdentity};
use orion_node::{
    LinuxHostFactsSource, NodeApp, NodeConfig, NodeId, OperatorAutoEnroll, OperatorOpenRead,
};

const SERIAL: &str = "6acf6699775f28dd";

/// A node for a board with `SERIAL`, enrolling 127.0.0.0/8 when `open`.
async fn node(label: &str, open: bool) -> (std::net::SocketAddr, impl Drop) {
    let dir =
        std::env::temp_dir().join(format!("atlas-board-nodes-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proc/device-tree")).unwrap();
    std::fs::write(
        dir.join("proc/device-tree/serial-number"),
        format!("{SERIAL}\0"),
    )
    .unwrap();
    let rules = if open {
        OperatorAutoEnroll::parse_all("127.0.0.0/8=update*,slot.*,reboot,locate").unwrap()
    } else {
        Vec::new()
    };
    let config = NodeConfig::for_local_node(NodeId::new(format!("raze-{label}")))
        .with_ipc_socket_path(dir.join("control.sock"))
        .with_runtime_tuning_mut(|mut tuning| {
            tuning.actions = tuning
                .actions
                .with_operator_open_read(OperatorOpenRead {
                    status: false,
                    streams: false,
                    state: false,
                })
                .with_operator_auto_enroll(rules);
            tuning
        });
    let app = NodeApp::try_new(config).unwrap();
    // The node's own record (board serial and all) comes from its host
    // facts, as orion-node's loop samples them.
    app.refresh_host_facts_from(&LinuxHostFactsSource::with_root(&dir));
    let (addr, server) = app
        .start_peer_tcp_server("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    struct Running<S>(
        #[allow(dead_code)] S,
        #[allow(dead_code)] NodeApp,
        std::path::PathBuf,
    );
    impl<S> Drop for Running<S> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.2);
        }
    }
    (addr, Running(server, app, dir))
}

fn board() -> Identity {
    Identity {
        key: DeviceKey::new("raze", SERIAL),
        model: "Raze".into(),
        mode: DeviceMode::Normal,
        versions: BTreeMap::new(),
        name: None,
        link: LinkId("usb-gadget".into()),
        address: "http://127.0.0.1:5899/.well-known/pd-device".into(),
        // As the board driver sets it: the serial's last 8 hex digits.
        attributes: BTreeMap::from([("board_serial".to_string(), SERIAL[8..].to_string())]),
    }
}

async fn capabilities_after_a_scan(port: u16) -> atlas_driver::Capabilities {
    let nodes = BoardNodes::on_port(
        OperatorIdentity::generate("atlas-test").unwrap(),
        None,
        None,
        None,
        port,
    );
    // The first identify finds the node; a scan after it has it (once the
    // node has sampled its host facts).
    let _ = nodes.capabilities_for(&board());
    for _ in 0..30 {
        nodes.links().await;
        let capabilities = nodes.capabilities_for(&board());
        if capabilities.telemetry.is_some() {
            return capabilities;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    nodes.capabilities_for(&board())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_that_enrolls_this_network_is_reached_with_no_key() {
    let (addr, _node) = node("open", true).await;
    let capabilities = capabilities_after_a_scan(addr.port()).await;
    assert!(
        capabilities.telemetry.is_some() && capabilities.actions.is_some(),
        "the board's own node, enrolled on the hello"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_that_enrolls_nobody_adds_nothing() {
    let (addr, _node) = node("closed", false).await;
    let capabilities = capabilities_after_a_scan(addr.port()).await;
    assert!(
        capabilities.telemetry.is_none() && capabilities.slots.is_none(),
        "not enrolled: the board's own ways only"
    );
}
