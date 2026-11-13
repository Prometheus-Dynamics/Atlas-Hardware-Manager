//! Live counts of what this layer holds open: watches, their tasks, and
//! device handles. Hotplug tests use them to prove nothing leaks.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// What is alive right now. All zero when nothing is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    /// [`crate::Watch`] handles not yet dropped or closed.
    pub watchers: usize,
    /// Background tasks still running for a watch.
    pub tasks: usize,
    /// Open device handles.
    pub devices: usize,
}

#[derive(Debug, Default)]
struct Counts {
    watchers: AtomicUsize,
    tasks: AtomicUsize,
    devices: AtomicUsize,
}

/// A shared set of counters. Buses and watches created from the same
/// `Gauges` count together.
#[derive(Clone, Debug, Default)]
pub struct Gauges {
    counts: Arc<Counts>,
}

impl Gauges {
    /// A fresh set of counters, for tests and for isolated buses.
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-wide counters the real bus uses.
    pub fn global() -> Self {
        static GLOBAL: OnceLock<Gauges> = OnceLock::new();
        GLOBAL.get_or_init(Gauges::new).clone()
    }

    /// The counts right now.
    pub fn live(&self) -> Live {
        Live {
            watchers: self.counts.watchers.load(Ordering::SeqCst),
            tasks: self.counts.tasks.load(Ordering::SeqCst),
            devices: self.counts.devices.load(Ordering::SeqCst),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    Watcher,
    Task,
    Device,
}

/// One counted item. It counts while alive and is uncounted on drop, so a
/// handle is counted exactly as long as it exists.
#[derive(Debug)]
pub(crate) struct Tracked {
    gauges: Gauges,
    kind: Kind,
}

impl Tracked {
    pub(crate) fn new(gauges: &Gauges, kind: Kind) -> Self {
        slot(&gauges.counts, kind).fetch_add(1, Ordering::SeqCst);
        Self {
            gauges: gauges.clone(),
            kind,
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        slot(&self.gauges.counts, self.kind).fetch_sub(1, Ordering::SeqCst);
    }
}

fn slot(counts: &Counts, kind: Kind) -> &AtomicUsize {
    match kind {
        Kind::Watcher => &counts.watchers,
        Kind::Task => &counts.tasks,
        Kind::Device => &counts.devices,
    }
}
