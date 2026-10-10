//! A board's status and event log merged into its device history.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use atlas_core::{ActivityKind, Atlas, HistoryOrigin, MemoryStore};
use atlas_driver::{
    CapabilityKind, DeviceEvent, DeviceKey, DeviceStatus, DriverError, EventSource, Identity,
    StatusCapability, UpdateState,
};
use atlas_driver_mock::{MockFleet, SIM_HELIOS};

/// A board's event log: `events` answers with what is there at or after
/// `since`, and records each `since` it was asked for.
#[derive(Default)]
struct FakeBoard {
    log: Mutex<Vec<DeviceEvent>>,
    asked: Mutex<Vec<Option<i64>>>,
}

fn event(t: i64, kind: &str, source: EventSource) -> DeviceEvent {
    DeviceEvent {
        t,
        boot_id: "b1".into(),
        kind: kind.into(),
        source,
        message: format!("{kind} at {t}"),
        seq: None,
        uptime_s: None,
        at_ms: None,
        data: BTreeMap::new(),
    }
}

#[async_trait::async_trait]
impl StatusCapability for FakeBoard {
    async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
        Ok(DeviceStatus {
            update: Some(UpdateState {
                state: "staging".into(),
                progress: 420,
                started_by: Some(EventSource::Orion),
                ..UpdateState::default()
            }),
            ..DeviceStatus::default()
        })
    }

    async fn events(
        &self,
        _device: &Identity,
        since: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DeviceEvent>, DriverError> {
        self.asked.lock().unwrap().push(since);
        let log = self.log.lock().unwrap();
        let from = since.unwrap_or(0);
        let matching: Vec<DeviceEvent> = log.iter().filter(|e| e.t >= from).cloned().collect();
        let skip = matching.len().saturating_sub(limit);
        Ok(matching.into_iter().skip(skip).collect())
    }
}

struct Source(DeviceKey, Arc<FakeBoard>);

impl atlas_driver::CapabilitySource for Source {
    fn capabilities_for(&self, device: &Identity) -> atlas_driver::Capabilities {
        atlas_driver::Capabilities {
            status: (device.key == self.0).then(|| self.1.clone() as Arc<dyn StatusCapability>),
            ..atlas_driver::Capabilities::default()
        }
    }
}

#[tokio::test]
async fn board_events_merge_into_the_device_history() {
    let camera = DeviceKey::new(SIM_HELIOS, "H-1001");
    let board = Arc::new(FakeBoard::default());
    board.log.lock().unwrap().extend([
        event(100, "boot", EventSource::Local),
        event(110, "update.stage", EventSource::Orion),
    ]);
    let fleet = MockFleet::demo();
    let store = Arc::new(MemoryStore::default());
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .capability_source(Arc::new(Source(camera.clone(), board.clone())))
        .store(store.clone());
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    let atlas = builder.build().unwrap();
    atlas.scan().await;
    assert!(
        atlas
            .device(&camera)
            .unwrap()
            .capabilities
            .contains(&CapabilityKind::Status)
    );

    // Reading the status fetches the board's past; that isn't fleet news.
    let status = atlas.device_status(&camera).await.unwrap();
    assert_eq!(status.update.unwrap().started_by, Some(EventSource::Orion));
    let history = atlas.device_history(&camera, 100);
    let board_lines: Vec<(&str, EventSource)> = history
        .iter()
        .filter(|entry| entry.origin == HistoryOrigin::Board)
        .map(|entry| (entry.kind.as_str(), entry.source))
        .collect();
    assert_eq!(
        board_lines,
        [
            ("update.stage", EventSource::Orion),
            ("boot", EventSource::Local)
        ]
    );
    assert!(
        history
            .iter()
            .any(|entry| entry.origin == HistoryOrigin::Atlas && entry.source == EventSource::Atlas)
    );
    assert!(
        !atlas
            .activity(100)
            .iter()
            .any(|entry| entry.kind == ActivityKind::DeviceEvent)
    );

    // The next fetch starts at the newest kept event; repeats are dropped,
    // and an outcome someone else caused reaches the fleet history.
    board.log.lock().unwrap().extend([
        event(150, "update.staged", EventSource::Orion),
        event(160, "update.confirmed", EventSource::Orion),
        event(170, "clock.set", EventSource::Atlas),
    ]);
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 3);
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 0);
    // (The scan's own sync may have asked first, too.)
    let asked = board.asked.lock().unwrap().clone();
    assert_eq!(asked[asked.len() - 2..], [Some(110), Some(170)]);
    let news: Vec<String> = atlas
        .activity(100)
        .into_iter()
        .filter(|entry| entry.kind == ActivityKind::DeviceEvent)
        .map(|entry| entry.message)
        .collect();
    assert_eq!(news.len(), 2, "{news:?}");
    assert!(news.iter().all(|line| line.ends_with("(through Orion)")));
    assert_eq!(
        atlas
            .device_history(&camera, 100)
            .iter()
            .filter(|entry| entry.origin == HistoryOrigin::Board)
            .count(),
        5
    );

    // The events are kept with the inventory.
    let saved = atlas_core::InventoryStore::load(store.as_ref()).unwrap();
    assert_eq!(saved.board_events.len(), 1);
    assert_eq!(saved.board_events[0].events.len(), 5);
}
