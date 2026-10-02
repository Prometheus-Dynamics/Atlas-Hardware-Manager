use std::collections::BTreeMap;
use std::sync::Arc;

use atlas_core::{
    ActivityKind, Atlas, CoreError, JsonFileStore, MemoryStore, ReleaseTarget, StagedRollout,
    UpdateRequest,
};
use atlas_driver::{CapabilityKind, DeviceKey, Family};
use atlas_driver_mock::{MockFleet, SIM_HELIOS, SIM_MCU};

fn atlas_for(fleet: &MockFleet, store: Arc<dyn atlas_core::InventoryStore>) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(store);
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().expect("atlas builds")
}

fn kinds(atlas: &Atlas) -> Vec<ActivityKind> {
    atlas
        .activity(100)
        .into_iter()
        .rev()
        .map(|entry| entry.kind)
        .collect()
}

#[tokio::test]
async fn the_history_follows_devices_and_updates() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;
    assert_eq!(kinds(&atlas), vec![ActivityKind::DeviceFound; 6]);

    // A quiet rescan adds nothing.
    atlas.scan().await;
    assert_eq!(atlas.activity(100).len(), 6);

    let rear = DeviceKey::new(SIM_HELIOS, "H-1003");
    fleet.set_online(&rear, false);
    atlas.scan().await;
    fleet.set_online(&rear, true);
    atlas.scan().await;
    let job = atlas
        .start_update(UpdateRequest {
            devices: vec![rear.clone()],
            releases: BTreeMap::from([(
                Family::new(SIM_HELIOS),
                ReleaseTarget::version("2026.3.1"),
            )]),
            staged: StagedRollout::Off,
        })
        .unwrap();
    atlas.wait_job(job).await.unwrap();
    // The refreshed version is not reported twice by the next scan.
    atlas.scan().await;

    let latest = atlas.activity(3);
    assert_eq!(latest[0].kind, ActivityKind::UpdateResult);
    assert!(latest[0].message.contains("2026.3.1"));
    assert_eq!(latest[1].kind, ActivityKind::DeviceOnline);
    assert_eq!(latest[2].kind, ActivityKind::DeviceOffline);
    assert_eq!(atlas.activity(100).len(), 9);
}

#[tokio::test]
async fn the_history_is_remembered() {
    let path = std::env::temp_dir().join(format!("atlas-activity-{}.json", std::process::id()));
    let fleet = MockFleet::demo();
    {
        let atlas = atlas_for(&fleet, Arc::new(JsonFileStore::new(&path)));
        atlas.scan().await;
    }
    let atlas = atlas_for(&fleet, Arc::new(JsonFileStore::new(&path)));
    assert_eq!(atlas.activity(100).len(), 6);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn running_devices_report_metrics_and_logs() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;

    let camera = DeviceKey::new(SIM_HELIOS, "H-1001");
    let record = atlas.device(&camera).unwrap();
    assert!(record.capabilities.contains(&CapabilityKind::Telemetry));
    assert!(record.capabilities.contains(&CapabilityKind::Logs));
    assert!(record.identity.attributes.contains_key("manage_url"));

    let metrics = atlas.telemetry(&camera).await.unwrap();
    assert!(metrics.iter().any(|metric| metric.id == "temp"));
    let logs = atlas.logs(&camera, 20).await.unwrap();
    assert_eq!(logs.len(), 20);

    atlas.run_action(&camera, "locate").await.unwrap();
    assert_eq!(atlas.activity(1)[0].kind, ActivityKind::ActionRun);

    // A board in recovery offers neither.
    let recovering = DeviceKey::new(SIM_MCU, "M-2003");
    assert!(matches!(
        atlas.telemetry(&recovering).await,
        Err(CoreError::Unsupported { .. })
    ));
}
