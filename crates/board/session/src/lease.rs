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

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orion::actions::{ActionCall, Outcome};
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError, Orion};
use tokio::sync::watch;

/// The lease period the bridge keeps per caller identity, until a describe reply gives its own.
// board: Lemnos bbb3c420 service/orion/src/config.rs:220 (LEASE, the default when LEMNOS_ORION_LEASE is unset).
// The board's own period is read from the device record's `lease_seconds` (see `lease_seconds_of`).
pub const LEASE: Duration = Duration::from_secs(60);

/// Whether the board's bridge keeps a lease for each caller. At the pin it does: the bridge drops a
/// silent caller's holds after the lease period, and says so in its next reply (`LEASE_ENDED_NOTE`).
/// A session with holds renews the lease, and records a lapse when it was silent past the period.
// board: Lemnos bbb3c420 service/orion/src/config.rs:81 (LEMNOS_ORION_LEASE, unset is the lease) and
// service/orion/src/calls.rs:440-463 (expire: a caller silent for the lease is released).
pub const LEASE_IN_FORCE: bool = true;

/// How often a held lease is renewed when nothing else renewed it: a third of [`LEASE`]. Two
/// renewals may fail (a refused or slow one) before the lease runs out. The session checks every
/// 100 ms, so a renewal is never more than that late.
pub const LEASE_RENEW: Duration = Duration::from_secs(20);

/// How long a renewal waits for the bridge's answer. A refusal answers at once, which is the
/// normal case. A bridge that stays silent is cut here, so one renewal never holds the session.
pub const RENEW_WAIT: Duration = Duration::from_secs(2);

/// The prefix of the resources a node's bridge lists its devices under.
// board: Lemnos bbb3c420 service/orion/src/lib.rs:51 (RESOURCE_PREFIX)
const DEVICE_PREFIX: &str = "lemnos.";
// board: Lemnos bbb3c420 service/orion/src/lib.rs:53 (RESOURCE_KIND)
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

/// What a session's holds share: how many exist, how many lapses the session recorded, and the
/// lease period the board gave (or the default, until it has).
#[derive(Debug)]
pub(crate) struct Shared {
    holds: AtomicUsize,
    lapses: watch::Sender<u64>,
    /// The board's lease period in milliseconds; 0 until a describe reply gave one.
    period_ms: AtomicU64,
    /// The period kept until the board gives its own (see [`Lease::period`]).
    fallback: Duration,
}

impl Shared {
    pub(crate) fn new(fallback: Duration) -> Self {
        Self {
            holds: AtomicUsize::new(0),
            lapses: watch::channel(0).0,
            period_ms: AtomicU64::new(0),
            fallback,
        }
    }

    /// The board's lease period: the one a describe gave, else the fallback.
    pub(crate) fn period(&self) -> Duration {
        match self.period_ms.load(Ordering::SeqCst) {
            0 => self.fallback,
            ms => Duration::from_millis(ms),
        }
    }

    pub(crate) fn set_period(&self, period: Duration) {
        let ms = u64::try_from(period.as_millis()).unwrap_or(u64::MAX).max(1);
        self.period_ms.store(ms, Ordering::SeqCst);
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
    /// The board's lease period as this session knows it: the one the board's describe reply gave, or
    /// the default until one has.
    #[must_use]
    pub fn lease_period(&self) -> Duration {
        self.shared.period()
    }

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

/// The sentence a holder shows when the session lapsed while its hold existed: what the silence
/// cost, in words a person can act on. `what` names the hold (a run, a file, a shell).
#[must_use]
pub fn lapse_words(period: Duration, what: &str) -> String {
    format!(
        "Atlas sent nothing to this board for more than {} seconds, the board's lease, so the board may have dropped the hold on {what}",
        period.as_secs()
    )
}

/// The `lease_seconds` a describe reply carries on its device record (`device generation=... lease_seconds=60`),
/// when the board gives one. The record is the reply's `device` line, after its `describe 1` version line.
// board: Lemnos bbb3c420 service/orion/src/description.rs:246 (`number(&mut out, "lease_seconds", ...)`, written only when the lease is on).
#[must_use]
pub fn lease_seconds_of(description: &str) -> Option<Duration> {
    // The reply's first line is its version (`describe 1`); the device record follows it.
    let device = description
        .lines()
        .find(|line| line.starts_with("device "))?;
    device
        .split_whitespace()
        .find_map(|word| word.strip_prefix("lease_seconds="))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
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

/// What one renewal found: whether the bridge renewed the lease, and the lease period its describe
/// reply gave, when it gave one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Renewal {
    pub renewed: bool,
    pub period: Option<Duration>,
}

/// Renews the lease through the bridge of `device` (a resource name from [`device_for_renewal`]).
///
/// It is one action run on the device's `.call describe`, which reads the description and changes
/// nothing. The session calls it only while [`Lease::in_force`] is set. Its reply carries the board's
/// lease period (see [`lease_seconds_of`]).
///
/// `renewed` only when the bridge answered the renewal successfully. A refusal, a run that did not
/// succeed, or one that did not finish in [`RENEW_WAIT`] (the run is then cancelled, and the cancel is a
/// request too) is not renewed: the session tries again on a backoff.
///
/// # Errors
///
/// [`OrionError`] when the session is gone or the run could not be sent. The caller checks
/// [`OrionError::is_lost`].
pub(crate) fn renew(orion: &Orion, _node: &NodeName, device: &str) -> Result<Renewal, OrionError> {
    // The renewal is the bridge's `describe` with no other argument: the device's description as
    // text, a read. The call is built in one place (`crate::lemnos`): the device's resource, the bridge's `describe`.
    let refused = Renewal {
        renewed: false,
        period: None,
    };
    let Ok(call) = crate::lemnos::describe_call(device) else {
        return Ok(refused);
    };
    let mut run = orion.run(&ActionCall {
        name: &call.name,
        target: call.target,
        args: &call.args,
        deadline_ms: 0,
        detached: false,
    })?;
    match run.finish(RENEW_WAIT) {
        Ok(finished) if matches!(finished.outcome, Outcome::Succeeded) => {
            let text = String::from_utf8_lossy(&finished.output);
            Ok(Renewal {
                renewed: true,
                period: lease_seconds_of(&text),
            })
        }
        Ok(_) => Ok(refused),
        Err(ClientError::Refused { .. }) => Ok(refused),
        Err(ClientError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {
            let _ = run.cancel();
            Ok(refused)
        }
        Err(error) => Err(OrionError::Client(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_period_is_the_device_records_lease_seconds() {
        let text = "describe 1\ndevice generation=4 lease_seconds=30 state=off\ncontract index=0 tag=\"x\"";
        assert_eq!(lease_seconds_of(text), Some(Duration::from_secs(30)));
    }

    #[test]
    fn a_board_that_gives_no_lease_seconds_leaves_the_default_to_the_session() {
        assert_eq!(
            lease_seconds_of("describe 1\ndevice generation=4 state=off"),
            None
        );
        assert_eq!(lease_seconds_of("describe 1\ndevice lease_seconds=0"), None);
        assert_eq!(lease_seconds_of(""), None);
    }

    #[test]
    fn the_lapse_sentence_names_the_period_and_the_hold() {
        let words = lapse_words(Duration::from_secs(60), "run 7");
        assert!(words.contains("for more than 60 seconds"), "{words}");
        assert!(words.contains("the hold on run 7"), "{words}");
    }
}
