//! The lease a board keeps per Orion caller identity (Lemnos I6g), on a simulated clock.
//!
//! - A caller's request (an action run, or its cancel) renews its lease at once.
//! - [`SimNode::lease_advance`] moves the simulated clock. A caller silent for longer than the
//!   period loses its lease: its running action runs end, as the bridge's connection carries them.
//!   The bridge sees no detached flag at the locked Orion revision, so every run of the caller ends.
//! - A stalled run (the stall behaviour of a script, the only transfer-like state this node has)
//!   ends when it has been stalled for longer than the stall time. Files are not modelled here.
//! - The defaults are the locked bridge's: [`PERIOD`] 60 s and [`STALL`] 600 s.
//!   [`SimNode::lease_set`] shortens them for a test.
//!
//! Nothing here sleeps: a drop is applied on the node's own thread at its next tick, and each
//! drop is counted so a test can wait for it without a real delay.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// The lease period of the locked bridge.
// LOCK: Lemnos I6g lease period (the bridge's default, 60 s).
pub const PERIOD: Duration = Duration::from_secs(60);
/// The stall time of the locked bridge's transfers.
// LOCK: Lemnos I6g stall rule (a stalled transfer is dropped after 600 s).
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

    /// Moves the simulated clock by `by` and decides the drops: every caller silent for longer
    /// than the period, and every stalled run stalled for longer than the stall time. A caller
    /// that is dropped has no lease until its next request.
    pub(crate) fn advance(&self, by: Duration) {
        let mut state = self.state();
        state.now += by;
        let now = state.now;
        let (period, stall) = (state.period, state.stall);
        let silent: Vec<String> = state
            .last
            .iter()
            .filter(|(_, last)| now.saturating_sub(**last) > period)
            .map(|(caller, _)| caller.clone())
            .collect();
        for caller in silent {
            state.last.remove(&caller);
            state.pending.push(Drop::Caller(caller));
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

/// What a run ends with when its caller's lease ran out.
pub(crate) const CALLER_WORDS: &str = "the lease ran out: the caller was silent for longer than its lease, so the board dropped its connection and this run with it";
/// What a stalled run ends with when the stall time ran out.
pub(crate) const STALL_WORDS: &str = "the stall time ran out: this run was stalled for longer than the board allows, and was dropped";
