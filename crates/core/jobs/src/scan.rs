//! The background scan: every registered driver, on every link it handles.
//!
//! Each discovery and each identify is its own task with its own timeout. The
//! loop only decides what is due and starts it, so nothing waits on a slow
//! device. Two kinds of answer:
//! - a failure that can change on its own (absent device, timeout, I/O) backs
//!   off, capped, and is retried for as long as it takes;
//! - a settled answer (the driver says the device is incompatible or
//!   unsupported as it is now) is recorded and reported once. It is asked again
//!   only when the device disappears and returns, or when the user asks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use atlas_driver::{Driver, DriverRegistry};
use atlas_types::{Candidate, Family, Link, LinkId};
use tokio::time::{self, Instant};

use crate::call::guarded;
use crate::{Backoff, Event, Events};

/// The links present right now. Called every tick, so it must be a quick
/// snapshot: the host keeps the list current, the scan never waits for it.
pub trait LinkSource: Send + Sync {
    fn links(&self) -> Vec<Link>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanConfig {
    /// How often the loop looks for due work.
    pub tick: Duration,
    /// How long a successful discovery waits before the next one.
    pub interval: Duration,
    /// Limit on one discovery. A call past it is dropped and retried.
    pub discover_timeout: Duration,
    /// Limit on one identify (and the capabilities read after it).
    pub identify_timeout: Duration,
    /// Wait after a failure that can change on its own, doubling up to the cap.
    pub backoff: Backoff,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_millis(250),
            interval: Duration::from_secs(2),
            discover_timeout: Duration::from_secs(10),
            identify_timeout: Duration::from_secs(10),
            backoff: Backoff::default(),
        }
    }
}

/// One thing the scan does on its own schedule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Discover {
        family: Family,
        link: LinkId,
    },
    Identify {
        family: Family,
        link: LinkId,
        address: String,
    },
}

impl Key {
    fn family(&self) -> &Family {
        match self {
            Self::Discover { family, .. } | Self::Identify { family, .. } => family,
        }
    }

    fn link(&self) -> &LinkId {
        match self {
            Self::Discover { link, .. } | Self::Identify { link, .. } => link,
        }
    }
}

/// Whether `key` is an identify of one of `addresses` on this family and link.
fn identifies_one_of(
    key: &Key,
    family: &Family,
    link: &LinkId,
    addresses: &BTreeSet<String>,
) -> bool {
    matches!(key, Key::Identify { family: f, link: l, address }
        if f == family && l == link && addresses.contains(address))
}

#[derive(Clone, Copy, Debug)]
struct Schedule {
    failures: u32,
    next_at: Instant,
}

#[derive(Debug, Default)]
struct Board {
    /// Calls in flight. A key is claimed while its call runs.
    busy: BTreeSet<Key>,
    schedule: BTreeMap<Key, Schedule>,
    /// Settled answers, with the error text. Not called again until re-asked.
    settled: BTreeMap<Key, String>,
    /// The addresses each family's discovery listed on each link last time.
    listed: BTreeMap<(Family, LinkId), BTreeSet<String>>,
}

#[derive(Debug, Default)]
struct Control {
    stopped: bool,
    all_paused: bool,
    paused: BTreeSet<Family>,
}

#[derive(Debug)]
struct Shared {
    board: Mutex<Board>,
    control: Mutex<Control>,
    events: Events,
    config: ScanConfig,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl Shared {
    /// Takes the key if nothing is running for it, it is due and it is not
    /// settled. The claim is released when dropped, even if its task is aborted.
    fn claim(self: &Arc<Self>, key: Key) -> Option<Claim> {
        let now = Instant::now();
        let mut board = lock(&self.board);
        if board.busy.contains(&key)
            || board.settled.contains_key(&key)
            || board.schedule.get(&key).is_some_and(|s| s.next_at > now)
        {
            return None;
        }
        board.busy.insert(key.clone());
        Some(Claim {
            shared: Arc::clone(self),
            key,
        })
    }

    /// Records a success and schedules the next call. Returns whether the key
    /// had been failing.
    fn succeeded(&self, key: &Key, interval: Duration) -> bool {
        let mut board = lock(&self.board);
        let was_failing = board.schedule.get(key).is_some_and(|s| s.failures > 0);
        board.schedule.insert(
            key.clone(),
            Schedule {
                failures: 0,
                next_at: Instant::now() + interval,
            },
        );
        was_failing
    }

    /// Records a failure that may clear on its own and returns the attempt
    /// number and the wait before the next one.
    fn failed(&self, key: &Key) -> (u32, Duration) {
        let mut board = lock(&self.board);
        let failures = board
            .schedule
            .get(key)
            .map_or(0, |s| s.failures)
            .saturating_add(1);
        let delay = self.config.backoff.delay(failures);
        board.schedule.insert(
            key.clone(),
            Schedule {
                failures,
                next_at: Instant::now() + delay,
            },
        );
        (failures, delay)
    }

    /// Records a settled answer. Returns true the first time it is recorded.
    fn settle(&self, key: Key, error: String) -> bool {
        let mut board = lock(&self.board);
        board.schedule.remove(&key);
        board.settled.insert(key, error).is_none()
    }

    /// Notes what a discovery listed. Addresses that appeared or went away
    /// have their schedules and settled answers cleared, so a device that
    /// returns is identified at once and asked again.
    fn reconcile(&self, family: &Family, link: &LinkId, listed: &[String]) {
        let now: BTreeSet<String> = listed.iter().cloned().collect();
        let mut board = lock(&self.board);
        let previous = board
            .listed
            .insert((family.clone(), link.clone()), now.clone())
            .unwrap_or_default();
        let changed: BTreeSet<String> = now.symmetric_difference(&previous).cloned().collect();
        if changed.is_empty() {
            return;
        }
        board
            .schedule
            .retain(|key, _| !identifies_one_of(key, family, link, &changed));
        board
            .settled
            .retain(|key, _| !identifies_one_of(key, family, link, &changed));
    }

    /// Forgets everything about links that are gone, so the tables stay small
    /// and a link that returns is asked again.
    fn prune_links(&self, live: &BTreeSet<LinkId>) {
        let mut board = lock(&self.board);
        board.schedule.retain(|key, _| live.contains(key.link()));
        board.settled.retain(|key, _| live.contains(key.link()));
        board.listed.retain(|(_, link), _| live.contains(link));
    }

    /// Clears settled answers for one family, so the next scan asks again.
    fn ask_again(&self, family: &Family) {
        lock(&self.board)
            .settled
            .retain(|key, _| key.family() != family);
    }
}

/// Holds a claimed key. Dropping it frees the key.
struct Claim {
    shared: Arc<Shared>,
    key: Key,
}

impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.shared.board).busy.remove(&self.key);
    }
}

/// Starts the scan and returns the handle that pauses or stops it. Needs a
/// running tokio runtime. The scan starts at once and runs until stopped.
pub fn start_scan(
    registry: DriverRegistry,
    links: Arc<dyn LinkSource>,
    events: Events,
    config: ScanConfig,
) -> ScanControl {
    let shared = Arc::new(Shared {
        board: Mutex::new(Board::default()),
        control: Mutex::new(Control::default()),
        events,
        config,
    });
    tokio::spawn(run(Arc::clone(&shared), registry, links));
    ScanControl { shared }
}

async fn run(shared: Arc<Shared>, registry: DriverRegistry, links: Arc<dyn LinkSource>) {
    let drivers = registry.all();
    loop {
        let (stopped, all_paused, paused) = {
            let control = lock(&shared.control);
            (control.stopped, control.all_paused, control.paused.clone())
        };
        if stopped {
            return;
        }
        if !all_paused {
            let live = links.links();
            for driver in &drivers {
                let family = driver.manifest().family.clone();
                if paused.contains(&family) {
                    continue;
                }
                for link in live.iter().filter(|l| driver.manifest().handles(&l.kind)) {
                    let key = Key::Discover {
                        family: family.clone(),
                        link: link.id.clone(),
                    };
                    if let Some(claim) = shared.claim(key) {
                        tokio::spawn(discover(
                            Arc::clone(&shared),
                            Arc::clone(driver),
                            link.clone(),
                            claim,
                        ));
                    }
                }
            }
            let live_ids: BTreeSet<LinkId> = live.into_iter().map(|l| l.id).collect();
            shared.prune_links(&live_ids);
        }
        time::sleep(shared.config.tick).await;
    }
}

async fn discover(shared: Arc<Shared>, driver: Arc<dyn Driver>, link: Link, claim: Claim) {
    let family = driver.manifest().family.clone();
    let key = claim.key.clone();
    let limit = shared.config.discover_timeout;
    let (d, l) = (Arc::clone(&driver), link.clone());
    match guarded(limit, async move { d.discover(&l).await }).await {
        Ok(candidates) => {
            if shared.succeeded(&key, shared.config.interval) {
                shared.events.emit(Event::ProbeRecovered {
                    family: family.clone(),
                    link: link.id.clone(),
                });
            }
            let addresses: Vec<String> = candidates.iter().map(|c| c.address.clone()).collect();
            shared.reconcile(&family, &link.id, &addresses);
            shared.events.emit(Event::Discovered {
                family: family.clone(),
                link: link.id.clone(),
                addresses,
            });
            for candidate in candidates {
                let identify_key = Key::Identify {
                    family: family.clone(),
                    link: link.id.clone(),
                    address: candidate.address.clone(),
                };
                if let Some(identify_claim) = shared.claim(identify_key) {
                    tokio::spawn(identify(
                        Arc::clone(&shared),
                        Arc::clone(&driver),
                        candidate,
                        identify_claim,
                    ));
                }
            }
        }
        Err(error) if error.is_settled() => {
            let message = error.to_string();
            if shared.settle(key, message.clone()) {
                shared.events.emit(Event::Settled {
                    family,
                    link: link.id,
                    address: None,
                    error: message,
                });
            }
        }
        Err(error) => {
            let (attempt, delay) = shared.failed(&key);
            shared.events.emit(Event::ProbeFailed {
                family,
                link: link.id,
                error: error.to_string(),
                timed_out: error.timed_out(),
                attempt,
                retry_in_ms: millis(delay),
            });
        }
    }
}

async fn identify(
    shared: Arc<Shared>,
    driver: Arc<dyn Driver>,
    candidate: Candidate,
    claim: Claim,
) {
    let key = claim.key.clone();
    let family = driver.manifest().family.clone();
    let address = candidate.address.clone();
    let limit = shared.config.identify_timeout;
    let (d, c) = (Arc::clone(&driver), candidate.clone());
    let result = guarded(limit, async move {
        let identity = d.identify(&c).await?;
        let capabilities = d.capabilities(&identity);
        Ok::<_, atlas_driver::DriverError>((identity, capabilities))
    })
    .await;
    match result {
        Ok((identity, capabilities)) => {
            shared.succeeded(&key, shared.config.interval);
            shared.events.emit(Event::Identified {
                identity,
                capabilities,
            });
        }
        Err(error) if error.is_settled() => {
            let message = error.to_string();
            if shared.settle(key, message.clone()) {
                shared.events.emit(Event::Settled {
                    family,
                    link: candidate.link,
                    address: Some(address),
                    error: message,
                });
            }
        }
        Err(error) => {
            shared.failed(&key);
            shared.events.emit(Event::Unanswered {
                candidate,
                error: error.to_string(),
                timed_out: error.timed_out(),
            });
        }
    }
}

/// Pauses and resumes the scan. Every method returns at once; calls already
/// running finish or time out on their own.
#[derive(Clone)]
pub struct ScanControl {
    shared: Arc<Shared>,
}

impl fmt::Debug for ScanControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScanControl").finish_non_exhaustive()
    }
}

impl ScanControl {
    /// Starts no new calls for any driver. The manual off.
    pub fn pause_all(&self) {
        lock(&self.shared.control).all_paused = true;
        self.shared.events.emit(Event::ScanPaused { family: None });
    }

    pub fn resume_all(&self) {
        lock(&self.shared.control).all_paused = false;
        self.shared.events.emit(Event::ScanResumed { family: None });
    }

    pub fn pause_family(&self, family: &Family) {
        lock(&self.shared.control).paused.insert(family.clone());
        self.shared.events.emit(Event::ScanPaused {
            family: Some(family.clone()),
        });
    }

    pub fn resume_family(&self, family: &Family) {
        lock(&self.shared.control).paused.remove(family);
        self.shared.events.emit(Event::ScanResumed {
            family: Some(family.clone()),
        });
    }

    /// Whether new calls for this family are held back.
    pub fn is_paused(&self, family: &Family) -> bool {
        let control = lock(&self.shared.control);
        control.all_paused || control.paused.contains(family)
    }

    /// Asks again every settled answer of this family on the next round.
    pub fn ask_again(&self, family: &Family) {
        self.shared.ask_again(family);
    }

    /// Ends the scan loop within one tick. Calls already running finish.
    pub fn stop(&self) {
        lock(&self.shared.control).stopped = true;
    }
}
