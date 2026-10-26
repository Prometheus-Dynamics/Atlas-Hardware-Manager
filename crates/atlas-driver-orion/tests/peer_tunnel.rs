//! A board behind a real robot controller, through its HeliOS's Orion
//! tunnel (ignored: needs one). Each run enrolls a throwaway operator on the
//! board while its security is off. Run with
//! `ATLAS_PEER=http://172.27.0.1:5880/v1/peers/raze-37bc/board/identity
//! ATLAS_PEER_SERIAL=0708c973b00a37bc cargo test -p atlas-driver-orion
//! --test peer_tunnel -- --ignored`.

use atlas_driver::{
    CapabilitySource, DeviceKey, DeviceMode, Identity, LinkId, LinkSource, attributes,
};
use atlas_driver_orion::{BoardNodes, LiveStreams, OperatorIdentity};

fn operator(what: &str) -> OperatorIdentity {
    OperatorIdentity::generate(&format!("atlas-peer-test-{what}-{}", std::process::id())).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a robot controller with a board behind it (ATLAS_PEER, ATLAS_PEER_SERIAL)"]
async fn the_board_gets_orions_slots_through_the_controller() {
    let peer = std::env::var("ATLAS_PEER").expect("ATLAS_PEER");
    let serial = std::env::var("ATLAS_PEER_SERIAL").expect("ATLAS_PEER_SERIAL");
    let url = LiveStreams::url_for(&peer);
    assert!(url.starts_with("orion+tcp://127.0.0.1:"), "{url}");

    let nodes = BoardNodes::new(operator("slots"), None, None, None);
    let mut device = Identity {
        key: DeviceKey::new("raze", &serial),
        model: "Raze".into(),
        mode: DeviceMode::Normal,
        versions: Default::default(),
        name: None,
        link: LinkId("controller".into()),
        address: peer,
        attributes: Default::default(),
    };
    device.attributes.insert(
        attributes::BOARD_SERIAL.into(),
        attributes::normalize_board_serial(&serial).unwrap(),
    );
    // The first look starts the board's directory; the next scan finds it.
    let _ = nodes.capabilities_for(&device);
    nodes.links().await;
    let caps = nodes.capabilities_for(&device);
    let slots = caps.slots.expect("Orion's slots for the board");
    let set = slots.slots(&device).await.unwrap().expect("a slot set");
    assert!(
        set.slots
            .iter()
            .any(|slot| slot.flags.iter().any(|f| f == "running")),
        "{set:?}"
    );
}
