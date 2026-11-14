//! The lease: a board keeps a caller's holds only while the caller keeps asking.
//!
//! The board's bridge keeps one lease per Orion caller identity. A caller that makes no request to
//! that board's bridge for the lease period has its daemon connection dropped, and that releases
//! what it held: claimed devices, idle terminals, and any run or transfer it started. Only silence
//! expires a lease. The board says whether it keeps one at all, and for how long, in its hello
//! facts ([`LEASE_FACT`]): with no such fact, or a fact that is 0 or not a number, there is no lease
//! and this session sends nothing for one.
//!
//! A [`Hold`] says "this session holds something on its board", and has one of three kinds. An
//! idle hold (an open terminal, a claimed device) is kept alive by the session: while one exists and
//! nothing else is in flight, the session makes sure some request reached the bridge every third of
//! the lease period. A run hold (a job's run, a board install) stands for work the board keeps its
//! caller alive for while it is in flight: the session sends no renewal and counts no silence. A
//! transfer hold (a file push) gets no renewal either, since a transfer that stalls past the lease
//! expires; its silence counts, and each accepted step of it is a request ([`Handle::hold_transfer`]).
//! The renewal is one `describe` through the node's device bridge, a read that changes nothing.
//!
//! A lapse is seen two ways. The first reply after an expiry starts its detail with
//! [`LAPSE_NOTE`] (once per expiry), which [`strip_note`] recognises. The hello fact
//! [`LEASES_ENDED_FACT`] moving while this session held something is the cross-check, and the only
//! signal before the note exists. A lapse records what was held ([`lapse_words`]); nothing is
//! re-claimed or retried, and the session keeps connecting as before.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use orion::actions::{ActionCall, Target};
use orion::node::Node;
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError, Orion};
use tokio::sync::watch;

/// How long a renewal waits for the bridge's answer. A refusal answers at once, which is the
/// normal case. A bridge that stays silent is cut here, so one renewal never holds the session.
pub const RENEW_WAIT: Duration = Duration::from_secs(2);

/// The hello fact with the lease period in seconds. Absent, 0 or not a number: no lease.
// LOCK: Lemnos I6g (`lease.seconds` in the bridge's hello facts, default 60 s).
pub const LEASE_FACT: &str = "lease.seconds";
/// The hello fact that counts the leases the board has ended (drops).
// LOCK: Lemnos I6g (`leases.ended` in the bridge's hello facts).
pub const LEASES_ENDED_FACT: &str = "leases.ended";
/// The exact start of the detail of the first reply after a lease expiry, once per expiry. The
/// reply's own detail, if it had one, follows after `"; "`.
// LOCK: Lemnos I6g2 (`lease-ended: holds released`, the bridge's reply note).
pub const LAPSE_NOTE: &str = "lease-ended: holds released";

/// The hello fact type that carries text (the same code the board identity reader uses).
// LOCK: Lemnos I6g: fact type 4 is text, as `drivers/board/src/identity.rs` reads it.
const TEXT: u8 = 4;
/// How often the hello facts are read while holds exist and the period is not known yet: the
/// board may turn its lease on while Atlas holds something.
pub(crate) const FACTS_EVERY: Duration = Duration::from_secs(5);
/// The prefix of the resources a node's bridge lists its devices under.
// LOCK: Lemnos I6g: `RESOURCE_PREFIX` in `service/orion/src/lib.rs`, kind `lemnos.device`.
const DEVICE_PREFIX: &str = "lemnos.";
// LOCK: Lemnos I6g: `RESOURCE_KIND` in `service/orion/src/lib.rs`.
const DEVICE_KIND: &str = "lemnos.device";

/// The lease facts a node's hello document carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeaseFacts {
    /// The lease period, when the board keeps one (`lease.seconds`, above zero).
    pub period: Option<Duration>,
    /// How many leases the board has ended so far (`leases.ended`), when it says.
    pub ended: Option<u64>,
}

impl LeaseFacts {
    /// The lease facts of `node`'s hello document.
    #[must_use]
    pub fn of(node: &Node) -> Self {
        Self {
            period: number(node, LEASE_FACT)
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs),
            ended: number(node, LEASES_ENDED_FACT),
        }
    }

    /// How often a renewal is due while holds exist: a third of the period. `None` with no lease.
    #[must_use]
    pub fn renew_every(&self) -> Option<Duration> {
        self.period.map(|period| period / 3)
    }
}

/// A fact that is a text of plain decimal digits, read as a number. `None` when it is absent, not
/// text, or not plain digits.
fn number(node: &Node, name: &str) -> Option<u64> {
    let fact = node.facts.iter().find(|fact| fact.name == name)?;
    if fact.ty != TEXT || fact.value.is_empty() || !fact.value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(&fact.value).ok()?.parse().ok()
}

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

/// What a hold stands for, for the lease (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Kept alive by the session's renewal.
    Idle,
    /// A transfer in flight: no renewal, its silence counts.
    Transfer,
    /// A run in flight: the board keeps its caller alive while it runs.
    Run,
}

/// One hold on the board, as the session counts it.
#[derive(Clone, Copy, Debug)]
struct Entry {
    id: u64,
    what: &'static str,
    kind: Kind,
}

/// What a session's holds share: the holds that exist, and the lapses the session recorded.
#[derive(Debug)]
pub(crate) struct Shared {
    holds: Mutex<Vec<Entry>>,
    next: AtomicU64,
    /// Set by a lapse and cleared by the next request that got through: one lapse per expiry.
    marked: AtomicBool,
    /// What was held when the last lapse was recorded.
    released: Mutex<Vec<&'static str>>,
    lapses: watch::Sender<u64>,
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            holds: Mutex::new(Vec::new()),
            next: AtomicU64::new(0),
            marked: AtomicBool::new(false),
            released: Mutex::new(Vec::new()),
            lapses: watch::channel(0).0,
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.holds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many holds exist now, of both kinds.
    pub(crate) fn holds(&self) -> usize {
        self.entries().len()
    }

    /// How many run holds exist: runs and installs the session started.
    pub(crate) fn runs(&self) -> usize {
        self.of_kind(Kind::Run)
    }

    /// How many transfer holds exist.
    pub(crate) fn transfers(&self) -> usize {
        self.of_kind(Kind::Transfer)
    }

    fn of_kind(&self, kind: Kind) -> usize {
        self.entries().iter().filter(|entry| entry.kind == kind).count()
    }

    fn enter(&self, what: &'static str, kind: Kind) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.entries().push(Entry { id, what, kind });
        id
    }

    fn leave(&self, id: u64) {
        self.entries().retain(|entry| entry.id != id);
    }

    /// Records a lapse: the board may have dropped every hold. Once per expiry: a second signal
    /// for the same expiry does nothing until a request gets through again (see [`Shared::alive`]).
    pub(crate) fn lapse(&self) {
        if self.marked.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut what: Vec<&'static str> = Vec::new();
        for entry in self.entries().iter() {
            if !what.contains(&entry.what) {
                what.push(entry.what);
            }
        }
        *self.released.lock().unwrap_or_else(PoisonError::into_inner) = what;
        self.lapses.send_modify(|count| *count += 1);
    }

    /// A request reached the board: an expiry that comes later is a new one.
    pub(crate) fn alive(&self) {
        self.marked.store(false, Ordering::SeqCst);
    }

    fn count(&self) -> u64 {
        *self.lapses.borrow()
    }

    /// What the last lapse released, in words for people (see [`lapse_words`]).
    pub(crate) fn released(&self) -> Vec<&'static str> {
        self.released
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    id: u64,
    /// The lapse count this hold has seen. A higher count is a lapse it has not yet acknowledged.
    seen: u64,
    what: &'static str,
    kind: Kind,
}

impl Hold {
    pub(crate) fn take(shared: &Arc<Shared>, what: &'static str, kind: Kind) -> Self {
        let id = shared.enter(what, kind);
        Self {
            shared: Arc::clone(shared),
            id,
            seen: shared.count(),
            what,
            kind,
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

    /// What this hold stands for, in words for people (for example "an open terminal").
    #[must_use]
    pub const fn what(&self) -> &'static str {
        self.what
    }

}

impl Clone for Hold {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            id: self.shared.enter(self.what, self.kind),
            seen: self.seen,
            what: self.what,
            kind: self.kind,
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.shared.leave(self.id);
    }
}

/// The detail of a reply after the lapse note: `Some(rest)` when `detail` is exactly [`LAPSE_NOTE`]
/// or starts with it followed by `"; "`, where `rest` is what follows the separator. `None` for any
/// other detail, including one that only contains the note further on.
#[must_use]
pub fn strip_note(detail: &str) -> Option<&str> {
    let rest = detail.strip_prefix(LAPSE_NOTE)?;
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix("; ")
    }
}

/// What a person is told after a lapse: which holds the board released, and that nothing was retried
/// or claimed again. `released` lists the kinds of hold that were held when the lapse was recorded.
#[must_use]
pub fn lapse_words(released: &[&str]) -> String {
    if released.is_empty() {
        "the board's lease for this session ran out; nothing was retried or claimed again".to_owned()
    } else {
        format!(
            "the board's lease for this session ran out and released {}; nothing was retried or claimed again",
            released.join(", ")
        )
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
/// nothing. Returns the reply's detail (empty when the bridge refused or did not finish in time), so
/// the caller can see the lapse note. The session calls it only while the board keeps a lease.
///
/// # Errors
///
/// [`OrionError`] when the session is gone or the run could not be sent. The caller checks
/// [`OrionError::is_lost`].
pub(crate) fn renew(orion: &Orion, node: &NodeName, device: &str) -> Result<String, OrionError> {
    // LOCK: Lemnos I6g: `describe` through the device's `.call` is a request the bridge renews on.
    // It is the request the lock offers that reads and changes nothing. At d6b9361d the bridge has
    // no `describe` yet (its I6h is merged with I6g), so nothing is sent there: the board has no
    // lease fact until the lock moves.
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
        Ok(finished) => Ok(finished.detail),
        Err(ClientError::Refused(_)) => Ok(String::new()),
        Err(ClientError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {
            let _ = run.cancel();
            Ok(String::new())
        }
        Err(error) => Err(OrionError::Client(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lapse_note_is_exactly_the_note_or_the_note_then_a_separator() {
        assert_eq!(strip_note("lease-ended: holds released"), Some(""));
        assert_eq!(
            strip_note("lease-ended: holds released; the run was refused"),
            Some("the run was refused")
        );
        // A note with no separator, and a note only contained later, are not a lapse.
        assert_eq!(strip_note("lease-ended: holds released-ish"), None);
        assert_eq!(strip_note("the reply said lease-ended: holds released"), None);
        assert_eq!(strip_note(""), None);
    }

    #[test]
    fn the_lease_period_is_read_from_plain_digits_only() {
        assert_eq!(
            number_of("60"),
            Some(60),
            "a plain number is the period in seconds"
        );
        assert_eq!(number_of("0"), Some(0));
        assert_eq!(number_of(""), None);
        assert_eq!(number_of("60s"), None);
        assert_eq!(number_of(" 60"), None);
        assert_eq!(number_of("+60"), None);
    }

    #[test]
    fn a_renewal_is_a_third_of_the_period_and_none_without_a_lease() {
        let facts = LeaseFacts {
            period: Some(Duration::from_secs(60)),
            ended: None,
        };
        assert_eq!(facts.renew_every(), Some(Duration::from_secs(20)));
        assert_eq!(LeaseFacts::default().renew_every(), None);
    }

    #[test]
    fn the_words_name_what_was_released_and_say_nothing_was_retried() {
        let words = lapse_words(&["an open terminal", "a board install"]);
        assert!(words.contains("an open terminal, a board install"), "{words}");
        assert!(words.contains("nothing was retried"), "{words}");
        assert!(lapse_words(&[]).contains("nothing was retried"));
    }

    fn number_of(text: &str) -> Option<u64> {
        let node = Node {
            name: String::new(),
            cluster: String::new(),
            public_key: [0; 32],
            fingerprint: String::new(),
            revision: 0,
            time_ms: 0,
            facts: vec![orion::node::Fact {
                name: LEASE_FACT.to_owned(),
                ty: TEXT,
                value: text.as_bytes().to_vec(),
            }],
        };
        number(&node, LEASE_FACT)
    }
}
