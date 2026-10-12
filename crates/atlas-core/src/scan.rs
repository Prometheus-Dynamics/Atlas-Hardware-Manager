use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use atlas_driver::{Candidate, DeviceKey, Driver, Identity, Link, LinkKind};

use crate::links::host_ip;
use serde::Serialize;
use tokio::task::JoinSet;

use crate::activity::{self, ActivityEntry, ActivityKind, ActivityLevel};
use crate::atlas::{Inner, LiveDevice};
use crate::inventory::{Upsert, link_rank};
use crate::time::now_ms;
use crate::{DeviceRecord, Event};

/// How long an online device that is still found but doesn't answer stays
/// online. A camera on a busy robot network misses an identify now and then;
/// marking it offline each time makes its views come and go.
const MISSED_ANSWER_GRACE_MS: u64 = 30_000;

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
    /// Best link rank so far this scan, for devices new to this scan (the
    /// ones Atlas already follows change link only as links.rs says).
    best: BTreeMap<DeviceKey, u8>,
    /// Devices not followed when this scan first saw them.
    fresh: BTreeSet<DeviceKey>,
    /// Devices that answered on any link this scan.
    seen: BTreeSet<DeviceKey>,
    warnings: Vec<String>,
    /// Whether any device was new or changed, so robot statuses may too.
    changed: bool,
    /// Candidates found this scan that didn't identify: still there, just
    /// not answering.
    unanswered: BTreeSet<String>,
}

impl Scan<'_> {
    fn warn(&mut self, message: String) {
        self.inner.events.emit(Event::ScanWarning {
            message: message.clone(),
        });
        self.warnings.push(message);
    }

    /// Records a sighting. The device's record follows it if it is the link
    /// the record follows, or takes over as links.rs decides (for a device
    /// new to this scan: the best link of the scan). Any sighting keeps the
    /// device online. Returns whether the record follows this sighting.
    fn accept(&mut self, pending: &Pending, mut identity: Identity) -> bool {
        let rank = link_rank(&pending.link_kind);
        let key = identity.key.clone();
        self.seen.insert(key.clone());
        // The IPs it has on this link: the address reached, and the ones it
        // advertised there.
        let mut ips: Vec<std::net::IpAddr> =
            host_ip(&pending.candidate.address).into_iter().collect();
        if let Some(listed) = identity.attributes.get(atlas_driver::attributes::ADDRESSES) {
            ips.extend(
                listed
                    .split(',')
                    .filter_map(|ip| ip.trim().parse::<std::net::IpAddr>().ok()),
            );
        }
        let merged = {
            let mut state = self.inner.state();
            let current = state
                .live
                .get(&key)
                .map(|live| live.candidate.address.clone());
            if current.is_none() {
                self.fresh.insert(key.clone());
            }
            let links = state.links.entry(key.clone()).or_default();
            links.answered(&pending.candidate.address, &pending.link_kind, ips);
            let follows = if self.fresh.contains(&key) {
                !self.best.get(&key).is_some_and(|best| *best <= rank)
            } else {
                links.takes_over(current.as_deref(), &pending.candidate.address)
            };
            if !follows {
                state.inventory.touch(&key, now_ms());
                return false;
            }
            links.addresses()
        };
        self.best.insert(key.clone(), rank);
        // Every IP it has on any link, in one stable order.
        if !merged.is_empty() {
            identity.attributes.insert(
                atlas_driver::attributes::ADDRESSES.into(),
                merged
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }

        let capabilities = self.inner.capabilities(pending.driver.as_ref(), &identity);
        let kinds = capabilities.kinds_for(identity.mode);
        let key = identity.key.clone();
        let (outcome, record, previous): (Upsert, DeviceRecord, Option<DeviceRecord>) = {
            let mut state = self.inner.state();
            let previous = state.inventory.get(&key).cloned();
            state.live.insert(
                key,
                LiveDevice {
                    driver: pending.driver.clone(),
                    candidate: pending.candidate.clone(),
                    capabilities,
                },
            );
            let (outcome, record) =
                state
                    .inventory
                    .upsert(identity, pending.link_kind.clone(), kinds, now_ms());
            (outcome, record, previous)
        };
        if outcome != Upsert::Unchanged {
            self.changed = true;
            self.inner
                .record_activity(activity::for_sighting(previous.as_ref(), &record));
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
                Ok((pending, Ok(Err(error)))) => {
                    self.unanswered.insert(pending.candidate.address.clone());
                    self.warn(format!(
                        "{} at {} did not identify: {error}",
                        pending.driver.manifest().name,
                        pending.candidate.address
                    ))
                }
                Ok((pending, Err(_elapsed))) => {
                    self.unanswered.insert(pending.candidate.address.clone());
                    self.warn(format!(
                        "{} at {} did not answer within {} ms",
                        pending.driver.manifest().name,
                        pending.candidate.address,
                        timeout.as_millis()
                    ))
                }
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
    let _one_at_a_time = inner.scan_lock.lock().await;
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
        fresh: BTreeSet::new(),
        seen: BTreeSet::new(),
        warnings: Vec::new(),
        changed: false,
        unanswered: BTreeSet::new(),
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

    let seen: BTreeSet<DeviceKey> = scan.seen.clone();
    let went_offline = {
        let mut state = inner.state();
        for links in state.links.values_mut() {
            links.end_scan();
        }
        // A device found again at its address that just didn't answer in
        // time stays online for a while; one that's gone goes offline now.
        let now = now_ms();
        let mut keep = seen.clone();
        keep.extend(
            state
                .live
                .iter()
                .filter(|(key, live)| {
                    !seen.contains(*key) && scan.unanswered.contains(&live.candidate.address)
                })
                .filter(|(key, _)| {
                    state.inventory.get(key).is_some_and(|record| {
                        now.saturating_sub(record.last_seen_ms) < MISSED_ANSWER_GRACE_MS
                    })
                })
                .map(|(key, _)| key.clone()),
        );
        let gone = state.inventory.mark_offline_except(&keep);
        for key in &gone {
            state.live.remove(key);
        }
        gone
    };
    let offline_entries = {
        let state = inner.state();
        went_offline
            .iter()
            .filter_map(|key| state.inventory.get(key))
            .map(|record| {
                ActivityEntry::about(
                    record,
                    ActivityKind::DeviceOffline,
                    ActivityLevel::Warning,
                    format!("{} went offline", record.display_name()),
                )
            })
            .collect()
    };
    for key in &went_offline {
        inner.events.emit(Event::DeviceOffline { key: key.clone() });
    }
    inner.record_activity(offline_entries);
    // Only now are records that left this scan offline, so a board that
    // went from recovery to running can take its old record's place.
    if crate::lineage::fold_recovery_records(inner, &seen) {
        scan.changed = true;
    }
    if (scan.changed || !went_offline.is_empty()) && !inner.state().robots.is_empty() {
        inner.events.emit(Event::RobotsChanged);
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
