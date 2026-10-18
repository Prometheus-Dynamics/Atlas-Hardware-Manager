//! What an idle board is asked while Atlas watches it, per minute, on
//! Tokio's paused clock: scans (identity), event syncs, status reads and
//! push-channel connects, for a board that pushes, one without a push
//! channel, one whose channel won't open, and one whose own status has no
//! channel while Orion's does. The device's page is open the
//! whole time, reading the status as the desktop's status store does.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_core::{Atlas, MemoryStore, WatchOptions};
use atlas_driver::{
    Candidate, Capabilities, CapabilitySource, DeviceKey, DeviceStatus, Driver, DriverError,
    DriverManifest, EventPage, EventQuery, Identity, Link, PushSink, StatusCapability, StatusPush,
};
use atlas_driver_mock::{MockDevice, MockFleet, SIM_HELIOS};

const MINUTES: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Channel {
    /// Pushes (board-stream, or Orion's status lane).
    Open,
    /// The board has none (an older OS).
    None,
    /// It has one that won't open (board-stream down).
    Refused,
    /// Its own status has none; Orion's (a source whose data comes first)
    /// pushes.
    OrionOnly,
}

#[derive(Default)]
struct Counts {
    identify: AtomicUsize,
    status: AtomicUsize,
    events: AtomicUsize,
    connects: AtomicUsize,
}

struct Board {
    channel: Channel,
    counts: Arc<Counts>,
    sink: Mutex<Option<PushSink>>,
}

#[async_trait]
impl StatusCapability for Board {
    async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
        self.counts.status.fetch_add(1, Ordering::Relaxed);
        Ok(DeviceStatus::default())
    }

    async fn watch(&self, _device: &Identity, sink: PushSink) -> Result<bool, DriverError> {
        match self.channel {
            Channel::None | Channel::OrionOnly => Ok(false),
            Channel::Refused => {
                self.counts.connects.fetch_add(1, Ordering::Relaxed);
                Err(DriverError::Unreachable("connection refused".into()))
            }
            Channel::Open => {
                self.counts.connects.fetch_add(1, Ordering::Relaxed);
                sink(StatusPush::Hello {
                    boot_id: "b1".into(),
                    newest_seq: Some(1),
                });
                *self.sink.lock().unwrap() = Some(sink);
                std::future::pending::<()>().await;
                Ok(true)
            }
        }
    }

    async fn event_page(
        &self,
        _device: &Identity,
        _query: EventQuery,
    ) -> Result<EventPage, DriverError> {
        self.counts.events.fetch_add(1, Ordering::Relaxed);
        Ok(EventPage {
            boot_id: Some("b1".into()),
            newest_seq: Some(1),
            ..EventPage::default()
        })
    }
}

/// The board's status; `data_first` for Orion's.
struct Source(DeviceKey, Arc<Board>, bool);

impl CapabilitySource for Source {
    fn data_first(&self) -> bool {
        self.2
    }

    fn capabilities_for(&self, device: &Identity) -> Capabilities {
        Capabilities {
            status: (device.key == self.0).then(|| self.1.clone() as Arc<dyn StatusCapability>),
            ..Capabilities::default()
        }
    }
}

/// Counts identity requests, the one request a scan makes of a board.
struct Counted(Arc<dyn Driver>, Arc<Counts>);

#[async_trait]
impl Driver for Counted {
    fn manifest(&self) -> &DriverManifest {
        self.0.manifest()
    }
    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        self.0.discover(link).await
    }
    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        self.1.identify.fetch_add(1, Ordering::Relaxed);
        self.0.identify(candidate).await
    }
    fn capabilities(&self, device: &Identity) -> Capabilities {
        self.0.capabilities(device)
    }
}

/// Requests per minute an idle board gets, with its page open.
#[derive(Debug)]
struct PerMinute {
    identify: f64,
    status: f64,
    events: f64,
    connects: f64,
}

impl PerMinute {
    fn total(&self) -> f64 {
        self.identify + self.status + self.events + self.connects
    }
}

async fn idle(channel: Channel) -> PerMinute {
    let key = DeviceKey::new(SIM_HELIOS, "H-1001");
    let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-1001"));
    let counts = Arc::new(Counts::default());
    let board = |channel| {
        Arc::new(Board {
            channel,
            counts: counts.clone(),
            sink: Mutex::new(None),
        })
    };
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .capability_source(Arc::new(Source(key.clone(), board(channel), false)))
        .store(Arc::new(MemoryStore::default()));
    if channel == Channel::OrionOnly {
        builder =
            builder.capability_source(Arc::new(Source(key.clone(), board(Channel::Open), true)));
    }
    for driver in fleet.drivers() {
        builder = builder.driver(Arc::new(Counted(driver, counts.clone())));
    }
    let atlas = builder.build().unwrap();
    let stop = atlas.watch(WatchOptions::default()).unwrap();

    // The device's page: the status store's idle poll (apps/desktop
    // stores/status.svelte.ts: IDLE_MS, PUSHED_MS for a board that pushes).
    let view = {
        let atlas = atlas.clone();
        let key = key.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            loop {
                let pushed = atlas.device_status(&key).await.is_ok_and(|s| s.pushed);
                tokio::time::sleep(Duration::from_secs(if pushed { 60 } else { 15 })).await;
            }
        })
    };

    // Settle (first scan, channel opened), then count a steady stretch.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let before = snapshot(&counts);
    tokio::time::sleep(Duration::from_secs(MINUTES * 60)).await;
    let after = snapshot(&counts);
    view.abort();
    stop.cancel();
    let per = |i: usize| (after[i] - before[i]) as f64 / MINUTES as f64;
    PerMinute {
        identify: per(0),
        status: per(1),
        events: per(2),
        connects: per(3),
    }
}

fn snapshot(counts: &Counts) -> [usize; 4] {
    [
        counts.identify.load(Ordering::Relaxed),
        counts.status.load(Ordering::Relaxed),
        counts.events.load(Ordering::Relaxed),
        counts.connects.load(Ordering::Relaxed),
    ]
}

// Before pushes replaced polls (default 20 s fallback, the page open):
// Open 11.0/min (identity 3, status 4, events 4), None 11.0, Refused 17.0
// (and 6 connects).
#[tokio::test(start_paused = true)]
#[allow(clippy::print_stdout, reason = "the rates are the measurement")]
async fn an_idle_board_is_asked_little() {
    for channel in [
        Channel::Open,
        Channel::None,
        Channel::Refused,
        Channel::OrionOnly,
    ] {
        let rate = idle(channel).await;
        println!(
            "{channel:?}: {:.1}/min (identity {:.1}, status {:.1}, events {:.1}, connects {:.1})",
            rate.total(),
            rate.identify,
            rate.status,
            rate.events,
            rate.connects
        );
        match channel {
            // Pushes cover events and the update state: a scan and a status
            // read a minute, and an event sync every ten.
            Channel::Open => {
                assert!(rate.identify <= 1.0, "{rate:?}");
                assert!(rate.status <= 1.0, "{rate:?}");
                assert!(rate.events <= 0.2, "{rate:?}");
                assert!(rate.connects == 0.0, "{rate:?}");
            }
            // No push: polled as before.
            Channel::None => assert!(rate.total() <= 11.0, "{rate:?}"),
            // A channel that won't open is tried less and less often.
            Channel::Refused => assert!(rate.connects <= 0.5, "{rate:?}"),
            // Orion's channel stands in for the board's own: as Open, with
            // each status read asking both sources.
            Channel::OrionOnly => {
                assert!(rate.identify <= 1.0, "{rate:?}");
                assert!(rate.status <= 2.0, "{rate:?}");
                assert!(rate.events <= 0.2, "{rate:?}");
                assert!(rate.connects == 0.0, "{rate:?}");
            }
        }
    }
}
