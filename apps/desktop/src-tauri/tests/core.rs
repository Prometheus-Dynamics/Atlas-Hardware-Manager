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
        dirs: Some(Dirs::under(root)),
        mode: Mode::Sim,
        scan: quick(),
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

#[tokio::test(flavor = "multi_thread")]
async fn the_scan_fills_the_inventory_and_a_device_that_leaves_goes_offline_and_returns() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(options(dir.path())).await;
    until(&core, |core| devices(core).len() == 5).await;
    until(&core, |core| {
        core.topics()["scan"]["first_round_done"] == true
    })
    .await;

    let id = "simulated:SIM-RUN-1";
    atlas_app::call::call(
        &core,
        "sim.set_online",
        json!({ "id": id, "online": false }),
    )
    .await
    .unwrap();
    until(&core, |core| {
        presence(core, id).as_deref() == Some("offline")
    })
    .await;
    // The others are untouched: one device leaving never takes its neighbours with it.
    assert_eq!(
        devices(&core)
            .iter()
            .filter(|d| d["presence"] == "online")
            .count(),
        4
    );
    let activity = core.topics()["inventory"]["activity"].clone();
    assert_eq!(activity[0]["level"], "warn", "{activity}");
    assert_eq!(activity[0]["device"], id);

    atlas_app::call::call(&core, "sim.set_online", json!({ "id": id, "online": true }))
        .await
        .unwrap();
    until(&core, |core| {
        presence(core, id).as_deref() == Some("online")
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn it_opens_with_the_last_known_inventory_before_the_scan_has_answered() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = Core::open(options(dir.path())).await;
        until(&core, |core| devices(core).len() == 5).await;
        core.save_now().await;
    }
    let core = Core::open(options(dir.path())).await;
    // No await between open and this read: every record it knew is there before any driver has
    // answered. The inventory's own rule is that nothing is online until a scan sees it again.
    assert_eq!(devices(&core).len(), 5);
    until(&core, |core| {
        devices(core)
            .iter()
            .all(|device| device["presence"] == "online")
    })
    .await;
    // A record is never called gone because the scan listed its address before asking who is
    // there: no "not seen" line was written on the way.
    until(&core, |core| {
        core.topics()["scan"]["first_round_done"] == true
    })
    .await;
    let activity = core.topics()["inventory"]["activity"].to_string();
    assert!(!activity.contains("not seen"), "{activity}");
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
