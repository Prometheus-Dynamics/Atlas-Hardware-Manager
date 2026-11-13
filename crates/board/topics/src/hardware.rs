//! A node's hardware as the app shows it: what the node describes, its readings now, its logs,
//! and whether any of that is current.
//!
//! [`Hardware::snapshot`] makes one read on the node's session. A read that cannot be made says
//! so in the snapshot's [`Connection`] and keeps the last devices, each stale since the last time
//! it was current: nothing is an error and nothing is silently frozen.
//!
//! A device the node stops listing for a moment (the bridge's session is published again when a
//! fact appears or disappears, which withdraws its devices briefly) is held with its last
//! description for [`DEVICE_GRACE`], marked not listed and stale. A device that stays gone longer
//! is removed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use atlas_session::{Handle, Presence, SessionError};
use atlas_types::{Identity, LinkId};

use crate::device::{Availability, Device, Freshness, RawDevice, describe};
use crate::logs::{LogLine, LogRing};
use crate::read::{self, Read};
use crate::value::Reported;

/// How long a device the node has stopped listing keeps its last description. Lemnos publishes
/// the bridge's session again when a fact appears or disappears, which withdraws the devices for
/// that republish; the identity facts are read every two seconds, so one republish takes about
/// that long plus the session's own time. Five seconds covers several of them. A device gone
/// for longer than that is really gone.
pub const DEVICE_GRACE: Duration = Duration::from_secs(5);

/// The node's session, as the last read saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    /// The last read was made.
    Live,
    /// The last read could not be made. Everything shown is from `since`, the last time a read
    /// was made, and `reason` is what stopped it.
    Stale {
        /// The last time a read was made.
        since: Instant,
        /// Why the read was not made.
        reason: String,
    },
}

/// Everything the app shows of one node's hardware at one read.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Whether the read was made.
    pub connection: Connection,
    /// Who the node is. `None` until the first read on a session that makes one.
    pub identity: Option<Identity>,
    /// The devices the node lists, and those held within the grace, by name.
    pub devices: Vec<Device>,
    /// The newest log lines, oldest first (see [`crate::LOG_CAPACITY`]).
    pub logs: Vec<LogLine>,
    /// How many log lines were lost before this app read them.
    pub logs_missed: u64,
}

/// The node's hardware, read over its session. One per node: the session keeps the node's
/// subscriptions for this crate, and a second reader of the same node would share them.
#[derive(Debug)]
pub struct Hardware {
    handle: Handle,
    link: LinkId,
    grace: Duration,
    state: Mutex<State>,
}

/// A device held by the crate, with when it was last current.
#[derive(Clone, Debug)]
struct Held {
    device: Device,
    /// The last time the node listed it.
    last_seen: Instant,
    /// The last time it was current.
    last_fresh: Instant,
    /// Since when it has not been current, `None` while it is.
    stale_since: Option<Instant>,
}

#[derive(Debug)]
struct State {
    connection: Connection,
    /// The last time a read was made.
    last_live: Option<Instant>,
    identity: Option<Identity>,
    devices: BTreeMap<String, Held>,
    logs: LogRing,
    logs_missed: u64,
}

impl Hardware {
    /// The hardware of the node `handle` keeps, on the link `link` reaches it over. Devices held
    /// for the grace use [`DEVICE_GRACE`].
    #[must_use]
    pub fn new(handle: Handle, link: LinkId) -> Self {
        Self::with_grace(handle, link, DEVICE_GRACE)
    }

    /// As [`Hardware::new`], with the grace chosen by the caller.
    #[must_use]
    pub fn with_grace(handle: Handle, link: LinkId, grace: Duration) -> Self {
        Self {
            handle,
            link,
            grace,
            state: Mutex::new(State {
                connection: Connection::Stale {
                    since: Instant::now(),
                    reason: "not read yet".into(),
                },
                last_live: None,
                identity: None,
                devices: BTreeMap::new(),
                logs: LogRing::default(),
                logs_missed: 0,
            }),
        }
    }

    /// Reads the node now and returns what the app shows. Never an error: a read that cannot be
    /// made is in the [`Connection`], and the devices shown are stale.
    pub async fn snapshot(&self) -> Snapshot {
        let address = self.handle.address().to_string();
        // The connection this read is for: the node as the monitor saw it when it connected.
        let session = match self.handle.presence() {
            Presence::Found(node) => format!("{}/{}", node.fingerprint, node.time_ms),
            _ => String::new(),
        };
        let link = self.link.clone();
        let key = address.clone();
        let answer = self
            .handle
            .with_client(move |orion| read::read(orion, &key, &session, &link))
            .await;
        let now = Instant::now();
        let mut state = lock(&self.state);
        match answer {
            Ok(Ok(read)) => state.absorb(read, now, self.grace),
            Ok(Err(reason)) => state.lose(reason, now),
            Err(SessionError::Gone(reason) | SessionError::Failed(reason)) => {
                state.lose(reason, now);
            }
        }
        state.snapshot()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Held {
    /// A device as the node lists it now.
    fn new(device: Device, now: Instant) -> Self {
        let mut held = Self {
            device,
            last_seen: now,
            last_fresh: now,
            stale_since: None,
        };
        held.observe(now);
        held
    }

    /// Takes the device as the node lists it now, and says whether it is current.
    fn observe_listed(&mut self, device: Device, now: Instant) {
        self.device = device;
        self.last_seen = now;
        self.observe(now);
    }

    /// Records whether the device is current at `now`.
    fn observe(&mut self, now: Instant) {
        if is_current(&self.device) {
            self.last_fresh = now;
            self.stale_since = None;
        } else {
            self.stale_since.get_or_insert(self.last_fresh);
        }
    }
}

/// Whether a listed device's readings are current: it is listed, its publisher is there, and its
/// provider has not gone.
fn is_current(device: &Device) -> bool {
    device.listed && !device.gone && device.availability != Reported::Value(Availability::Lost)
}

impl State {
    /// Takes a read the node made.
    fn absorb(&mut self, read: Read, now: Instant, grace: Duration) {
        self.connection = Connection::Live;
        self.last_live = Some(now);
        self.identity = Some(read.identity);
        self.logs_missed = self.logs_missed.saturating_add(read.logs_missed);
        self.absorb_logs(read.logs);
        self.observe_devices(read.devices, now, grace);
    }

    /// Takes the devices the node lists, and holds those it has stopped listing for the grace.
    fn observe_devices(&mut self, devices: Vec<RawDevice>, now: Instant, grace: Duration) {
        let mut listed = BTreeSet::new();
        for raw in devices {
            let device = describe(raw);
            listed.insert(device.name.clone());
            match self.devices.get_mut(&device.name) {
                Some(held) => held.observe_listed(device.clone(), now),
                None => {
                    self.devices
                        .insert(device.name.clone(), Held::new(device, now));
                }
            }
        }
        for (name, held) in &mut self.devices {
            if !listed.contains(name) {
                held.device.listed = false;
                held.stale_since.get_or_insert(held.last_fresh);
            }
        }
        self.devices.retain(|name, held| {
            listed.contains(name) || now.duration_since(held.last_seen) <= grace
        });
    }

    /// Takes the log lines the read found, skipping the ones the ring already holds: a new
    /// session replays the node's log topic, and those lines were taken before.
    fn absorb_logs(&mut self, lines: Vec<(u64, String)>) {
        for (time, text) in lines {
            let line = LogLine { time, text };
            if !self.logs.contains(&line) {
                self.logs.push(line);
            }
        }
    }

    /// A read that could not be made: the node is stale since the last read that was made.
    fn lose(&mut self, reason: String, now: Instant) {
        let since = self.last_live.unwrap_or(now);
        self.connection = Connection::Stale { since, reason };
    }

    fn snapshot(&self) -> Snapshot {
        let connection_stale = match &self.connection {
            Connection::Stale { since, .. } => Some(*since),
            Connection::Live => None,
        };
        let devices = self
            .devices
            .values()
            .map(|held| {
                let mut device = held.device.clone();
                device.freshness = match held.stale_since.or(connection_stale) {
                    Some(since) => Freshness::Stale { since },
                    None => Freshness::Live,
                };
                device
            })
            .collect();
        Snapshot {
            connection: self.connection.clone(),
            identity: self.identity.clone(),
            devices,
            logs: self.logs.iter().cloned().collect(),
            logs_missed: self.logs_missed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{RawField, RawTopic, TopicLive};
    use crate::value::Scalar;
    use orion::provider::ResourceState;

    fn raw(name: &str, value: f64) -> RawDevice {
        RawDevice {
            resource: name.into(),
            state: ResourceState::Ready,
            labels: Vec::new(),
            topics: vec![RawTopic {
                name: format!("{name}.values"),
                live: TopicLive::Live,
                fields: vec![RawField {
                    name: "speed".into(),
                    unit: "m/s".into(),
                    value: Some(Scalar::Number(value)),
                }],
            }],
        }
    }

    fn state() -> State {
        State {
            connection: Connection::Live,
            last_live: None,
            identity: None,
            devices: BTreeMap::new(),
            logs: LogRing::default(),
            logs_missed: 0,
        }
    }

    #[test]
    fn a_device_the_node_stops_listing_is_held_then_removed_after_the_grace() {
        let grace = Duration::from_secs(5);
        let t0 = Instant::now();
        let mut state = state();
        state.observe_devices(vec![raw("lemnos.a", 1.0)], t0, grace);

        // The republish withdraws it: held, not listed, stale since it was last current.
        let t1 = t0 + Duration::from_secs(1);
        state.observe_devices(Vec::new(), t1, grace);
        let held = state.snapshot().devices;
        assert_eq!(held.len(), 1, "a blip does not remove the device");
        assert!(!held[0].listed);
        assert_eq!(held[0].freshness, Freshness::Stale { since: t0 });

        // It is listed again within the grace: back, and current.
        let t2 = t0 + Duration::from_secs(2);
        state.observe_devices(vec![raw("lemnos.a", 2.0)], t2, grace);
        let back = state.snapshot().devices;
        assert!(back[0].listed);
        assert_eq!(back[0].freshness, Freshness::Live);

        // Gone for longer than the grace: removed.
        let t3 = t2 + Duration::from_secs(1);
        state.observe_devices(Vec::new(), t3, grace);
        let t4 = t3 + grace + Duration::from_millis(1);
        state.observe_devices(Vec::new(), t4, grace);
        assert!(state.snapshot().devices.is_empty());
    }

    #[test]
    fn a_lost_session_keeps_the_devices_stale_since_the_last_read_that_was_made() {
        let t0 = Instant::now();
        let mut state = state();
        state.observe_devices(vec![raw("lemnos.a", 1.0)], t0, DEVICE_GRACE);
        state.last_live = Some(t0);

        let t1 = t0 + Duration::from_secs(3);
        state.lose("the node is gone".into(), t1);
        let snapshot = state.snapshot();
        assert_eq!(
            snapshot.connection,
            Connection::Stale {
                since: t0,
                reason: "the node is gone".into()
            }
        );
        assert_eq!(snapshot.devices.len(), 1, "a lost session removes nothing");
        assert_eq!(
            snapshot.devices[0].freshness,
            Freshness::Stale { since: t0 }
        );
    }

    #[test]
    fn a_lost_publisher_makes_its_device_stale_and_a_return_makes_it_live() {
        let t0 = Instant::now();
        let mut state = state();
        let mut gone = raw("lemnos.a", 1.0);
        gone.topics[0].live = TopicLive::Gone;
        state.observe_devices(vec![gone], t0, DEVICE_GRACE);
        assert!(matches!(
            state.snapshot().devices[0].freshness,
            Freshness::Stale { .. }
        ));

        let t1 = t0 + Duration::from_secs(1);
        state.observe_devices(vec![raw("lemnos.a", 2.0)], t1, DEVICE_GRACE);
        assert_eq!(state.snapshot().devices[0].freshness, Freshness::Live);
    }

    #[test]
    fn log_lines_already_held_are_not_taken_twice_and_a_restarted_log_is_taken() {
        let mut state = state();
        state.absorb_logs(vec![(1, "a".into()), (2, "b".into())]);
        // A new session replays the topic: the lines held are not taken again.
        state.absorb_logs(vec![(1, "a".into()), (2, "b".into()), (3, "c".into())]);
        let texts: Vec<_> = state.logs.iter().map(|line| line.text.clone()).collect();
        assert_eq!(texts, ["a", "b", "c"]);

        // A node that began its log again has new times and new lines: they are taken.
        state.absorb_logs(vec![(1, "again".into())]);
        assert_eq!(
            state.logs.iter().last().map(|line| line.text.as_str()),
            Some("again")
        );
    }
}
