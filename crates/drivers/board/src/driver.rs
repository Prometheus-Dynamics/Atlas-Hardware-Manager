//! The board driver: the one `Driver` that finds Orion nodes and identifies them.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

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
        }
    }

    /// Stops watching the node at `address` until [`BoardDriver::turn_on`]. This is the manual
    /// off: nothing turns a node off by itself, and a scan does not start it again.
    pub fn turn_off(&self, address: &str) {
        lock(&self.expects_lemnos).remove(address);
        self.sessions.turn_off(address);
    }

    /// Watches the node at `address` again after [`BoardDriver::turn_off`].
    pub fn turn_on(&self, address: &str) {
        self.sessions.turn_on(address);
    }

    fn expects_lemnos(&self, address: &str) -> bool {
        lock(&self.expects_lemnos).contains(address)
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
        identity::identity(
            &answer.node,
            &answer.lemnos,
            &candidate.link,
            &candidate.address,
        )
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
