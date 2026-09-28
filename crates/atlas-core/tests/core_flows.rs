use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use atlas_core::{
    Atlas, CoreError, DeviceJobStatus, Event, InventoryStore, JobState, JsonFileStore, MemoryStore,
    Presence, ReleaseTarget, StagedRollout, UpdateRequest,
};
use atlas_driver::{DeviceKey, DeviceMode, Family, LinkKind};
use atlas_driver_mock::{MockDevice, MockFleet, SIM_HELIOS, SIM_MCU};

fn helios(serial: &str) -> DeviceKey {
    DeviceKey::new(SIM_HELIOS, serial)
}

fn mcu(serial: &str) -> DeviceKey {
    DeviceKey::new(SIM_MCU, serial)
}

fn atlas_for(fleet: &MockFleet) -> Atlas {
    atlas_with_store(fleet, Arc::new(MemoryStore::default()))
}

fn atlas_with_store(fleet: &MockFleet, store: Arc<dyn InventoryStore>) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(store);
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().expect("atlas builds")
}

fn cameras_request(version: &str, staged: StagedRollout) -> UpdateRequest {
    UpdateRequest {
        devices: vec![helios("H-1001"), helios("H-1002"), helios("H-1003")],
        releases: BTreeMap::from([(Family::new(SIM_HELIOS), ReleaseTarget::version(version))]),
        staged,
    }
}

#[tokio::test]
async fn scan_finds_direct_gateway_and_recovery_devices() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);

    let report = atlas.scan().await;

    assert_eq!(report.online.len(), 6, "warnings: {:?}", report.warnings);
    assert!(report.warnings.is_empty());
    let board = atlas.device(&mcu("M-2001")).unwrap();
    assert_eq!(
        board.link_kind,
        LinkKind::Gateway {
            via: helios("H-1001")
        }
    );
    let recovering = atlas.device(&mcu("M-2003")).unwrap();
    assert_eq!(recovering.identity.mode, DeviceMode::Recovery);
    assert_eq!(
        recovering.identity.primary_version(),
        Some("bootloader-2.1")
    );
}

#[tokio::test]
async fn devices_that_disappear_go_offline_and_stay_listed() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    fleet.set_online(&helios("H-1002"), false);
    let report = atlas.scan().await;

    assert_eq!(report.went_offline, vec![helios("H-1002")]);
    assert_eq!(
        atlas.device(&helios("H-1002")).unwrap().presence,
        Presence::Offline
    );
    assert_eq!(atlas.devices().len(), 6);
}

#[tokio::test]
async fn first_sighting_is_new_and_unchanged_rescans_are_quiet() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    let mut events = atlas.subscribe();

    atlas.scan().await;
    atlas.scan().await;

    let mut new_sightings = 0;
    let mut repeat_sightings = 0;
    while let Ok(event) = events.try_recv() {
        if let Event::DeviceSeen { new, .. } = event {
            if new {
                new_sightings += 1;
            } else {
                repeat_sightings += 1;
            }
        }
    }
    assert_eq!(new_sightings, 6);
    assert_eq!(repeat_sightings, 0);
}

#[tokio::test]
async fn bulk_update_runs_in_parallel_and_refreshes_versions() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas
        .start_update(cameras_request("2026.3.1", StagedRollout::Off))
        .unwrap();
    let record = atlas.wait_job(job).await.unwrap();

    assert_eq!(record.state, JobState::Finished);
    assert!(record.summary.unwrap().all_verified());
    assert_eq!(fleet.peak_running(), 3);
    for serial in ["H-1001", "H-1002", "H-1003"] {
        let device = atlas.device(&helios(serial)).unwrap();
        assert_eq!(device.identity.primary_version(), Some("2026.3.1"));
    }
}

#[tokio::test]
async fn one_failure_does_not_stop_the_others() {
    let fleet = MockFleet::flaky();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas
        .start_update(cameras_request("2026.3.1", StagedRollout::Off))
        .unwrap();
    let summary = atlas.wait_job(job).await.unwrap().summary.unwrap();

    assert_eq!(summary.verified, 2);
    assert_eq!(summary.rolled_back, 1);
    assert_eq!(
        fleet.device(&helios("H-1003")).unwrap().version,
        "2026.2.4",
        "a rolled-back device keeps its old version"
    );
}

#[tokio::test]
async fn staged_rollout_stops_when_the_canary_fails() {
    let fleet = MockFleet::flaky();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;
    let mut request = cameras_request("2026.3.1", StagedRollout::Auto);
    request.devices.reverse(); // H-1003, the flaky camera, goes first.

    let plan = atlas.plan_update(&request).unwrap();
    assert!(
        plan.devices[0].canary,
        "3 devices of one family turn staging on"
    );

    let job = atlas.start_update(request).unwrap();
    let record = atlas.wait_job(job).await.unwrap();

    let summary = record.summary.unwrap();
    assert_eq!(summary.rolled_back, 1);
    assert_eq!(summary.skipped, 2);
    assert!(matches!(
        &record.devices[1].status,
        DeviceJobStatus::Skipped { reason } if reason.contains("cam-rear")
    ));
}

#[tokio::test]
async fn devices_sharing_a_gateway_update_one_at_a_time() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas
        .start_update(UpdateRequest {
            devices: vec![mcu("M-2001"), mcu("M-2002")],
            releases: BTreeMap::from([(Family::new(SIM_MCU), ReleaseTarget::version("1.5.0"))]),
            staged: StagedRollout::Off,
        })
        .unwrap();
    let summary = atlas.wait_job(job).await.unwrap().summary.unwrap();

    assert_eq!(summary.verified, 2);
    assert_eq!(fleet.peak_running_for("gateway:sim-helios:H-1001"), 1);
}

#[tokio::test]
async fn recovery_brings_a_device_back_to_normal() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas
        .start_update(UpdateRequest {
            devices: vec![mcu("M-2003")],
            releases: BTreeMap::from([(Family::new(SIM_MCU), ReleaseTarget::version("1.5.0"))]),
            staged: StagedRollout::Off,
        })
        .unwrap();
    atlas.wait_job(job).await.unwrap();

    let device = atlas.device(&mcu("M-2003")).unwrap();
    assert_eq!(device.identity.mode, DeviceMode::Normal);
    assert_eq!(device.identity.primary_version(), Some("1.5.0"));
}

#[tokio::test]
async fn cancelling_before_apply_leaves_devices_untouched() {
    let fleet = MockFleet::new()
        .with(MockDevice::new(SIM_HELIOS, "slow-a").step_time(Duration::from_secs(5)))
        .with(MockDevice::new(SIM_HELIOS, "slow-b").step_time(Duration::from_secs(5)));
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas
        .start_update(UpdateRequest {
            devices: vec![helios("slow-a"), helios("slow-b")],
            releases: BTreeMap::from([(Family::new(SIM_HELIOS), ReleaseTarget::version("2.0.0"))]),
            staged: StagedRollout::Off,
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    atlas.cancel_job(job).unwrap();
    let record = atlas.wait_job(job).await.unwrap();

    assert_eq!(record.state, JobState::Cancelled);
    assert_eq!(record.summary.unwrap().cancelled, 2);
    assert_eq!(fleet.device(&helios("slow-a")).unwrap().version, "1.0.0");
}

#[tokio::test]
async fn offline_devices_are_rejected_before_a_job_starts() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;
    fleet.set_online(&helios("H-1002"), false);
    atlas.scan().await;

    let error = atlas
        .plan_update(&cameras_request("2026.3.1", StagedRollout::Off))
        .unwrap_err();

    assert!(matches!(error, CoreError::DeviceOffline(key) if key == helios("H-1002")));
}

#[tokio::test]
async fn a_release_is_required_for_every_family() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let error = atlas
        .plan_update(&UpdateRequest {
            devices: vec![helios("H-1001"), mcu("M-2001")],
            releases: BTreeMap::from([(
                Family::new(SIM_HELIOS),
                ReleaseTarget::version("2026.3.1"),
            )]),
            staged: StagedRollout::Off,
        })
        .unwrap_err();

    assert!(matches!(error, CoreError::NoReleaseForFamily(family) if family.as_str() == SIM_MCU));
}

#[tokio::test]
async fn inventory_is_remembered_between_sessions() {
    let path = std::env::temp_dir().join(format!(
        "atlas-core-session-{}-{:?}.json",
        std::process::id(),
        std::time::SystemTime::now()
    ));
    let fleet = MockFleet::demo();
    atlas_with_store(&fleet, Arc::new(JsonFileStore::new(&path)))
        .scan()
        .await;

    let next_session = Atlas::builder()
        .store(Arc::new(JsonFileStore::new(&path)))
        .build()
        .unwrap();

    let devices = next_session.devices();
    assert_eq!(devices.len(), 6);
    assert!(
        devices
            .iter()
            .all(|device| device.presence == Presence::Offline)
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn actions_run_on_online_devices_only() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    atlas.run_action(&helios("H-1001"), "locate").await.unwrap();
    let unknown = atlas.run_action(&helios("H-1001"), "self-destruct").await;
    let recovering = atlas.actions(&mcu("M-2003"));

    assert_eq!(
        fleet.actions_log(),
        vec![(helios("H-1001"), "locate".to_string())]
    );
    assert!(matches!(unknown, Err(CoreError::UnknownAction { .. })));
    assert!(matches!(recovering, Err(CoreError::NoActionsCapability(_))));
}
