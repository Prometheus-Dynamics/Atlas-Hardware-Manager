//! A board that answers busy: how long a call waits it out, and who can see the wait.
//!
//! Orion's busy means the board had no room for the call and did not take it, so the call is sent
//! again on the failure rule's delays ([`crate::backoff`]). Two bounds keep that honest:
//!
//! - **Shown.** A call that will have waited [`SHOWN_AFTER`] is listed in the session's
//!   [`BusyWait`], which the page a person is looking at shows ("the board is busy; trying
//!   again", with how long). A person can end every such wait ([`crate::Handle::stop_busy`]).
//! - **Bounded.** A call still refused busy after [`BOUND`] with no success is handed back as the
//!   refusal it is, with the board's own words. Orion also says busy for things that are not
//!   load (a lease another holds, a file prefix another offers), and those never clear by waiting.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::watch;

/// A wait that will have lasted this long is shown.
pub const SHOWN_AFTER: Duration = Duration::from_secs(2);
/// A call refused busy for this long, with no success, is a refusal.
pub const BOUND: Duration = Duration::from_secs(30);

/// The busy wait a session shows: the oldest of its calls that are being sent again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusyWait {
    /// When that call was first answered busy.
    pub since: Instant,
    /// How long such a call is sent again before it is a refusal.
    pub bound: Duration,
}

/// The calls of one session that are waiting out a busy board.
#[derive(Debug)]
pub(crate) struct Waits {
    shown: watch::Sender<Option<BusyWait>>,
    stop: watch::Sender<u64>,
    listed: Mutex<BTreeMap<u64, BusyWait>>,
    next: AtomicU64,
}

impl Waits {
    pub(crate) fn new() -> Self {
        Self {
            shown: watch::channel(None).0,
            stop: watch::channel(0).0,
            listed: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(0),
        }
    }

    pub(crate) fn shown(&self) -> watch::Receiver<Option<BusyWait>> {
        self.shown.subscribe()
    }

    /// A receiver that changes when a person ends the waits (see [`Waits::stop`]).
    pub(crate) fn stops(&self) -> watch::Receiver<u64> {
        self.stop.subscribe()
    }

    /// Ends every wait now: each waiting call is handed back as the refusal it last got.
    pub(crate) fn stop(&self) {
        self.stop.send_modify(|count| *count += 1);
    }

    fn publish(&self, listed: &BTreeMap<u64, BusyWait>) {
        let oldest = listed.values().min_by_key(|wait| wait.since).copied();
        self.shown.send_if_modified(|shown| {
            let changed = *shown != oldest;
            *shown = oldest;
            changed
        });
    }
}

/// One call's wait. Dropping it (the call ended, or its caller stopped waiting) takes it off the list.
#[derive(Debug)]
pub(crate) struct Waiting {
    waits: Arc<Waits>,
    wait: BusyWait,
    listed: Option<u64>,
}

impl Waiting {
    pub(crate) fn begin(waits: &Arc<Waits>, bound: Duration) -> Self {
        Self {
            waits: Arc::clone(waits),
            wait: BusyWait {
                since: Instant::now(),
                bound,
            },
            listed: None,
        }
    }

    /// How long ago the call was first answered busy.
    pub(crate) fn waited(&self) -> Duration {
        self.wait.since.elapsed()
    }

    /// Lists the wait, so the session shows it.
    pub(crate) fn show(&mut self) {
        if self.listed.is_some() {
            return;
        }
        let id = self.waits.next.fetch_add(1, Ordering::Relaxed);
        let mut listed = self
            .waits
            .listed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        listed.insert(id, self.wait);
        self.waits.publish(&listed);
        self.listed = Some(id);
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if let Some(id) = self.listed {
            let mut listed = self
                .waits
                .listed
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            listed.remove(&id);
            self.waits.publish(&listed);
        }
    }
}
