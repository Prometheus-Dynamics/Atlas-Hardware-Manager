//! The lease a board keeps per Orion caller identity (Lemnos I6g), on a simulated clock.
//!
//! - A board keeps a lease only when its hello facts say so: `lease.seconds` (the period) and
//!   `leases.ended` (how many leases it has ended). With no period (see [`crate::Hello`]) there is
//!   no lease and none of these facts.
//! - A caller's request renews its lease at once: every request, including one refused for its
//!   arguments, the end or cancel of one of its runs, and every progress report of its long job.
//!   Resource queries and topic watching do not reach the lease and renew nothing.
//! - A caller with a run in flight is alive: its lease is not ended while the run is running.
//! - [`SimNode::lease_advance`] moves the simulated clock. A caller silent for longer than the
//!   period, with no run in flight, loses its lease: the board drops its holds (counted in
//!   `leases.ended`), and the caller's first reply after that carries [`NOTE`] once.
//! - A stalled run (the stall behaviour of a script, the only transfer-like state this node has)
//!   ends when it has been stalled for longer than the stall time. Files are not modelled here.
//! - Nothing here sleeps: a drop is applied on the node's own thread at its next tick, and each
//!   drop is counted so a test can wait for it without a real delay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

/// The lease period of the locked bridge.
// LOCK: Lemnos I6g lease period (the bridge's default, 60 s; the fact `lease.seconds`).
pub const PERIOD: Duration = Duration::from_secs(60);
/// The stall time of the locked bridge's transfers.
// LOCK: Lemnos I6g stall rule (a stalled transfer is dropped after 600 s).
pub const STALL: Duration = Duration::from_secs(600);
/// The start of the detail of the first reply after a lease expiry, once per expiry.
// LOCK: Lemnos I6g2 (`lease-ended: holds released`, checked by the session's `strip_note`).
pub const NOTE: &str = "lease-ended: holds released";

/// The detail of a reply that carries the lease note: the note alone, or the note then the reply's
/// own detail after `"; "`.
pub(crate) fn with_note(detail: &str) -> String {
    if detail.is_empty() {
        NOTE.to_owned()
    } else {
        format!("{NOTE}; {detail}")
    }
}

/// What a stalled run ends with when the stall time ran out.
pub(crate) const STALL_WORDS: &str = "the stall time ran out: this run was stalled for longer than the board allows, and was dropped";

/// What the node must end, as a drop the lease decided: one stalled run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    /// One stalled run.
    Run(u64),
}

/// The lease state of one simulated node.
#[derive(Debug)]
pub(crate) struct Leases {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    now: Duration,
    /// The lease period, `None` when the board keeps no lease.
    period: Option<Duration>,
    stall: Duration,
    /// The simulated time of each caller's last request.
    last: BTreeMap<String, Duration>,
    /// The callers with a run in flight: alive whatever their silence.
    running: BTreeSet<String>,
    /// The callers whose lease ended, and whose first reply carries the note.
    noted: BTreeSet<String>,
    /// Whether the note is sent at all. Off models a board before I6g2: only `leases.ended` says so.
    note: bool,
    /// The simulated time each run was stalled at.
    stalled: BTreeMap<u64, Duration>,
    pending: Vec<Drop>,
    dropped: usize,
    ended: usize,
}

impl Leases {
    pub(crate) fn new(period: Option<Duration>) -> Self {
        Self {
            state: Mutex::new(State {
                now: Duration::ZERO,
                period,
                stall: STALL,
                last: BTreeMap::new(),
                running: BTreeSet::new(),
                noted: BTreeSet::new(),
                note: true,
                stalled: BTreeMap::new(),
                pending: Vec::new(),
                dropped: 0,
                ended: 0,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The hello facts of this lease, as `(name, text)`: none when the board keeps no lease.
    pub(crate) fn facts(&self) -> Vec<(&'static str, String)> {
        let state = self.state();
        match state.period {
            Some(period) => vec![
                (crate::names::LEASE_SECONDS, period.as_secs().to_string()),
                (crate::names::LEASES_ENDED, state.ended.to_string()),
            ],
            None => Vec::new(),
        }
    }

    /// A request from `caller` reached the node: its lease starts again from now. Off: nothing.
    pub(crate) fn request(&self, caller: &str) {
        let mut state = self.state();
        if state.period.is_none() {
            return;
        }
        let now = state.now;
        state.last.insert(caller.to_owned(), now);
    }

    /// Sets the callers with a run in flight, as the node's own runs stand now.
    pub(crate) fn set_running(&self, callers: impl Iterator<Item = String>) {
        self.state().running = callers.collect();
    }

    /// Whether `caller`'s first reply carries the lease note; the note is spent by the reply.
    pub(crate) fn take_note(&self, caller: &str) -> bool {
        let mut state = self.state();
        let noted = state.noted.remove(caller);
        noted && state.note
    }

    /// Turns the note on or off (a test of a board without I6g2 has it off).
    pub(crate) fn set_note(&self, on: bool) {
        self.state().note = on;
    }

    /// Run `call` stalled now.
    pub(crate) fn stalled(&self, call: u64) {
        let mut state = self.state();
        let now = state.now;
        state.stalled.insert(call, now);
    }

    /// Moves the simulated clock by `by` and decides the drops: every caller silent for longer than
    /// the period with no run in flight, and every stalled run stalled for longer than the stall
    /// time. A dropped caller has no lease until its next request, and its next reply notes it.
    pub(crate) fn advance(&self, by: Duration) {
        let mut state = self.state();
        state.now += by;
        let now = state.now;
        let (period, stall) = (state.period, state.stall);
        for caller in state.running.clone() {
            state.last.insert(caller, now);
        }
        if let Some(period) = period {
            let silent: Vec<String> = state
                .last
                .iter()
                .filter(|(_, last)| now.saturating_sub(**last) > period)
                .map(|(caller, _)| caller.clone())
                .collect();
            for caller in silent {
                state.last.remove(&caller);
                state.noted.insert(caller);
                state.ended += 1;
                state.dropped += 1;
            }
        }
        let stuck: Vec<u64> = state
            .stalled
            .iter()
            .filter(|(_, since)| now.saturating_sub(**since) > stall)
            .map(|(call, _)| *call)
            .collect();
        for call in stuck {
            state.stalled.remove(&call);
            state.pending.push(Drop::Run(call));
        }
    }

    /// Sets the period and the stall time (a test's shorter ones). Turns the lease on.
    pub(crate) fn set(&self, period: Duration, stall: Duration) {
        let mut state = self.state();
        state.period = Some(period);
        state.stall = stall;
    }

    /// The stalled-run drops decided since the last call. Counted as they are taken.
    pub(crate) fn take(&self) -> Vec<Drop> {
        let mut state = self.state();
        let pending = std::mem::take(&mut state.pending);
        state.dropped += pending.len();
        pending
    }

    /// How many drops have been applied in all: silent callers and stalled runs.
    pub(crate) fn dropped(&self) -> usize {
        let state = self.state();
        state.dropped + state.pending.len()
    }
}
