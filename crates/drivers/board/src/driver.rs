//! The board driver: the one `Driver` that finds Orion nodes and identifies them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, Serves};
use atlas_session::{Handle, Pause, Presence, RealPause, Seed, Sessions};
use atlas_types::{Candidate, CapabilitySet, Family, Identity, LemnosState, Link, LinkKind};
use orion::node::Node;
use tokio::sync::watch;

use crate::discovery;
use crate::identity;
use crate::lemnos;

/// The label this driver is listed under. It serves every family, so the label is only the
/// scan's name for it. No device's key carries it: a node's key is its own kind (see
/// [`atlas_types::DeviceKey`]).
pub const FAMILY: &str = "orion";
/// How long `identify` waits for a node that was seen but has just dropped out.
const IDENTIFY_WAIT: Duration = Duration::from_secs(2);
/// How long a node's identity is held through a republish of its hello facts: a board key that
/// goes missing is kept for this long after the last read that had it. A bridge that republishes
/// its session (a fact came or went) says every fact again well within this.
pub const BLIP_HOLD: Duration = Duration::from_secs(5);
/// The link kinds this driver discovers nodes on.
pub const LINK_KINDS: [LinkKind; 3] = [
    LinkKind::UsbNetwork,
    LinkKind::Ethernet,
    LinkKind::UsbSerial,
];

/// Finds and identifies Orion nodes at the addresses it was given, on the links they sit on.
///
/// It serves [`Serves::Any`]: a node's family comes from its identity. The connection to each
/// node is a session from [`atlas_session`]: the first scan that names a node starts its
/// monitor. A node that is up is offered by `discover`; one that is gone is not, until its
/// monitor finds it again.
#[derive(Debug)]
pub struct BoardDriver {
    manifest: DriverManifest,
    seeds: Vec<Seed>,
    sessions: Sessions,
    /// Nodes that have published Lemnos facts or offers. Once one has, it is expected to answer
    /// for Lemnos from then on, even while its facts are gone (after its own restart).
    expects_lemnos: Mutex<BTreeSet<String>>,
    /// The identity each node was last read with, by address, and when it was last read with a
    /// board key (see [`BoardDriver::settle`]).
    known: Mutex<BTreeMap<String, Known>>,
}

/// The identity a node was last held to, and when it was last read with its board key.
#[derive(Debug)]
struct Known {
    identity: Identity,
    seen: Instant,
}

/// What a node answers for its identity: its hello document, what its Lemnos is doing, and
/// whether the node is expected to run Lemnos.
struct Answer {
    node: Node,
    lemnos: LemnosState,
    expected: bool,
}

impl BoardDriver {
    /// A driver for the nodes at `seeds`, waiting between tries for real.
    pub fn new(seeds: Vec<Seed>) -> Self {
        Self::with_pause(seeds, Arc::new(RealPause))
    }

    /// As [`BoardDriver::new`], with the wait between tries chosen by the caller.
    pub fn with_pause(seeds: Vec<Seed>, pause: Arc<dyn Pause>) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Orion node".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: LINK_KINDS.to_vec(),
                priority: 0,
            },
            seeds,
            sessions: Sessions::new(pause),
            expects_lemnos: Mutex::new(BTreeSet::new()),
            known: Mutex::new(BTreeMap::new()),
        }
    }

    /// Stops watching the node at `address` until [`BoardDriver::turn_on`]. This is the manual
    /// off: nothing turns a node off by itself, and a scan does not start it again.
    pub fn turn_off(&self, address: &str) {
        lock(&self.expects_lemnos).remove(address);
        lock(&self.known).remove(address);
        self.sessions.turn_off(address);
    }

    /// Watches the node at `address` again after [`BoardDriver::turn_off`].
    pub fn turn_on(&self, address: &str) {
        self.sessions.turn_on(address);
    }

    fn expects_lemnos(&self, address: &str) -> bool {
        lock(&self.expects_lemnos).contains(address)
    }

    /// Keeps a node's identity from changing across a republish.
    ///
    /// When a fact comes or goes, the bridge publishes its session again: the node's facts and
    /// its Lemnos actions are said again, and for a moment a read finds neither. A read in that
    /// window that lacks the board key the node had keeps the identity it had, for up to
    /// [`BLIP_HOLD`] after the last read that had the key. In that window a failed Lemnos probe
    /// is the blip, not a fault, so the held Lemnos state stays answering. After the hold a missing
    /// board key is the node's identity: a fact that is really gone is reported once the hold runs out.
    fn settle(&self, address: &str, fresh: Identity, now: Instant) -> Identity {
        let mut known = lock(&self.known);
        let held = match known.get(address) {
            Some(last)
                if last.identity.key.is_board()
                    && !fresh.key.is_board()
                    && now.duration_since(last.seen) < BLIP_HOLD =>
            {
                let mut held = last.identity.clone();
                held.lemnos = match (&fresh.lemnos, &last.identity.lemnos) {
                    (LemnosState::NotAnswering { .. }, LemnosState::Answering) => {
                        LemnosState::Answering
                    }
                    _ => fresh.lemnos.clone(),
                };
                Some((held, last.seen))
            }
            _ => None,
        };
        match held {
            Some((held, seen)) => {
                known.insert(
                    address.to_owned(),
                    Known {
                        identity: held.clone(),
                        seen,
                    },
                );
                held
            }
            None => {
                known.insert(
                    address.to_owned(),
                    Known {
                        identity: fresh.clone(),
                        seen: now,
                    },
                );
                fresh
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl Driver for BoardDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    fn serves(&self) -> Serves {
        Serves::Any
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        let mut found = Vec::new();
        for address in discovery::addresses_on(&self.seeds, link) {
            let Some(handle) = self.sessions.handle(&address) else {
                continue;
            };
            if matches!(handle.presence(), Presence::Found(_)) {
                found.push(Candidate {
                    link: link.id.clone(),
                    family: self.manifest.family.clone(),
                    address: address.to_string(),
                });
            }
        }
        Ok(found)
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let handle: Handle = self.sessions.existing(&candidate.address).ok_or_else(|| {
            DriverError::Unsupported(format!(
                "{} is not a node this driver was told about",
                candidate.address
            ))
        })?;
        let mut presence = handle.watch();
        if !wait_found(&mut presence).await {
            let reason = match &*presence.borrow() {
                Presence::Lost(reason) => reason.clone(),
                _ => "not answering yet".into(),
            };
            return Err(unreachable(&candidate.address, &reason));
        }
        let was_expected = self.expects_lemnos(&candidate.address);
        let answer = handle
            .with_client(move |orion| {
                let node = orion.node()?;
                let expected = was_expected || lemnos::published_by(&node);
                let lemnos = if expected {
                    lemnos::probe(orion, &node)?
                } else {
                    LemnosState::Absent
                };
                Ok(Answer {
                    node,
                    lemnos,
                    expected,
                })
            })
            .await
            .map_err(|error| unreachable(&candidate.address, &error.to_string()))?;
        if answer.expected {
            lock(&self.expects_lemnos).insert(candidate.address.clone());
        }
        let fresh = identity::identity(
            &answer.node,
            &answer.lemnos,
            &candidate.link,
            &candidate.address,
        )?;
        Ok(self.settle(&candidate.address, fresh, Instant::now()))
    }

    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        identity::capabilities(device)
    }
}

fn unreachable(address: &str, reason: &str) -> DriverError {
    DriverError::Unreachable(format!("{address}: {reason}"))
}

/// Whether the monitor has found the node within the identify wait.
async fn wait_found(presence: &mut watch::Receiver<Presence>) -> bool {
    let waited = tokio::time::timeout(
        IDENTIFY_WAIT,
        presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await;
    matches!(waited, Ok(Ok(_)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_types::{DeviceKey, DeviceState, LinkId};

    use super::*;

    fn read(key: DeviceKey, lemnos: LemnosState) -> Identity {
        Identity {
            key,
            model: "test-model".into(),
            state: DeviceState::Running,
            lemnos,
            versions: BTreeMap::from([("os".to_owned(), "1.0.0".to_owned())]),
            name: None,
            link: LinkId::new("usbnet:a"),
            address: "x".into(),
            attributes: BTreeMap::new(),
        }
    }

    fn board_read(lemnos: LemnosState) -> Identity {
        read(
            DeviceKey::board("test-family", "0123456789abcdef").unwrap(),
            lemnos,
        )
    }

    fn node_read(lemnos: LemnosState) -> Identity {
        read(DeviceKey::node("ab:cd").unwrap(), lemnos)
    }

    fn driver() -> BoardDriver {
        BoardDriver::with_pause(Vec::new(), Arc::new(atlas_session::RealPause))
    }

    fn not_answering() -> LemnosState {
        LemnosState::NotAnswering {
            reason: "-4: not answering".into(),
        }
    }

    #[test]
    fn a_board_key_is_kept_through_a_republish_and_then_reported_gone() {
        let driver = driver();
        let t0 = Instant::now();
        let first = driver.settle("a", board_read(LemnosState::Answering), t0);
        assert!(first.key.is_board());
        // Inside the hold, a read that lacks the facts is the identity the node had.
        let held = driver.settle(
            "a",
            node_read(LemnosState::Answering),
            t0 + Duration::from_secs(1),
        );
        assert_eq!(held.key, first.key);
        assert_eq!(held.versions, first.versions);
        // Past the hold, the missing key is the node's own.
        let gone = driver.settle("a", node_read(LemnosState::Answering), t0 + BLIP_HOLD);
        assert!(!gone.key.is_board(), "{gone:?}");
    }

    #[test]
    fn a_failed_lemnos_probe_in_the_blip_is_not_a_fault_and_outside_it_is() {
        let driver = driver();
        let t0 = Instant::now();
        driver.settle("a", board_read(LemnosState::Answering), t0);
        // The blip: the facts and the probe are both missing.
        let during = driver.settle("a", node_read(not_answering()), t0 + Duration::from_secs(1));
        assert_eq!(during.lemnos, LemnosState::Answering, "not a fault");
        // The facts are back and the probe answers: the read is the node's own again.
        let after = driver.settle(
            "a",
            board_read(LemnosState::Answering),
            t0 + Duration::from_secs(2),
        );
        assert!(after.key.is_board());
        // Past the hold, the same failed probe is a fault.
        let fault = driver.settle("a", node_read(not_answering()), t0 + BLIP_HOLD * 2);
        assert_eq!(fault.lemnos, not_answering());
    }

    #[test]
    fn a_board_key_whose_probe_fails_with_its_facts_present_is_reported_as_a_fault() {
        let driver = driver();
        let t0 = Instant::now();
        driver.settle("a", board_read(LemnosState::Answering), t0);
        let fault = driver.settle(
            "a",
            board_read(not_answering()),
            t0 + Duration::from_secs(1),
        );
        assert_eq!(fault.lemnos, not_answering());
        assert!(fault.key.is_board());
    }

    #[test]
    fn a_node_that_never_had_a_board_key_is_not_held() {
        let driver = driver();
        let t0 = Instant::now();
        let plain = driver.settle("a", node_read(LemnosState::Absent), t0);
        assert_eq!(plain.lemnos, LemnosState::Absent);
        assert!(!plain.key.is_board());
    }
}
