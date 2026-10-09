use std::sync::Arc;
use std::time::Duration;

use atlas_core::{Atlas, MemoryStore, Presence, WatchOptions};
use atlas_driver::DeviceKey;
use atlas_driver_mock::{MockFleet, SIM_HELIOS};

fn atlas_for(fleet: &MockFleet) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(Arc::new(MemoryStore::default()));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().unwrap()
}

async fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn unplugging_is_noticed_without_polling() {
    let fleet = MockFleet::demo();
    let atlas = atlas_for(&fleet);
    // A fallback this long never fires in the test: only notices count.
    let stop = atlas
        .watch(WatchOptions {
            debounce: Duration::from_millis(10),
            fallback: Duration::from_secs(3600),
        })
        .unwrap();

    assert!(
        eventually(|| atlas.devices().len() == 6).await,
        "initial scan"
    );
    let status = atlas.discovery();
    assert!(status.live);
    assert_eq!(status.watching, vec!["Simulated".to_string()]);

    let rear = DeviceKey::new(SIM_HELIOS, "H-1003");
    fleet.set_online(&rear, false);
    assert!(
        eventually(|| atlas.device(&rear).unwrap().presence == Presence::Offline).await,
        "the unplug was noticed"
    );
    fleet.set_online(&rear, true);
    assert!(
        eventually(|| atlas.device(&rear).unwrap().presence == Presence::Online).await,
        "the replug was noticed"
    );

    stop.cancel();
    assert!(eventually(|| !atlas.discovery().live).await);
}
