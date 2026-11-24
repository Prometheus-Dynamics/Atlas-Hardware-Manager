//! The lease a board keeps per Orion caller identity (Lemnos I6g), on a simulated clock.
//!
//! - A caller's request (an action run, or its cancel) renews its lease at once.
//! - [`SimNode::lease_advance`] moves the simulated clock. A caller silent for longer than the
//!   period loses its lease, unless it has a run in flight: a caller with a run going is never
//!   dropped (Lemnos M39, fce51daf). Its lease is renewed while its run is in flight.
//! - A caller that loses its lease has its holds released. Its next reply's detail begins with
//!   [`LEASE_ENDED_NOTE`] (once; Lemnos M39-TCP, `LEASE_ENDED_NOTE` in calls.rs).
//! - A stalled run (the stall behaviour of a script, the only transfer-like state this node has)
//!   ends when it has been stalled for longer than the stall time. Files are not modelled here.
//! - The defaults are the locked bridge's: [`PERIOD`] 60 s and [`STALL`] 600 s.
//!   [`SimNode::lease_set`] shortens them for a test.
//!
//! Nothing here sleeps: a drop is applied on the node's own thread at its next tick, and each
//! drop is counted so a test can wait for it without a real delay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

/// The lease period of the locked bridge.
// board: Lemnos 214c0ca7 service/orion/src/config.rs:224 (LEASE, the default when LEMNOS_ORION_LEASE is unset).
pub const PERIOD: Duration = Duration::from_secs(60);
/// The stall time of the locked bridge's transfers.
// board: Lemnos 214c0ca7 service/orion/src/files.rs:40 (IDLE, 600 s: a file with no traffic is given up).
pub const STALL: Duration = Duration::from_secs(600);

/// What the node must end, as a drop the lease decided.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    /// Every running action run of this caller.
    Caller(String),
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
    period: Duration,
    stall: Duration,
    /// The simulated time of each caller's last request.
    last: BTreeMap<String, Duration>,
    /// The simulated time each run was stalled at.
    stalled: BTreeMap<u64, Duration>,
    /// Callers whose lease ended and who have had no reply since.
    ended: BTreeSet<String>,
    pending: Vec<Drop>,
    dropped: usize,
}

impl Leases {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                now: Duration::ZERO,
                period: PERIOD,
                stall: STALL,
                last: BTreeMap::new(),
                stalled: BTreeMap::new(),
                ended: BTreeSet::new(),
                pending: Vec::new(),
                dropped: 0,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// A request from `caller` reached the node: its lease starts again from now.
    pub(crate) fn request(&self, caller: &str) {
        let mut state = self.state();
        let now = state.now;
        state.last.insert(caller.to_owned(), now);
    }

    /// Run `call` stalled now.
    pub(crate) fn stalled(&self, call: u64) {
        let mut state = self.state();
        let now = state.now;
        state.stalled.insert(call, now);
    }

    /// Moves the simulated clock by `by` and decides the stall drops: every stalled run stalled for
    /// longer than the stall time. Silent callers are decided by [`Leases::expire`], which knows
    /// what is in flight.
    pub(crate) fn advance(&self, by: Duration) {
        let mut state = self.state();
        state.now += by;
        let now = state.now;
        let stall = state.stall;
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

    /// Decides the caller drops. A caller silent for longer than the period loses its lease, except
    /// a caller in `busy` (a run of its is in flight): that lease is renewed from now instead. A
    /// dropped caller's next reply carries [`LEASE_ENDED_NOTE`].
    pub(crate) fn expire(&self, busy: &BTreeSet<String>) {
        let mut state = self.state();
        let now = state.now;
        let period = state.period;
        for caller in busy {
            state.last.insert(caller.clone(), now);
        }
        let silent: Vec<String> = state
            .last
            .iter()
            .filter(|(_, last)| now.saturating_sub(**last) >= period)
            .map(|(caller, _)| caller.clone())
            .collect();
        for caller in silent {
            state.last.remove(&caller);
            state.ended.insert(caller.clone());
            state.pending.push(Drop::Caller(caller));
        }
    }

    /// Whether `caller`'s lease ended since its last reply: the reply it gets now carries the note.
    /// The note is given once.
    pub(crate) fn take_note(&self, caller: &str) -> bool {
        self.state().ended.remove(caller)
    }

    /// The lease period in whole seconds, as the bridge writes it in a describe record.
    pub(crate) fn period_secs(&self) -> u64 {
        self.state().period.as_secs()
    }

    /// Sets the period and the stall time (a test's shorter ones).
    pub(crate) fn set(&self, period: Duration, stall: Duration) {
        let mut state = self.state();
        state.period = period;
        state.stall = stall;
    }

    /// The drops decided since the last call. Counted as they are taken.
    pub(crate) fn take(&self) -> Vec<Drop> {
        let mut state = self.state();
        let pending = std::mem::take(&mut state.pending);
        state.dropped += pending.len();
        pending
    }

    /// How many drops have been applied in all.
    pub(crate) fn dropped(&self) -> usize {
        let state = self.state();
        state.dropped + state.pending.len()
    }
}

/// The note a caller's first reply after its lease ended carries, at the start of its detail (the
/// bridge's `LEASE_ENDED_NOTE`, Lemnos M39-TCP). Matched by its prefix `lease-ended:`; the rest is
/// free text. A typed reply field replaces the prefix later (M40).
// board: Lemnos 214c0ca7 service/orion/src/calls.rs:473 (LEASE_ENDED_NOTE, the same text).
pub const LEASE_ENDED_NOTE: &str = "lease-ended: holds released";

/// A reply's detail with the note in front: the note alone when the detail is empty, else the note,
/// `; `, and the detail (the bridge's `noted`).
pub(crate) fn with_note(detail: &str) -> String {
    if detail.is_empty() {
        LEASE_ENDED_NOTE.to_owned()
    } else {
        format!("{LEASE_ENDED_NOTE}; {detail}")
    }
}
/// What a stalled run ends with when the stall time ran out.
pub(crate) const STALL_WORDS: &str = "the stall time ran out: this run was stalled for longer than the board allows, and was dropped";
