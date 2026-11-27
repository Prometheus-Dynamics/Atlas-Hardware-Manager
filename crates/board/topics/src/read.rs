//! Reads a node's devices, descriptions, readings and logs on its live session.
//!
//! This runs only inside [`atlas_session::Handle::with_client`], on the node's monitor thread,
//! which is the one place Orion's client may be used. A subscription is made once for each
//! session and kept: a topic's subscriber is not Send, so it lives on that thread, in a slot
//! keyed by the node's address. The slot names the connection it belongs to (the node's identity
//! as its monitor saw it on connect); a slot from an earlier connection is dropped, and its
//! subscriptions are made again on the new one. The comm core's session number is not used: a
//! restarted node numbers its sessions again from the start. Nothing is subscribed twice in one
//! session, and nothing is subscribed that a session does not need, so the slots are bounded by
//! the topics the node lists.
//!
//! A device's description is asked for once when this connection first sees the device, and again
//! only when its description generation moves (see [`crate::describe`]). The device's topics are
//! subscribed before the ask, so a change made while asking is seen by the next read.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use atlas_board::{identity, lemnos};
use atlas_session::Backoff;
use atlas_session::lemnos as session_lemnos;
use atlas_types::{Identity, LemnosState, LinkId};
use orion::Orion;
use orion::provider::ResourceState;
use orion::topics::{FieldId, FieldType, Next, Status, Subscriber};
use orion::types::NodeName;
use orion_msg_topics::Declare;
use orion_wire::Decode;

use crate::describe::{Asked, Refresh, fetch, is_due};
use crate::description::DescriptionState;
use crate::device::{
    RESOURCE_KIND, RESOURCE_PREFIX, RawDevice, RawField, RawTopic, TopicLive, generation_of,
};
use crate::logs::{LOG_CAPACITY, LOG_TOPIC};
use crate::value::{Scalar, decode};

/// The refresh in progress for a hardware reader: its answers are kept here, not on the
/// connection, so they outlive a session that fails. Only the refresh whose number a read was
/// given is written to.
pub(crate) type RefreshCell = Mutex<Option<Refresh>>;

thread_local! {
    /// The live state of each node this thread serves, by the node's address.
    static LIVE: RefCell<BTreeMap<String, Live>> = const { RefCell::new(BTreeMap::new()) };
}

/// What one read of a node found, as plain data the caller takes off the thread.
#[derive(Debug)]
pub(crate) struct Read {
    /// Who the node is.
    pub identity: Identity,
    /// The device resources it lists, with their topics read and their descriptions.
    pub devices: Vec<RawDevice>,
    /// The log lines since the last read, oldest first, with their times.
    pub logs: Vec<(u64, String)>,
    /// How many log lines were lost to the subscriber since the last read.
    pub logs_missed: u64,
}

/// The subscriptions, identity and descriptions one session holds.
#[derive(Default)]
struct Live {
    /// Which connection this state belongs to: see [`read`].
    session: Option<String>,
    /// The monitor's number for that connection: the key of its describe answers (see
    /// `atlas_session::lemnos`). Zero when none.
    number: u64,
    identity: Option<Identity>,
    topics: BTreeMap<String, Subscriber>,
    logs: Option<Subscriber>,
    /// The descriptions asked for on this connection, by device resource name. Only a complete description
    /// or a device that is off is kept here; a refusal of a device that is on is in `retry`.
    asked: BTreeMap<String, Asked>,
    /// The devices whose description was refused while on, with the backoff for the next ask.
    retry: BTreeMap<String, Retry>,
    /// Whether the Lemnos probe's answer is final for this connection (see `lemnos::Probe`). Until it is,
    /// the probe is asked again, on `probe_after`.
    lemnos_settled: bool,
    /// The backoff between probes while the answer is not final; restarted by a final answer.
    probe_backoff: Backoff,
    /// When the next probe may be asked, while the answer is not final.
    probe_after: Option<Instant>,
}

/// A refused description of a device that is on: when it may be asked again, the backoff for that ask, the
/// start of the stretch of refusals, and the board's words for the last one.
#[derive(Clone, Debug)]
struct Retry {
    after: Instant,
    backoff: Backoff,
    since: Instant,
    reason: String,
}

/// Reads the node at `address` on `orion`'s connection, which is the connection `session` names.
///
/// The state kept for the node is for one connection. `session` is the node's identity as the
/// monitor saw it on connect (its key fingerprint and clock at that connect): every connection
/// has its own, where the comm core's session number is reused by a restarted node and cannot
/// tell two connections apart. A state from another connection is dropped, and its
/// subscriptions and descriptions are made again on this one. `refresh`, when a refresh is in
/// progress, is its cell and its number: a device that has answered that refresh is held, and
/// every other device is asked.
///
/// The outer `Err` is the session (the caller applies the failure rule); the inner `Err` is a
/// reading this crate cannot make from what the node said (the session is up).
pub(crate) fn read(
    orion: &Orion,
    address: &str,
    session: &str,
    number: u64,
    link: &LinkId,
    refresh: Option<(&RefreshCell, u64)>,
) -> Result<Result<Read, String>, orion::Error> {
    LIVE.with(|cell| {
        let mut slots = cell.borrow_mut();
        let live = slots.entry(address.to_owned()).or_default();
        if live.session.as_deref() != Some(session) {
            // The connection this state belonged to is gone: its describe answers go with it.
            session_lemnos::forget_session(live.number);
            *live = Live {
                session: Some(session.to_owned()),
                number,
                ..Live::default()
            };
        }
        live.read(orion, link, address, number, refresh)
    })
}

impl Live {
    fn read(
        &mut self,
        orion: &Orion,
        link: &LinkId,
        address: &str,
        number: u64,
        refresh: Option<(&RefreshCell, u64)>,
    ) -> Result<Result<Read, String>, orion::Error> {
        orion.pump(Duration::ZERO)?;
        // The probe is asked until its answer is final: a transient one (the device is not answering yet)
        // is not kept, and the next read asks again, on the backoff.
        let due = self.probe_after.is_none_or(|after| Instant::now() >= after);
        if self.identity.is_none() || (!self.lemnos_settled && due) {
            let node = orion.node()?;
            let probe = if lemnos::published_by(&node) {
                lemnos::probe(orion, number)?
            } else {
                lemnos::Probe {
                    state: LemnosState::Absent,
                    settled: true,
                }
            };
            match identity::identity(&node, &probe.state, link, address) {
                Ok(identity) => self.identity = Some(identity),
                Err(error) => return Ok(Err(format!("the node's identity: {error}"))),
            }
            self.lemnos_settled = probe.settled;
            if probe.settled {
                self.probe_backoff.reset();
                self.probe_after = None;
            } else {
                let delay = self.probe_backoff.next_delay();
                self.probe_after = Some(Instant::now() + delay);
            }
        }
        let Some(identity) = self.identity.clone() else {
            return Ok(Err("the node's identity is not known".into()));
        };

        let mut devices = Vec::new();
        let mut listed = BTreeSet::new();
        for resource in orion.resources(RESOURCE_PREFIX)? {
            if resource.kind != RESOURCE_KIND {
                continue;
            }
            let mut topics = Vec::new();
            // TODO(user): a topic subscribed in this pass is read in the same pass, before its
            // first sample has arrived. So for about 1 s after a board opens (until the app's next
            // read, READ_EVERY in apps/desktop/src-tauri/src/board.rs), its page says "This board
            // reports no readings" (measured 2026-11-27: tiles at 1.4-1.5 s after open, the first
            // read at 0.3 s showed none). Proposed: read again early after a pass that made new
            // subscriptions, once their first sample is in, not a full READ_EVERY later. A product
            // change, held for the user.
            for name in &resource.topics {
                if !self.topics.contains_key(name) {
                    let subscriber = orion.subscribe(name)?;
                    self.topics.insert(name.clone(), subscriber);
                }
                if let Some(subscriber) = self.topics.get_mut(name) {
                    topics.push(read_topic(name, subscriber));
                }
            }
            // The topics are subscribed before the description is asked for, so a change made
            // during the ask is seen by the next read.
            // LOCK: Lemnos 214c0ca7 publishes the per-device generation topic (`lemnos.<device>.description`, field
            // `generation`: publish.rs:200 and :242), and it is followed here. The device-set topic (`lemnos.devices`) is not
            // subscribed: the resource list is read on every snapshot, so an appearing or leaving
            // device is seen there, and its description is asked once when it first shows.
            let description = self.description_of(
                orion,
                number,
                &resource.name,
                generation_of(&topics),
                resource.state == ResourceState::Pending,
                refresh,
            )?;
            listed.insert(resource.name.clone());
            devices.push(RawDevice {
                resource: resource.name,
                state: resource.state,
                topics,
                description,
            });
        }
        self.asked.retain(|name, _| listed.contains(name));
        self.retry.retain(|name, _| listed.contains(name));
        session_lemnos::retain_described(number, &listed.iter().cloned().collect::<Vec<_>>());
        if let Some((cell, id)) = refresh
            && let Some(current) = lock(cell).as_mut().filter(|current| current.id == id)
        {
            current.answered.retain(|name, _| listed.contains(name));
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

    /// The description of one device now: held when it is not due, else asked for. During a
    /// refresh the held answer is the refresh's own, so an answer it has received is not asked
    /// again, whatever the connection did since. A device this session has already been described
    /// for (by the Lemnos resolver, at connect) is held as that answer, not asked again. A node with
    /// no name yet is not asked (its state stays pending until it has one).
    fn description_of(
        &mut self,
        orion: &Orion,
        number: u64,
        device: &str,
        generation: Option<u64>,
        off: bool,
        refresh: Option<(&RefreshCell, u64)>,
    ) -> Result<DescriptionState, orion::Error> {
        // A refused description of a device that is on is asked again by the reader, on its backoff (a
        // refresh asks now). Until then it is shown as not answering yet.
        if refresh.is_none()
            && let Some(retry) = self.retry.get(device)
            && Instant::now() < retry.after
        {
            return Ok(DescriptionState::NotYet {
                since: retry.since,
                reason: retry.reason.clone(),
            });
        }
        let mut held = match refresh {
            Some((cell, id)) => {
                // The refresh's answer is also this connection's: once the refresh is over, the
                // connection follows it, and does not ask again.
                let answer = refreshed(cell, id, device);
                if let Some(asked) = &answer {
                    self.asked.insert(device.to_owned(), asked.clone());
                }
                answer
            }
            None => self.asked.get(device).cloned(),
        };
        // Outside a refresh, a device this session has already been described for is held as that
        // answer. A refresh asks every device that has not answered it, so its answer is not shared.
        if held.is_none()
            && refresh.is_none()
            && let Some(answer) = session_lemnos::held_description(number, device)
        {
            // A description carries its own generation. A refusal does not: it was asked at a generation
            // this reader never saw, so it is not taken as current. It is due at the next sample.
            let mut adopted = asked_from(&answer, generation);
            if !matches!(adopted.state, DescriptionState::Described(_)) {
                adopted.generation = None;
            }
            held = Some(adopted);
        }
        if !is_due(held.as_ref(), generation) {
            return Ok(held.map_or(DescriptionState::Pending, |asked| asked.state));
        }
        // A node with no name yet is not asked: its state stays pending until it has one.
        if orion.node()?.name.parse::<NodeName>().is_err() {
            return Ok(DescriptionState::Pending);
        }
        // A held answer that is due has a generation that moved (or an answer that is not the
        // current one), and a refresh asks each device it has not heard from: either way the device
        // is asked again, once, and the session keeps the new answer.
        if held.is_some() || refresh.is_some() {
            session_lemnos::forget_description(number, device);
        }
        let answer = fetch(orion, number, device)?;
        let asked = match &answer {
            // A device that is off is not "not yet": its refusal is its answer, held as before.
            Err(_) if off => asked_from(&answer, generation),
            Err(reason) => {
                let (since, mut backoff) = match self.retry.remove(device) {
                    Some(retry) => (retry.since, retry.backoff),
                    None => (Instant::now(), Backoff::default()),
                };
                let after = Instant::now() + backoff.next_delay();
                self.retry.insert(
                    device.to_owned(),
                    Retry {
                        after,
                        backoff,
                        since,
                        reason: reason.to_string(),
                    },
                );
                self.asked.remove(device);
                Asked {
                    state: DescriptionState::NotYet {
                        since,
                        reason: reason.to_string(),
                    },
                    generation: None,
                }
            }
            Ok(_) => {
                self.retry.remove(device);
                asked_from(&answer, generation)
            }
        };
        if let Some((cell, id)) = refresh {
            answer_refresh(cell, id, device, &asked);
        }
        let state = asked.state.clone();
        if !matches!(state, DescriptionState::NotYet { .. }) {
            self.asked.insert(device.to_owned(), asked);
        }
        Ok(state)
    }
}

/// What a describe answer means as a held state: the description it carries, or its refusal, and the
/// generation it answers (the description's own, or for a refusal, the topic's generation now).
fn asked_from(answer: &session_lemnos::Answer, generation: Option<u64>) -> Asked {
    let (state, answers) = match answer {
        Ok(bytes) => match crate::description::parse_bytes(bytes) {
            Ok(description) => {
                let answers = description.device.generation;
                (DescriptionState::Described(Box::new(description)), answers)
            }
            Err(refusal) => (DescriptionState::Unreadable(refusal), generation),
        },
        Err(reason) => (
            DescriptionState::NotAvailable(reason.to_string()),
            generation,
        ),
    };
    Asked {
        state,
        generation: answers,
    }
}

fn lock(cell: &RefreshCell) -> std::sync::MutexGuard<'_, Option<Refresh>> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The answer a device has given refresh `id`, if it has given one.
fn refreshed(cell: &RefreshCell, id: u64, device: &str) -> Option<Asked> {
    lock(cell)
        .as_ref()
        .filter(|current| current.id == id)
        .and_then(|current| current.answered.get(device).cloned())
}

/// Keeps device `device`'s answer for refresh `id`, if that refresh is still the one in progress.
fn answer_refresh(cell: &RefreshCell, id: u64, device: &str, asked: &Asked) {
    if let Some(current) = lock(cell).as_mut().filter(|current| current.id == id) {
        current.answered.insert(device.to_owned(), asked.clone());
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
    // The declaration gives each field's name and type. It is there once the topic is.
    let declared: Vec<(String, FieldType)> = subscriber.with_declaration(|declaration| {
        declaration
            .and_then(|bytes| Declare::decode(bytes).ok())
            .map(|declare| {
                declare
                    .fields
                    .iter()
                    .flatten()
                    .map(|field| (field.name.to_owned(), field.ty))
                    .collect()
            })
            .unwrap_or_default()
    });
    let fields = declared
        .into_iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            let value = subscriber
                .latest(FieldId(index as u16))
                .and_then(|sample| decode(&ty, sample.payload));
            RawField { name, value }
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
                    && let Some(Scalar::Text(text)) = decode(&FieldType::Text, sample.payload)
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
