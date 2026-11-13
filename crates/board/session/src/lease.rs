//! The lease: a board keeps a caller's holds only while the caller keeps asking.
//!
//! The board's bridge keeps one lease per Orion caller identity. A caller that makes no request to
//! that board's bridge for the lease period has its daemon connection dropped, and that releases
//! what it held: claimed devices, running action runs and transfers. Any request renews the lease.
//! So a client that takes a hold and then goes silent loses it, and a client that dies cannot keep
//! it. This protects the board, and it breaks a client that stays silent while it holds something.
//!
//! A [`Hold`] says "this session holds something on its board". Holding it means a hold exists;
//! dropping it ends the hold. While at least one hold exists, the session keeps the lease alive: it
//! makes sure some request reached the board's bridge at least every [`Lease::renew`]. A request
//! of the holder's own already does that (see [`crate::Handle::with_bridge`]), so the session then
//! sends nothing. With no hold, the session sends nothing for the lease at all. A node without
//! Lemnos has no bridge and no lease, so nothing is sent to it either.
//!
//! What counts as a request to the bridge: an action run, its cancel (through
//! [`crate::Handle::with_bridge`]), and the renewal itself. Reads through [`crate::Handle::with_client`]
//! (topics, the node's own hello) do not reach the bridge, so they do not renew the lease. A file
//! transfer's chunks are not counted either: at the locked Orion revision the transfer is not keyed
//! by caller (the answers in `I6g-answers.md`, A3), so the session does not rely on them.
//!
//! When the session has been silent for longer than the lease period while holds existed, it cannot
//! know whether the board dropped them (the machine may have slept, or the session was down). It
//! records a lapse, and each hold reports it through [`Hold::lapsed`]. Holders treat a lapse as an
//! outcome of its own: "the board may have dropped the hold".

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orion::actions::{ActionCall, Target};
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError, Orion};
use tokio::sync::watch;

/// The lease period the bridge keeps per caller identity.
// LOCK: Lemnos I6g lease period. The bridge's default is 60 s (`LEMNOS_ORION_LEASE`, its
// `config.rs` `LEASE`). No hello fact, topic or description tells a client the period, so it is
// kept here until one does; then read it from the node.
pub const LEASE: Duration = Duration::from_secs(60);

/// Whether the board's bridge keeps a lease at all: the lease is a Lemnos I6g change, and the locked
/// bridge (`d6b9361d`) keeps none. Nothing a client can read says it is in force: no hello fact, no
/// topic, no contract in `hello.offers`, and the only probe (`.call describe`, I6h) is itself a request.
/// Until the lock moves, a session sends nothing for a lease, lists no devices for one, and records no
/// lapse, so no hold can be reported as dropped by a lease that does not exist.
// LOCK: Lemnos I6g/I6h. The lock move that brings the lease flips this to `true`.
pub const LEASE_IN_FORCE: bool = false;

/// How often a held lease is renewed when nothing else renewed it: a third of [`LEASE`]. Two
/// renewals may fail (a refused or slow one) before the lease runs out. The session checks every
/// 100 ms, so a renewal is never more than that late.
pub const LEASE_RENEW: Duration = Duration::from_secs(20);

/// How long a renewal waits for the bridge's answer. A refusal answers at once, which is the
/// normal case. A bridge that stays silent is cut here, so one renewal never holds the session.
pub const RENEW_WAIT: Duration = Duration::from_secs(2);

/// The prefix of the resources a node's bridge lists its devices under.
// LOCK: Lemnos d6b9361d: `RESOURCE_PREFIX` in `service/orion/src/lib.rs`, kind `lemnos.device`.
const DEVICE_PREFIX: &str = "lemnos.";
// LOCK: Lemnos d6b9361d: `RESOURCE_KIND` in `service/orion/src/lib.rs`.
const DEVICE_KIND: &str = "lemnos.device";

/// The clock the lease is measured on. A session reads it for every silence, renewal and lapse. The
/// real clock is the monotonic time since it was made. A manual clock moves only when a test moves it
/// with [`Clock::advance`], so a test states "now the lease has passed" instead of sleeping and hoping.
#[derive(Clone, Debug)]
pub struct Clock {
    source: Arc<Source>,
}

#[derive(Debug)]
enum Source {
    Real(std::time::Instant),
    Manual(Mutex<Duration>),
}

impl Clock {
    /// The monotonic clock of this process: what a production session uses.
    #[must_use]
    pub fn real() -> Self {
        Self {
            source: Arc::new(Source::Real(std::time::Instant::now())),
        }
    }

    /// A clock that stands still until [`Clock::advance`] moves it. It starts at zero.
    #[must_use]
    pub fn manual() -> Self {
        Self {
            source: Arc::new(Source::Manual(Mutex::new(Duration::ZERO))),
        }
    }

    /// The time on this clock, from its start.
    #[must_use]
    pub fn now(&self) -> Duration {
        match &*self.source {
            Source::Real(origin) => origin.elapsed(),
            Source::Manual(now) => *now.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Moves a manual clock forward by `by`. A real clock does not move: it is only advanced by time
    /// passing, so this does nothing on it.
    pub fn advance(&self, by: Duration) {
        if let Source::Manual(now) = &*self.source {
            *now.lock().unwrap_or_else(PoisonError::into_inner) += by;
        }
    }
}

/// The period and the renewal interval of a session's lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    /// How long a caller may stay silent before the board drops its holds ([`LEASE`]).
    pub period: Duration,
    /// How long the session waits between renewals while holds exist ([`LEASE_RENEW`]). Must be
    /// shorter than `period`.
    pub renew: Duration,
    /// Whether the board keeps a lease for this session. `false` sends nothing, lists nothing and
    /// records no lapse. The locked board is [`LEASE_IN_FORCE`]; a test turns it on.
    pub in_force: bool,
}

impl Lease {
    /// The lease as the board keeps it at the locked revision ([`LEASE_IN_FORCE`]).
    pub const DEFAULT: Self = Self {
        period: LEASE,
        renew: LEASE_RENEW,
        in_force: LEASE_IN_FORCE,
    };
}

impl Default for Lease {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What a session's holds share: how many exist, and how many lapses the session recorded.
#[derive(Debug)]
pub(crate) struct Shared {
    holds: AtomicUsize,
    lapses: watch::Sender<u64>,
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            holds: AtomicUsize::new(0),
            lapses: watch::channel(0).0,
        }
    }

    /// How many holds exist now.
    pub(crate) fn holds(&self) -> usize {
        self.holds.load(Ordering::SeqCst)
    }

    /// Records a lapse: the board may have dropped every hold.
    pub(crate) fn lapse(&self) {
        self.lapses.send_modify(|count| *count += 1);
    }

    fn count(&self) -> u64 {
        *self.lapses.borrow()
    }

    /// A receiver of the lapse count, for a holder that waits on the board.
    pub(crate) fn lapses_watch(&self) -> watch::Receiver<u64> {
        self.lapses.subscribe()
    }
}

/// A hold on a session's board: while it exists, the session keeps the board's lease alive.
///
/// Cheap to clone; each clone is a hold of its own, and each drop ends one. Several holders may
/// hold at once. A hold does not keep the session connected and does not make a call by itself.
#[derive(Debug)]
pub struct Hold {
    shared: Arc<Shared>,
    /// The lapse count this hold has seen. A higher count is a lapse it has not yet acknowledged.
    seen: u64,
}

impl Hold {
    pub(crate) fn take(shared: &Arc<Shared>) -> Self {
        shared.holds.fetch_add(1, Ordering::SeqCst);
        Self {
            shared: Arc::clone(shared),
            seen: shared.count(),
        }
    }

    /// Whether the session lapsed while this hold existed and has not been acknowledged: the board
    /// may have dropped what the hold held. Its outcome is then "the board may have dropped the
    /// hold", not whatever the board's last answer said.
    #[must_use]
    pub fn lapsed(&self) -> bool {
        self.shared.count() > self.seen
    }

    /// Takes the lapses so far as seen: [`Hold::lapsed`] is false until the next one. A holder that
    /// handles a lapse by sending its work again calls this first.
    pub fn acknowledge(&mut self) {
        self.seen = self.shared.count();
    }
}

impl Clone for Hold {
    fn clone(&self) -> Self {
        self.shared.holds.fetch_add(1, Ordering::SeqCst);
        Self {
            shared: Arc::clone(&self.shared),
            seen: self.seen,
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.shared.holds.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The resource of the device the renewal goes through, as the bridge lists it: the first
/// `lemnos.device` resource under the prefix. `None` when the node lists no device.
pub(crate) fn device_for_renewal(orion: &Orion) -> Result<Option<String>, OrionError> {
    let found = orion.resources(DEVICE_PREFIX)?;
    Ok(found
        .into_iter()
        .find(|resource| resource.kind == DEVICE_KIND)
        .map(|resource| resource.name))
}

/// Renews the lease through the bridge of `device` (a resource name from [`device_for_renewal`]).
///
/// It is one action run on the device's `.call describe`, which reads the description and changes
/// nothing. The session calls it only while [`Lease::in_force`] is set.
///
/// `Ok` also when the bridge answered in any way, including a refusal, or did not finish in
/// [`RENEW_WAIT`] (the run is then cancelled, and the cancel is a request too).
///
/// # Errors
///
/// [`OrionError`] when the session is gone or the run could not be sent. The caller checks
/// [`OrionError::is_lost`].
pub(crate) fn renew(orion: &Orion, node: &NodeName, device: &str) -> Result<(), OrionError> {
    // LOCK: Lemnos I6h describe. The renewal is `describe` with no other arguments: the bridge
    // answers the device description as text, a read. The session calls this only while the lease
    // is in force (see `LEASE_IN_FORCE`); at the locked revision nothing is sent.
    const RENEW_ARGS: &[u8] = b"describe";
    let name = format!("{device}.call");
    let mut run = orion.run(&ActionCall {
        name: &name,
        target: Target::Node(node.clone()),
        args: RENEW_ARGS,
        deadline_ms: 0,
        detached: false,
    })?;
    match run.finish(RENEW_WAIT) {
        Ok(_) | Err(ClientError::Refused(_)) => Ok(()),
        Err(ClientError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {
            let _ = run.cancel();
            Ok(())
        }
        Err(error) => Err(OrionError::Client(error)),
    }
}
