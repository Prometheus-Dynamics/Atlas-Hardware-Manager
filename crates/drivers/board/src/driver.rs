//! The board driver: the one `Driver` that finds and identifies boards through Orion.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_types::{Candidate, CapabilitySet, Family, Identity, Link};
use tokio::sync::watch;

use crate::identity;
use crate::monitor::{Handle, Monitor, Pause, Presence, RealPause};
use crate::reach::{LINK_KINDS, Seed};

/// The family name the driver is listed under. A board's own family comes from its hello
/// document, so one driver covers every family.
pub const FAMILY: &str = "orion";
/// How long `identify` waits for a node that was seen but has just dropped out.
const IDENTIFY_WAIT: Duration = Duration::from_secs(2);

/// Finds and identifies Orion nodes at the addresses it was given, on the links they sit on.
///
/// The first scan that names a node starts its monitor. A node that is up is offered by
/// `discover`; one that is gone is not, until its monitor finds it again.
#[derive(Debug)]
pub struct BoardDriver {
    manifest: DriverManifest,
    seeds: Vec<Seed>,
    pause: Arc<dyn Pause>,
    monitors: Mutex<BTreeMap<String, Monitor>>,
    off: Mutex<BTreeSet<String>>,
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
            pause,
            monitors: Mutex::new(BTreeMap::new()),
            off: Mutex::new(BTreeSet::new()),
        }
    }

    /// Stops watching the node at `address` until [`BoardDriver::turn_on`]. This is the manual
    /// off: nothing turns a node off by itself.
    pub fn turn_off(&self, address: &str) {
        lock(&self.off).insert(address.to_owned());
        lock(&self.monitors).remove(address);
    }

    /// Watches the node at `address` again after [`BoardDriver::turn_off`].
    pub fn turn_on(&self, address: &str) {
        lock(&self.off).remove(address);
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

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        let off = lock(&self.off).clone();
        let mut monitors = lock(&self.monitors);
        let mut found = Vec::new();
        for seed in self.seeds.iter().filter(|seed| seed.is_on(link)) {
            let key = seed.address.to_string();
            if off.contains(&key) {
                continue;
            }
            let monitor = monitors
                .entry(key.clone())
                .or_insert_with(|| Monitor::start(seed.address.clone(), self.pause.clone()));
            if matches!(monitor.handle().presence(), Presence::Found(_)) {
                found.push(Candidate {
                    link: link.id.clone(),
                    family: self.manifest.family.clone(),
                    address: key,
                });
            }
        }
        Ok(found)
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let handle: Handle = lock(&self.monitors)
            .get(&candidate.address)
            .map(Monitor::handle)
            .ok_or_else(|| {
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
            return Err(DriverError::Unreachable(format!(
                "{}: {reason}",
                candidate.address
            )));
        }
        let node = handle.ask().await.map_err(|reason| {
            DriverError::Unreachable(format!("{}: {reason}", candidate.address))
        })?;
        identity::identity(&node, &candidate.link, &candidate.address)
    }

    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        identity::capabilities(device)
    }
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
