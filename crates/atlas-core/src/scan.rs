use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use atlas_driver::{Candidate, DeviceKey, Driver, Identity, Link, LinkKind};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::atlas::{Inner, LiveDevice};
use crate::inventory::{Upsert, link_rank};
use crate::time::now_ms;
use crate::{DeviceRecord, Event};

/// What one scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub online: Vec<DeviceKey>,
    pub went_offline: Vec<DeviceKey>,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
}

/// A candidate waiting to be identified, and how it was reached.
struct Pending {
    driver: Arc<dyn Driver>,
    candidate: Candidate,
    link_kind: LinkKind,
}

struct Scan<'a> {
    inner: &'a Inner,
    /// Best link rank per device seen so far in this scan.
    best: BTreeMap<DeviceKey, u8>,
    warnings: Vec<String>,
}

impl Scan<'_> {
    fn warn(&mut self, message: String) {
        self.inner.events.emit(Event::ScanWarning {
            message: message.clone(),
        });
        self.warnings.push(message);
    }

    /// Records a sighting unless this device was already seen on a better
    /// link in this scan. Returns whether the sighting was kept.
    fn accept(&mut self, pending: &Pending, identity: Identity) -> bool {
        let rank = link_rank(&pending.link_kind);
        if self
            .best
            .get(&identity.key)
            .is_some_and(|best| *best <= rank)
        {
            return false;
        }
        self.best.insert(identity.key.clone(), rank);

        let capabilities = pending.driver.capabilities(&identity);
        let kinds = capabilities.kinds_for(identity.mode);
        let key = identity.key.clone();
        let (outcome, record): (Upsert, DeviceRecord) = {
            let mut state = self.inner.state();
            state.live.insert(
                key,
                LiveDevice {
                    driver: pending.driver.clone(),
                    candidate: pending.candidate.clone(),
                    capabilities,
                },
            );
            state
                .inventory
                .upsert(identity, pending.link_kind.clone(), kinds, now_ms())
        };
        if outcome != Upsert::Unchanged {
            self.inner.events.emit(Event::DeviceSeen {
                record: Box::new(record),
                new: outcome == Upsert::New,
            });
        }
        true
    }

    async fn discover(&mut self, links: Vec<Link>) -> Vec<Pending> {
        let mut tasks = JoinSet::new();
        for link in links {
            for driver in self.inner.registry.for_link(&link.kind) {
                let link = link.clone();
                tasks.spawn(async move {
                    let result = driver.discover(&link).await;
                    (driver, link, result)
                });
            }
        }

        let mut pending = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((driver, link, Ok(candidates))) => {
                    pending.extend(candidates.into_iter().map(|candidate| Pending {
                        driver: driver.clone(),
                        candidate,
                        link_kind: link.kind.clone(),
                    }));
                }
                Ok((driver, link, Err(error))) => self.warn(format!(
                    "{} could not search {}: {error}",
                    driver.manifest().name,
                    link.label
                )),
                Err(error) => self.warn(format!("A discovery task stopped: {error}")),
            }
        }
        pending
    }

    /// Identifies every pending candidate in parallel, publishing each device
    /// as it answers. Returns the accepted devices for the gateway pass.
    async fn identify(&mut self, frontier: Vec<Pending>) -> Vec<(Arc<dyn Driver>, Identity)> {
        let timeout = self.inner.options.identify_timeout;
        let mut tasks = JoinSet::new();
        for pending in frontier {
            tasks.spawn(async move {
                let result =
                    tokio::time::timeout(timeout, pending.driver.identify(&pending.candidate))
                        .await;
                (pending, result)
            });
        }

        let mut accepted = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((pending, Ok(Ok(identity)))) => {
                    let driver = pending.driver.clone();
                    if self.accept(&pending, identity.clone()) {
                        accepted.push((driver, identity));
                    }
                }
                Ok((pending, Ok(Err(error)))) => self.warn(format!(
                    "{} at {} did not identify: {error}",
                    pending.driver.manifest().name,
                    pending.candidate.address
                )),
                Ok((pending, Err(_elapsed))) => self.warn(format!(
                    "{} at {} did not answer within {} ms",
                    pending.driver.manifest().name,
                    pending.candidate.address,
                    timeout.as_millis()
                )),
                Err(error) => self.warn(format!("An identify task stopped: {error}")),
            }
        }
        accepted
    }

    /// Asks each accepted device for devices behind it.
    async fn children(&mut self, parents: Vec<(Arc<dyn Driver>, Identity)>) -> Vec<Pending> {
        // A device accepted on a worse link and then a better one is asked once.
        let parents: BTreeMap<DeviceKey, (Arc<dyn Driver>, Identity)> = parents
            .into_iter()
            .map(|(driver, identity)| (identity.key.clone(), (driver, identity)))
            .collect();
        let mut tasks = JoinSet::new();
        for (driver, identity) in parents.into_values() {
            tasks.spawn(async move {
                let result = driver.children(&identity).await;
                (identity.key, result)
            });
        }

        let mut pending = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((parent, Ok(candidates))) => {
                    for candidate in candidates {
                        match self.inner.registry.get(&candidate.family) {
                            Some(driver) => pending.push(Pending {
                                driver: driver.clone(),
                                candidate,
                                link_kind: LinkKind::Gateway { via: parent.clone() },
                            }),
                            None => self.warn(format!(
                                "{parent} reports a {} device, but no driver for that family is installed",
                                candidate.family
                            )),
                        }
                    }
                }
                Ok((parent, Err(error))) => {
                    self.warn(format!(
                        "{parent} could not list devices behind it: {error}"
                    ));
                }
                Err(error) => self.warn(format!("A gateway task stopped: {error}")),
            }
        }
        pending
    }
}

pub(crate) async fn run(inner: &Inner) -> ScanReport {
    let started = Instant::now();
    inner.events.emit(Event::ScanStarted);

    let links = futures::future::join_all(inner.link_sources.iter().map(|source| source.links()))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    let mut scan = Scan {
        inner,
        best: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let mut frontier = scan.discover(links).await;
    for depth in 0..=inner.options.max_gateway_depth {
        if frontier.is_empty() {
            break;
        }
        let accepted = scan.identify(frontier).await;
        frontier = if depth < inner.options.max_gateway_depth {
            scan.children(accepted).await
        } else {
            Vec::new()
        };
    }

    let seen: BTreeSet<DeviceKey> = scan.best.keys().cloned().collect();
    let went_offline = {
        let mut state = inner.state();
        let gone = state.inventory.mark_offline_except(&seen);
        for key in &gone {
            state.live.remove(key);
        }
        gone
    };
    for key in &went_offline {
        inner.events.emit(Event::DeviceOffline { key: key.clone() });
    }
    inner.persist();

    let report = ScanReport {
        online: seen.into_iter().collect(),
        went_offline,
        warnings: scan.warnings,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    inner.events.emit(Event::ScanFinished {
        report: report.clone(),
    });
    report
}
