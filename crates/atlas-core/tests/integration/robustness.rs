use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use atlas_core::{
    Atlas, CoreError, DeviceJobStatus, JobState, MemoryStore, ReleaseTarget, StagedRollout,
    UpdateRequest,
};
use atlas_driver::{DeviceKey, Family};
use atlas_driver_mock::{MockBehavior, MockDevice, MockFleet, SIM_HELIOS};

fn atlas_for(fleet: &MockFleet) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(Arc::new(MemoryStore::default()));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().unwrap()
}

fn request(key: &DeviceKey) -> UpdateRequest {
    UpdateRequest {
        devices: vec![key.clone()],
        releases: BTreeMap::from([(Family::new(SIM_HELIOS), ReleaseTarget::version("2.0.0"))]),
        staged: StagedRollout::Off,
    }
}

#[tokio::test]
async fn a_panicking_driver_fails_the_device_instead_of_hanging() {
    let key = DeviceKey::new(SIM_HELIOS, "boom");
    let fleet =
        MockFleet::new().with(MockDevice::new(SIM_HELIOS, "boom").behavior(MockBehavior::Panics));
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas.start_update(request(&key)).unwrap();
    let record = tokio::time::timeout(Duration::from_secs(5), atlas.wait_job(job))
        .await
        .expect("the job finished")
        .unwrap();

    assert_eq!(record.state, JobState::Finished);
    match &record.devices[0].status {
        DeviceJobStatus::Failed { error } => assert!(error.contains("crashed"), "{error}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    // The device is free again.
    let again = atlas.start_update(request(&key)).unwrap();
    atlas.wait_job(again).await.unwrap();
}

#[tokio::test]
async fn a_device_cannot_be_in_two_jobs_at_once() {
    let key = DeviceKey::new(SIM_HELIOS, "slow");
    let fleet = MockFleet::new()
        .with(MockDevice::new(SIM_HELIOS, "slow").step_time(Duration::from_millis(200)));
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let first = atlas.start_update(request(&key)).unwrap();
    assert!(matches!(
        atlas.start_update(request(&key)),
        Err(CoreError::DeviceBusy(busy)) if busy == key
    ));
    atlas.wait_job(first).await.unwrap();
    assert!(atlas.start_update(request(&key)).is_ok());
}

#[tokio::test(start_paused = true)]
async fn cancel_ends_a_job_whose_driver_stopped_responding() {
    let key = DeviceKey::new(SIM_HELIOS, "stuck");
    let fleet =
        MockFleet::new().with(MockDevice::new(SIM_HELIOS, "stuck").behavior(MockBehavior::Hangs));
    let atlas = atlas_for(&fleet);
    atlas.scan().await;

    let job = atlas.start_update(request(&key)).unwrap();
    tokio::time::sleep(Duration::from_secs(5)).await;
    atlas.cancel_job(job).unwrap();
    let record = tokio::time::timeout(Duration::from_secs(120), atlas.wait_job(job))
        .await
        .expect("cancel ended the job")
        .unwrap();

    assert_eq!(record.state, JobState::Cancelled);
    match &record.devices[0].status {
        DeviceJobStatus::Failed { error } => assert!(error.contains("was let go"), "{error}"),
        other => panic!("expected the device to be let go, got {other:?}"),
    }
}

#[tokio::test]
async fn a_flashed_board_replaces_its_recovery_record() {
    // The same board as the boot ROM sees it, and as its running OS does.
    let recovery = DeviceKey::new("sim-boot", "e5226d57");
    let running = DeviceKey::new(SIM_HELIOS, "a317bcbee5226d57");
    let fleet = MockFleet::new()
        .with(
            MockDevice::new("sim-boot", "e5226d57")
                .recovery()
                .board("e5226d57"),
        )
        .with(MockDevice::new(SIM_HELIOS, "a317bcbee5226d57").board("e5226d57"));
    fleet.set_online(&running, false);
    let atlas = atlas_for(&fleet);
    atlas.scan().await;
    atlas
        .set_label(&recovery, Some("front cam".into()))
        .unwrap();

    // Flashed: it leaves USB boot and comes up running.
    fleet.set_online(&recovery, false);
    fleet.set_online(&running, true);
    atlas.scan().await;

    assert!(
        atlas.device(&recovery).is_none(),
        "the recovery record is gone"
    );
    let board = atlas.device(&running).unwrap();
    assert_eq!(board.label.as_deref(), Some("front cam"));
    assert!(
        atlas
            .activity(5)
            .iter()
            .any(|entry| entry.message.contains("left recovery")),
        "the history says what happened"
    );
}
