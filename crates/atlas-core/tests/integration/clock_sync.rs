//! Boards whose clock is off get this computer's time while Atlas watches,
//! by the clock policy, at most once per board every 10 minutes.

use std::time::Duration;

use atlas_core::{Atlas, ClockSync, MemoryStore, WatchOptions};
use atlas_driver::{DeviceKey, attributes};
use atlas_driver_mock::{MockDevice, MockFleet, SIM_HELIOS};

fn atlas_for(fleet: &MockFleet) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(std::sync::Arc::new(MemoryStore::default()));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().unwrap()
}

fn sets(fleet: &MockFleet, key: &DeviceKey) -> usize {
    fleet
        .actions_log()
        .iter()
        .filter(|(device, action)| device == key && action == "set-clock")
        .count()
}

fn watch(atlas: &Atlas, clock_sync: ClockSync) -> tokio_util::sync::CancellationToken {
    atlas
        .watch(WatchOptions {
            debounce: Duration::from_millis(10),
            fallback: Duration::from_millis(50),
            clock_sync,
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_three_hours_off_gets_this_computers_time() {
    let key = DeviceKey::new(SIM_HELIOS, "H-7");
    let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-7").clock_off(-3 * 3600));
    let atlas = atlas_for(&fleet);
    let stop = watch(&atlas, ClockSync::All);
    for _ in 0..200 {
        let synced = atlas.device(&key).is_some_and(|record| {
            !record
                .identity
                .attributes
                .contains_key(attributes::CLOCK_OFFSET_S)
        });
        if sets(&fleet, &key) == 1 && synced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(sets(&fleet, &key), 1, "set once");
    assert!(
        !atlas
            .device(&key)
            .unwrap()
            .identity
            .attributes
            .contains_key(attributes::CLOCK_OFFSET_S),
        "the rescan after it shows the clock in step"
    );
    stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_that_wont_take_it_isnt_asked_every_scan() {
    let key = DeviceKey::new(SIM_HELIOS, "H-8");
    let fleet = MockFleet::new().with(
        MockDevice::new(SIM_HELIOS, "H-8")
            .clock_off(40)
            .clock_stuck(),
    );
    let atlas = atlas_for(&fleet);
    let stop = watch(&atlas, ClockSync::All);
    // Many scans (every 50 ms) later: still asked once.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(sets(&fleet, &key), 1);
    stop.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_policy_decides_which_boards() {
    let key = DeviceKey::new(SIM_HELIOS, "H-9");
    for policy in [ClockSync::Off, ClockSync::Usb] {
        let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-9").clock_off(-500));
        let atlas = atlas_for(&fleet);
        let stop = watch(&atlas, policy);
        tokio::time::sleep(Duration::from_millis(300)).await;
        // Off: never; USB only: a simulated link isn't USB.
        assert_eq!(sets(&fleet, &key), 0, "{policy:?}");
        stop.cancel();
    }
}
