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
