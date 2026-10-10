//! Board events numbered by the board (seq): caught up after a clock that
//! stepped back, paged, placed in this computer's time, and refetched after
//! the board's counter started over.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use atlas_core::{Atlas, Event, HistoryOrigin, MemoryStore, WatchOptions};
use atlas_driver::{
    DeviceEvent, DeviceKey, DeviceStatus, DriverError, EventPage, EventQuery, EventSource,
    Identity, PushSink, StatusCapability, StatusPush,
};
use atlas_driver_mock::{MockFleet, SIM_HELIOS};

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// A board that numbers its events. Its clock is `offset` seconds off in the
/// current boot.
#[derive(Default)]
struct SeqBoard {
    log: Mutex<Vec<DeviceEvent>>,
    boot: Mutex<(String, i64)>,
    asked: Mutex<Vec<EventQuery>>,
    /// With a push channel: the viewer's sink while one is connected.
    pushes: bool,
    sink: Mutex<Option<PushSink>>,
    /// Ends the open channel (the board restarted).
    drop_channel: tokio::sync::Notify,
    connects: Mutex<usize>,
}

impl SeqBoard {
    fn boot(&self, id: &str, offset: i64) {
        *self.boot.lock().unwrap() = (id.into(), offset);
    }

    /// Writes an event and pushes it, as board-stream would.
    fn write_and_push(&self, kind: &str) {
        self.write(kind);
        let event = self.log.lock().unwrap().last().cloned().unwrap();
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            sink(StatusPush::Event(Box::new(event)));
        }
    }

    /// Writes an event now, by the board's clock, with the next seq.
    fn write(&self, kind: &str) {
        let (boot, offset) = self.boot.lock().unwrap().clone();
        let mut log = self.log.lock().unwrap();
        let seq = log.last().and_then(|e| e.seq).unwrap_or(0) + 1;
        log.push(DeviceEvent {
            t: now_s() + offset,
            boot_id: boot,
            kind: kind.into(),
            source: EventSource::Local,
            message: format!("{kind} {seq}"),
            data: BTreeMap::new(),
            seq: Some(seq),
            uptime_s: Some(seq),
            at_ms: None,
        });
    }
}

#[async_trait::async_trait]
impl StatusCapability for SeqBoard {
    async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
        Ok(DeviceStatus::default())
    }

    async fn watch(&self, _device: &Identity, sink: PushSink) -> Result<bool, DriverError> {
        if !self.pushes {
            return Ok(false);
        }
        *self.connects.lock().unwrap() += 1;
        let (boot, _) = self.boot.lock().unwrap().clone();
        let newest_seq = self.log.lock().unwrap().last().and_then(|e| e.seq);
        sink(StatusPush::Hello {
            boot_id: boot,
            newest_seq,
        });
        *self.sink.lock().unwrap() = Some(sink);
        self.drop_channel.notified().await;
        *self.sink.lock().unwrap() = None;
        Ok(true)
    }

    async fn event_page(
        &self,
        _device: &Identity,
        query: EventQuery,
    ) -> Result<EventPage, DriverError> {
        self.asked.lock().unwrap().push(query);
        let log = self.log.lock().unwrap();
        let events: Vec<DeviceEvent> = match query.after_seq {
            Some(after) => log
                .iter()
                .filter(|e| e.seq.unwrap_or(0) > after)
                .take(query.limit)
                .cloned()
                .collect(),
            None => {
                let from = query.since.unwrap_or(0);
                let matching: Vec<DeviceEvent> =
                    log.iter().filter(|e| e.t >= from).cloned().collect();
                let skip = matching.len().saturating_sub(query.limit);
                matching.into_iter().skip(skip).collect()
            }
        };
        let (boot, offset) = self.boot.lock().unwrap().clone();
        Ok(EventPage {
            events,
            time: Some(now_s() + offset),
            boot_id: Some(boot),
            newest_seq: log.last().and_then(|e| e.seq),
        })
    }
}

struct Source(DeviceKey, Arc<SeqBoard>);

impl atlas_driver::CapabilitySource for Source {
    fn capabilities_for(&self, device: &Identity) -> atlas_driver::Capabilities {
        atlas_driver::Capabilities {
            status: (device.key == self.0).then(|| self.1.clone() as Arc<dyn StatusCapability>),
            ..atlas_driver::Capabilities::default()
        }
    }
}

async fn atlas_with(board: Arc<SeqBoard>) -> (Atlas, DeviceKey) {
    let camera = DeviceKey::new(SIM_HELIOS, "H-1001");
    let fleet = MockFleet::demo();
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .capability_source(Arc::new(Source(camera.clone(), board)))
        .store(Arc::new(MemoryStore::default()));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    let atlas = builder.build().unwrap();
    atlas.scan().await;
    (atlas, camera)
}

fn board_kinds(atlas: &Atlas, key: &DeviceKey) -> Vec<String> {
    atlas
        .device_history(key, 2000)
        .into_iter()
        .filter(|entry| entry.origin == HistoryOrigin::Board)
        .map(|entry| entry.message)
        .collect()
}

#[tokio::test]
async fn events_after_a_clock_step_back_are_caught_up_by_seq() {
    let board = Arc::new(SeqBoard::default());
    board.boot("b1", 0);
    board.write("boot");
    board.write("update.apply");
    let (atlas, camera) = atlas_with(board.clone()).await;
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 2);

    // The next boot's clock is three hours behind: by time, its events
    // would all be before the newest kept one, and never fetched.
    board.boot("b2", -3 * 3600);
    board.write("boot");
    board.write("update.confirmed");
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 2);
    assert_eq!(
        board.asked.lock().unwrap().last().unwrap().after_seq,
        Some(2),
        "asked after the last seq, not since a time"
    );

    // Placed in this computer's time: newest first, by the corrected clock.
    let history = atlas.device_history(&camera, 10);
    let newest = history
        .iter()
        .find(|entry| entry.origin == HistoryOrigin::Board)
        .unwrap();
    assert_eq!(newest.message, "update.confirmed 4");
    let here_ms = now_s() as u64 * 1000;
    assert!(
        newest.at_ms.abs_diff(here_ms) < 5_000,
        "corrected to now, not three hours ago: {} vs {here_ms}",
        newest.at_ms
    );
    assert_eq!(
        board_kinds(&atlas, &camera),
        ["update.confirmed 4", "boot 3", "update.apply 2", "boot 1"]
    );

    // Nothing new: nothing added, and the same seq again is no duplicate.
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 0);
}

#[tokio::test]
async fn a_long_absence_is_paged_until_caught_up() {
    let board = Arc::new(SeqBoard::default());
    board.boot("b1", 0);
    board.write("boot");
    let (atlas, camera) = atlas_with(board.clone()).await;
    atlas.sync_device_events(&camera).await.unwrap();
    for _ in 0..450 {
        board.write("n");
    }
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 450);
    let pages: Vec<Option<u64>> = board
        .asked
        .lock()
        .unwrap()
        .iter()
        .skip(1)
        .map(|q| q.after_seq)
        .collect();
    assert_eq!(pages, [Some(1), Some(201), Some(401)]);
}

#[tokio::test]
async fn a_counter_that_started_over_is_fetched_from_its_start() {
    let board = Arc::new(SeqBoard::default());
    board.boot("b1", 0);
    for _ in 0..5 {
        board.write("n");
    }
    let (atlas, camera) = atlas_with(board.clone()).await;
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 5);

    // The board's /data was replaced: a new boot numbers from 1 again.
    board.log.lock().unwrap().clear();
    board.boot("b2", 0);
    board.write("boot");
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 1);
    assert_eq!(atlas.sync_device_events(&camera).await.unwrap(), 0);
}

/// Waits up to 5 s for `done`.
async fn eventually(what: &str, done: impl Fn() -> bool) {
    for _ in 0..250 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{what} didn't happen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pushed_event_is_in_the_history_at_once() {
    let board = Arc::new(SeqBoard {
        pushes: true,
        ..SeqBoard::default()
    });
    board.boot("b1", 0);
    board.write("boot");
    let (atlas, camera) = atlas_with(board.clone()).await;
    let mut events = atlas.subscribe();
    // A long fallback: only the push can make this quick.
    let watching = atlas
        .watch(WatchOptions {
            debounce: std::time::Duration::from_millis(10),
            fallback: std::time::Duration::from_secs(3600),
        })
        .unwrap();
    eventually("the channel opens", || board.sink.lock().unwrap().is_some()).await;
    eventually("the hello catches up", || {
        board_kinds(&atlas, &camera).len() == 1
    })
    .await;

    board.write_and_push("update.apply");
    eventually("the pushed event is kept", || {
        board_kinds(&atlas, &camera).first().map(String::as_str) == Some("update.apply 2")
    })
    .await;
    let mut told = false;
    while let Ok(event) = events.try_recv() {
        told |= matches!(event, Event::DeviceStatus { ref key } if *key == camera);
    }
    assert!(told, "hosts are told to re-read the status");

    // The board restarts: the channel drops, Atlas looks again and reconnects.
    board.boot("b2", -3600);
    board.drop_channel.notify_waiters();
    eventually("it reconnects", || *board.connects.lock().unwrap() >= 2).await;
    board.write_and_push("boot");
    eventually("the new boot's event arrives", || {
        board_kinds(&atlas, &camera).first().map(String::as_str) == Some("boot 3")
    })
    .await;
    watching.cancel();
}

#[tokio::test]
async fn a_device_without_a_channel_is_polled_as_before() {
    let board = Arc::new(SeqBoard::default());
    board.boot("b1", 0);
    let (atlas, _camera) = atlas_with(board.clone()).await;
    let watching = atlas
        .watch(WatchOptions {
            debounce: std::time::Duration::from_millis(10),
            fallback: std::time::Duration::from_secs(3600),
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(*board.connects.lock().unwrap(), 0);
    watching.cancel();
}
