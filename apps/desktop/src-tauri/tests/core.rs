//! The core against the simulated devices: what the window is given, with no window.
#![cfg(feature = "sim")]

use std::time::Duration;

use atlas_app::paths::Dirs;
use atlas_app::{Core, Mode, Options};
use atlas_jobs::ScanConfig;
use serde_json::{Value, json};

fn quick() -> ScanConfig {
    ScanConfig {
        tick: Duration::from_millis(10),
        interval: Duration::from_millis(40),
        ..ScanConfig::default()
    }
}

fn options(root: &std::path::Path) -> Options {
    Options {
        scan: quick(),
        ..Options::new(Some(Dirs::under(root)), Mode::Sim)
    }
}

fn devices(core: &Core) -> Vec<Value> {
    core.topics()["inventory"]["devices"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Waits for the core to say what `ready` asks for. The wait ends on the update that makes it
/// true, never on a fixed delay; the limit only fails a test that would otherwise hang.
async fn until(core: &Core, ready: impl Fn(&Core) -> bool) {
    let mut updates = core.subscribe();
    let wait = async {
        while !ready(core) {
            let _ = updates.recv().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the core never reached the state the test waits for");
}

fn presence(core: &Core, id: &str) -> Option<String> {
    devices(core)
        .iter()
        .find(|device| device["id"] == id)
        .map(|device| device["presence"].as_str().unwrap_or_default().to_owned())
}

const NODE: &str = "simulated:0123456789abcdef";
const OLD: &str = "simulated:SIM-OLD-1";
const RECOVERY: &str = "simulated:SIM-REC-1";

fn online(core: &Core) -> usize {
    devices(core)
        .iter()
        .filter(|device| device["presence"] == "online")
        .count()
}

/// The demo world as it settles: six records, the old board unplugged once it was seen.
async fn settled(core: &Core) {
    until(core, |core| {
        devices(core).len() == 6
            && online(core) == 5
            && presence(core, OLD).as_deref() == Some("offline")
    })
    .await;
}

fn device(core: &Core, id: &str) -> Value {
    devices(core)
        .into_iter()
        .find(|device| device["id"] == id)
        .unwrap_or_else(|| panic!("no record {id}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scan_fills_the_inventory_and_a_device_that_leaves_goes_offline_and_returns() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    settled(&core).await;
    until(&core, |core| {
        core.topics()["scan"]["first_round_done"] == true
    })
    .await;

    atlas_app::call::call(
        &core,
        "sim.set_online",
        json!({ "id": RECOVERY, "online": false }),
    )
    .await
    .unwrap();
    until(&core, |core| {
        presence(core, RECOVERY).as_deref() == Some("offline")
    })
    .await;
    // The others are untouched: one device leaving never takes its neighbours with it.
    assert_eq!(online(&core), 4);
    let activity = core.topics()["inventory"]["activity"].clone();
    assert_eq!(activity[0]["level"], "warn", "{activity}");
    assert_eq!(activity[0]["device"], RECOVERY);

    atlas_app::call::call(
        &core,
        "sim.set_online",
        json!({ "id": RECOVERY, "online": true }),
    )
    .await
    .unwrap();
    until(&core, |core| {
        presence(core, RECOVERY).as_deref() == Some("online")
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_driver_reports_its_simulated_device_and_a_board_on_two_links_is_one_record() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    settled(&core).await;

    // The simulated Orion node, through the real board driver: one record, both links, the
    // name a person gave it, and what its hello says it runs.
    until(&core, |core| {
        device(core, NODE)["links"]
            .as_array()
            .is_some_and(|l| l.len() == 2)
    })
    .await;
    until(&core, |core| device(core, NODE)["name"] == "bench-a").await;
    let board = device(&core, NODE);
    let kinds: Vec<&str> = board["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| link["kind"]["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"usb-network") && kinds.contains(&"ethernet"),
        "{kinds:?}"
    );
    assert_eq!(board["identity"]["state"], "running");
    assert_eq!(board["identity"]["lemnos"]["state"], "answering");
    assert_eq!(board["identity"]["versions"]["os"], "2026.11.2");

    // The boot ROM, the disk and the controller, each through its own real driver.
    let rom = device(&core, "rockchip-maskrom:usb-1-3");
    assert_eq!(rom["identity"]["state"], "boot-rom");
    assert_eq!(rom["shown"], json!(["info", "flash"]));
    assert_eq!(rom["links"][0]["kind"]["kind"], "usb-boot");
    let card = device(&core, "removable-disk:SIM-CARD-1");
    assert_eq!(card["identity"]["state"], "removable-disk");
    assert_eq!(card["links"][0]["kind"]["kind"], "local-disk");
    until(&core, |core| {
        device(core, "systemcore:9999")["robot"] == "Practice field"
    })
    .await;
    let controller = device(&core, "systemcore:9999");
    assert_eq!(controller["identity"]["state"], "robot-device");
    assert_eq!(controller["role"], "controller");
    assert_eq!(device(&core, RECOVERY)["identity"]["state"], "recovery");
    assert_eq!(
        core.topics()["inventory"]["robots"][0]["name"],
        "Practice field"
    );
    // The scan says which links it looks at, so a link sighting can be named.
    let links = core.topics()["scan"]["links"].clone();
    assert!(
        links
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["id"] == "usb-1-2" && link["label"] == "USB 1-2"),
        "{links}"
    );

    // The node stops (the board is unplugged) and comes back as the same record.
    atlas_app::call::call(
        &core,
        "sim.set_online",
        json!({ "id": NODE, "online": false }),
    )
    .await
    .unwrap();
    until(&core, |core| {
        presence(core, NODE).as_deref() == Some("offline")
    })
    .await;
    atlas_app::call::call(
        &core,
        "sim.set_online",
        json!({ "id": NODE, "online": true }),
    )
    .await
    .unwrap();
    until(&core, |core| {
        presence(core, NODE).as_deref() == Some("online")
    })
    .await;
    assert_eq!(devices(&core).len(), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_empty_world_has_every_driver_and_no_device() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(Options {
        scene: "empty".into(),
        ..options(dir.path())
    })
    .await;
    until(&core, |core| {
        core.topics()["scan"]["first_round_done"] == true
    })
    .await;
    assert!(devices(&core).is_empty());
    assert_eq!(core.topics()["scan"]["problems"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn it_opens_with_the_last_known_inventory_before_the_scan_has_answered() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = Core::open(options(dir.path())).await;
        settled(&core).await;
        core.save_now().await;
    }
    let core = Core::open(options(dir.path())).await;
    // No await between open and this read: every record it knew is there before any driver has
    // answered. The inventory's own rule is that nothing is online until a scan sees it again.
    assert_eq!(devices(&core).len(), 6);
    until(&core, |core| online(core) == 5).await;
    // A record is never called gone because the scan listed its address before asking who is
    // there: no "not seen" line was written in this run for a board that is plugged in.
    until(&core, |core| {
        core.topics()["scan"]["first_round_done"] == true
    })
    .await;
    assert_eq!(presence(&core, RECOVERY).as_deref(), Some("online"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_is_logged_with_what_was_asked_and_settings_are_saved() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    let error = atlas_app::call::call(&core, "no.such.call", json!({ "x": 1, "password": "p" }))
        .await
        .unwrap_err();
    assert!(error.0.contains("no.such.call"), "{error}");
    core.log().flush();
    let log = core.log().read_all().unwrap();
    assert!(
        log.contains("no.such.call") && log.contains("\"x\":1"),
        "{log}"
    );
    assert!(!log.contains("\"p\""), "{log}");

    let saved = atlas_app::call::call(
        &core,
        "settings.set",
        json!({ "keep_last": 5, "security": true, "channel": "dev", "theme": "dark" }),
    )
    .await
    .unwrap();
    assert_eq!(saved["keep_last"], 5);
    assert_eq!(core.topics()["settings"]["security"], true);
    let reopened = Core::open(options(dir.path())).await;
    assert_eq!(reopened.settings().keep_last, 5);
    assert!(reopened.settings().security);
}

fn board(core: &Core, id: &str) -> Value {
    core.topics()
        .get(&atlas_app::board::topic(id))
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_open_board_is_read_through_its_session_and_a_closed_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    let id = core.sim().unwrap().node_id().to_owned();
    until(&core, |core| {
        presence(core, &id).as_deref() == Some("online")
    })
    .await;
    // Nothing is read for a board nobody is looking at.
    assert_eq!(board(&core, &id), Value::Null);

    atlas_app::call::call(&core, "board.open", json!({ "id": id }))
        .await
        .unwrap();
    until(&core, |core| board(core, &id)["slots"]["state"] == "ok").await;
    let view = board(&core, &id);
    assert_eq!(view["reach"], "orion");
    assert_eq!(view["connection"]["live"], true);
    assert_eq!(view["slots"]["list"]["default"], "slot-4");
    assert!(
        view["actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| action["name"] == "locate")),
        "{view}"
    );
    assert!(view["hardware"].as_array().is_some_and(|d| !d.is_empty()));
    // The device list's Default column is filled from the slots that were read.
    until(&core, |core| {
        devices(core)
            .iter()
            .any(|device| device["id"] == id.as_str() && device["default_version"] == "2026.11.2")
    })
    .await;

    // The board's own refusal comes back in its words, and is on the page's topic.
    let refused = atlas_app::call::call(
        &core,
        "board.slots.remove",
        json!({ "id": id, "slot": "slot-4" }),
    )
    .await
    .unwrap_err();
    assert!(refused.0.contains("running slot"), "{refused}");
    until(&core, |core| board(core, &id)["said"]["ok"] == false).await;
    let said = atlas_app::call::call(
        &core,
        "board.action",
        json!({ "id": id, "action": "locate" }),
    )
    .await
    .unwrap();
    assert_eq!(said, "blinking");

    atlas_app::call::call(&core, "board.close", json!({ "id": id }))
        .await
        .unwrap();
    until(&core, |core| board(core, &id) == Value::Null).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_is_not_an_orion_node_has_nothing_to_read_and_refuses_board_calls() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    let id = "simulated:SIM-REC-1";
    until(&core, |core| presence(core, id).is_some()).await;
    atlas_app::call::call(&core, "board.open", json!({ "id": id }))
        .await
        .unwrap();
    until(&core, |core| board(core, id)["reach"] == "none").await;
    let error = atlas_app::call::call(
        &core,
        "board.action",
        json!({ "id": id, "action": "locate" }),
    )
    .await
    .unwrap_err();
    assert!(error.0.contains("not reached through Orion"), "{error}");
    let unknown = atlas_app::call::call(&core, "board.open", json!({ "id": "simulated:NOPE" }))
        .await
        .unwrap_err();
    assert!(unknown.0.contains("no device"), "{unknown}");
}
