//! The robot network as the drivers see it: every probe with its own timeout,
//! answers kept for a short while, and the helpers the three families share.
//!
//! Nothing here waits on another probe. Each call is bounded by its own limit
//! and a probe that hangs gives "no answer" for itself only, so an unreachable
//! network costs one limit per batch, not one per address. Results of connects
//! and name lookups are kept for [`CACHE_FOR`], so a scan that runs every two
//! seconds does not connect to the same addresses every time.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use atlas_driver::DriverError;
use atlas_types::{Candidate, Family, Link, LinkId};
use tokio::time::{self, Instant};

use crate::reach::{Answer, CONNECT_LIMIT, HTTP_LIMIT, NAME_LIMIT, Reach};
use crate::team::Teams;

/// How long a connect or a name lookup is kept before it is asked again.
pub const CACHE_FOR: Duration = Duration::from_secs(20);

/// Runs `call` for at most `limit`. `None` when it did not finish in time.
pub(crate) async fn bounded<T>(limit: Duration, call: impl Future<Output = T>) -> Option<T> {
    time::timeout(limit, call).await.ok()
}

#[derive(Debug)]
struct Kept<T> {
    at: Instant,
    value: T,
}

#[derive(Debug, Default)]
struct Cache {
    opened: BTreeMap<(Ipv4Addr, u16), Kept<bool>>,
    names: BTreeMap<String, Kept<Vec<Ipv4Addr>>>,
}

/// The robot network: what the drivers ask of it, bounded and cached.
pub struct RobotNet {
    reach: Arc<dyn Reach>,
    teams: Arc<dyn Teams>,
    cache: Mutex<Cache>,
}

impl fmt::Debug for RobotNet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RobotNet").finish_non_exhaustive()
    }
}

impl RobotNet {
    pub fn new(reach: Arc<dyn Reach>, teams: Arc<dyn Teams>) -> Arc<Self> {
        Arc::new(Self {
            reach,
            teams,
            cache: Mutex::new(Cache::default()),
        })
    }

    /// The teams to look for, as the inventory knows them now.
    pub fn teams(&self) -> Vec<u16> {
        self.teams.teams().into_iter().collect()
    }

    /// `GET path` on `ip:port`, within [`HTTP_LIMIT`].
    pub async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer> {
        bounded(HTTP_LIMIT, self.reach.get(ip, port, path))
            .await
            .flatten()
    }

    /// Whether `ip:port` accepts a connection, within [`CONNECT_LIMIT`].
    pub async fn open(&self, ip: Ipv4Addr, port: u16) -> bool {
        self.open_all(&[(ip, port)]).await[0]
    }

    /// Connects to every probe at once, each within [`CONNECT_LIMIT`], and
    /// says which accepted. Answers less than [`CACHE_FOR`] old are reused.
    pub async fn open_all(&self, probes: &[(Ipv4Addr, u16)]) -> Vec<bool> {
        let known: Vec<Option<bool>> = {
            let cache = self.lock();
            probes
                .iter()
                .map(|probe| fresh(cache.opened.get(probe)).copied())
                .collect()
        };
        let answers = futures::future::join_all(probes.iter().zip(&known).map(
            |(&(ip, port), known)| async move {
                match known {
                    Some(open) => *open,
                    None => bounded(CONNECT_LIMIT, self.reach.open(ip, port))
                        .await
                        .unwrap_or(false),
                }
            },
        ))
        .await;
        let mut cache = self.lock();
        let now = Instant::now();
        for ((probe, known), open) in probes.iter().zip(&known).zip(&answers) {
            if known.is_none() {
                cache.opened.insert(
                    *probe,
                    Kept {
                        at: now,
                        value: *open,
                    },
                );
            }
        }
        answers
    }

    /// The IPv4 addresses `name` resolves to, within [`NAME_LIMIT`].
    pub async fn resolve(&self, name: &str) -> Vec<Ipv4Addr> {
        if let Some(known) = fresh(self.lock().names.get(name)) {
            return known.clone();
        }
        let found = bounded(NAME_LIMIT, self.reach.resolve(name))
            .await
            .unwrap_or_default();
        self.lock().names.insert(
            name.to_owned(),
            Kept {
                at: Instant::now(),
                value: found.clone(),
            },
        );
        found
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The value kept under `kept`, if it is younger than [`CACHE_FOR`].
fn fresh<T>(kept: Option<&Kept<T>>) -> Option<&T> {
    kept.filter(|kept| kept.at.elapsed() < CACHE_FOR)
        .map(|kept| &kept.value)
}

/// A candidate for `family` at `ip` on `link`.
pub(crate) fn candidate(link: &LinkId, family: &str, ip: Ipv4Addr) -> Candidate {
    Candidate {
        link: link.clone(),
        family: Family::new(family),
        address: ip.to_string(),
    }
}

/// The address a candidate names. Anything else is refused, as settled: the
/// same address will not become an address later.
pub(crate) fn ipv4(address: &str) -> Result<Ipv4Addr, DriverError> {
    address.parse().map_err(|_| {
        DriverError::Incompatible(format!("`{address}` is not an IPv4 address of a robot"))
    })
}

/// Whether a driver handles this link at all. The registry checks this
/// already; a direct call on another kind finds nothing.
pub(crate) fn handles(manifest: &atlas_driver::DriverManifest, link: &Link) -> bool {
    manifest.handles(&link.kind)
}

/// A device that does not answer, which may answer later on its own.
pub(crate) fn unreachable(ip: Ipv4Addr, port: u16) -> DriverError {
    DriverError::Unreachable(format!("{ip}:{port} did not answer"))
}

/// A GET that must come back 2xx to count as the device's answer.
pub(crate) fn expect_ok(
    ip: Ipv4Addr,
    port: u16,
    answer: Option<Answer>,
) -> Result<Answer, DriverError> {
    match answer {
        Some(answer) if (200..300).contains(&answer.status) => Ok(answer),
        Some(answer) => Err(DriverError::Unreachable(format!(
            "{ip}:{port} answered HTTP {}",
            answer.status
        ))),
        None => Err(unreachable(ip, port)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fake::Fake;

    /// A connect is remembered for [`CACHE_FOR`], then asked again. A scan
    /// every two seconds does not connect to the same addresses each time.
    #[tokio::test(start_paused = true)]
    async fn a_connect_is_reused_until_the_cache_expires() {
        let ip = Ipv4Addr::new(10, 63, 90, 2);
        let fake = Fake::new().open(ip, 80);
        let net = RobotNet::new(fake.reach(), Arc::new(BTreeSet::<u16>::new()));
        assert_eq!(net.open_all(&[(ip, 80), (ip, 81)]).await, [true, false]);
        assert_eq!(net.open_all(&[(ip, 80), (ip, 81)]).await, [true, false]);
        assert_eq!(fake.connects(), 2);
        tokio::time::advance(CACHE_FOR + Duration::from_secs(1)).await;
        assert_eq!(net.open_all(&[(ip, 80)]).await, [true]);
        assert_eq!(fake.connects(), 3);
    }

    /// A probe that hangs is cut at its own limit, and reads as closed.
    #[tokio::test(start_paused = true)]
    async fn a_hung_connect_reads_as_closed_within_its_limit() {
        let ip = Ipv4Addr::new(10, 63, 90, 2);
        let net = RobotNet::new(Fake::new().hang().reach(), Arc::new(BTreeSet::<u16>::new()));
        let started = Instant::now();
        assert_eq!(net.open_all(&[(ip, 80)]).await, [false]);
        assert_eq!(started.elapsed(), CONNECT_LIMIT);
    }
}
