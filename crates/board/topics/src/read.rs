//! Reads a node's devices, readings and logs on its live session.
//!
//! This runs only inside [`atlas_session::Handle::with_client`], on the node's monitor thread,
//! which is the one place Orion's client may be used. A subscription is made once for each
//! session and kept: a topic's subscriber is not Send, so it lives on that thread, in a slot
//! keyed by the node's address. The session's number (`Welcomed::session`) says which session a
//! slot belongs to; a slot from an earlier session is dropped when the number changes, and its
//! subscriptions are made again on the new one. Nothing is subscribed twice in one session, and
//! nothing is subscribed that a session does not need, so the slots are bounded by the topics
//! the node lists.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Duration;

use atlas_board::{identity, lemnos};
use atlas_types::{Identity, LemnosState, LinkId};
use orion::Orion;
use orion::topics::{FieldId, FieldType, Next, Status, Subscriber};
use orion_msg_topics::Declare;
use orion_wire::Decode;

use crate::device::{RESOURCE_KIND, RESOURCE_PREFIX, RawDevice, RawField, RawTopic, TopicLive};
use crate::logs::{LOG_CAPACITY, LOG_TOPIC};
use crate::value::{ReadingKind, Scalar, decode};

thread_local! {
    /// The live state of each node this thread serves, by the node's address.
    static LIVE: RefCell<BTreeMap<String, Live>> = const { RefCell::new(BTreeMap::new()) };
}

/// What one read of a node found, as plain data the caller takes off the thread.
#[derive(Debug)]
pub(crate) struct Read {
    /// Who the node is.
    pub identity: Identity,
    /// The device resources it lists, with their topics read.
    pub devices: Vec<RawDevice>,
    /// The log lines since the last read, oldest first, with their times.
    pub logs: Vec<(u64, String)>,
    /// How many log lines were lost to the subscriber since the last read.
    pub logs_missed: u64,
}

/// The subscriptions and identity one session holds.
#[derive(Default)]
struct Live {
    session: Option<u32>,
    identity: Option<Identity>,
    topics: BTreeMap<String, Subscriber>,
    logs: Option<Subscriber>,
}

/// Reads the node at `key` (its address) on `orion`'s session. The outer `Err` is the session
/// (the caller applies the failure rule); the inner `Err` is a reading this crate cannot make
/// from what the node said (the session is up).
pub(crate) fn read(
    orion: &Orion,
    key: &str,
    link: &LinkId,
    address: &str,
) -> Result<Result<Read, String>, orion::Error> {
    let session = orion.welcomed().session;
    LIVE.with(|cell| {
        let mut slots = cell.borrow_mut();
        let live = slots.entry(key.to_owned()).or_default();
        if live.session != Some(session) {
            *live = Live {
                session: Some(session),
                ..Live::default()
            };
        }
        live.read(orion, link, address)
    })
}

impl Live {
    fn read(
        &mut self,
        orion: &Orion,
        link: &LinkId,
        address: &str,
    ) -> Result<Result<Read, String>, orion::Error> {
        orion.pump(Duration::ZERO)?;
        if self.identity.is_none() {
            let node = orion.node()?;
            let lemnos = if lemnos::published_by(&node) {
                lemnos::probe(orion, &node)?
            } else {
                LemnosState::Absent
            };
            match identity::identity(&node, &lemnos, link, address) {
                Ok(identity) => self.identity = Some(identity),
                Err(error) => return Ok(Err(format!("the node's identity: {error}"))),
            }
        }
        let Some(identity) = self.identity.clone() else {
            return Ok(Err("the node's identity is not known".into()));
        };

        let mut devices = Vec::new();
        for resource in orion.resources(RESOURCE_PREFIX)? {
            if resource.kind != RESOURCE_KIND {
                continue;
            }
            let mut topics = Vec::new();
            for name in &resource.topics {
                if !self.topics.contains_key(name) {
                    let subscriber = orion.subscribe(name)?;
                    self.topics.insert(name.clone(), subscriber);
                }
                if let Some(subscriber) = self.topics.get_mut(name) {
                    topics.push(read_topic(name, subscriber));
                }
            }
            devices.push(RawDevice {
                resource: resource.name,
                state: resource.state,
                labels: resource.labels,
                topics,
            });
        }

        if self.logs.is_none() {
            self.logs = Some(orion.subscribe(LOG_TOPIC)?);
        }
        let (logs, logs_missed) = match self.logs.as_mut() {
            Some(subscriber) => read_logs(subscriber),
            None => (Vec::new(), 0),
        };
        Ok(Ok(Read {
            identity,
            devices,
            logs,
            logs_missed,
        }))
    }
}

/// One topic as it is now: its publisher's state, and each field's newest value.
fn read_topic(name: &str, subscriber: &mut Subscriber) -> RawTopic {
    let live = match subscriber.status() {
        Status::Waiting => TopicLive::Waiting,
        Status::Live => TopicLive::Live,
        // A topic its publisher withdrew reads as gone too: the topic is not there, which is not a
        // fault of the node.
        Status::PublisherGone => TopicLive::Gone,
    };
    // The declaration gives each field's name, type and unit. It is there once the topic is.
    let declared: Vec<(String, FieldType, String)> = subscriber.with_declaration(|declaration| {
        declaration
            .and_then(|bytes| Declare::decode(bytes).ok())
            .map(|declare| {
                declare
                    .fields
                    .iter()
                    .flatten()
                    .map(|field| (field.name.to_owned(), field.ty, field.unit.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    });
    let fields = declared
        .into_iter()
        .enumerate()
        .map(|(index, (name, ty, unit))| {
            let kind = ReadingKind::of(&ty);
            let value = subscriber
                .latest(FieldId(index as u16))
                .and_then(|sample| decode(kind, sample.payload));
            RawField {
                name,
                kind,
                unit,
                value,
            }
        })
        .collect();
    RawTopic {
        name: name.to_owned(),
        live,
        fields,
    }
}

/// The log lines the subscriber has not returned yet, at most one ring's worth per read.
fn read_logs(subscriber: &mut Subscriber) -> (Vec<(u64, String)>, u64) {
    let mut lines = Vec::new();
    let mut missed = 0_u64;
    for _ in 0..LOG_CAPACITY {
        match subscriber.next() {
            Next::Sample(sample) => {
                if sample.field == FieldId(0)
                    && let Some(Scalar::Text(text)) = decode(ReadingKind::Text, sample.payload)
                {
                    lines.push((sample.time.as_nanos(), text));
                }
            }
            Next::Skipped(lost) => missed = missed.saturating_add(lost),
            Next::Empty => break,
        }
    }
    (lines, missed)
}
