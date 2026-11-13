//! The nodes Atlas keeps sessions with: one monitor per node, started on first use, and the
//! manual off that stops watching a node until it is turned on again.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use orion::Address;

use std::time::Duration;

use crate::monitor::{CALL_WAIT, Handle, Monitor, Pause};

/// The monitors of the nodes this program is watching.
#[derive(Debug)]
pub struct Sessions {
    pause: Arc<dyn Pause>,
    call_wait: Duration,
    monitors: Mutex<BTreeMap<String, Monitor>>,
    off: Mutex<BTreeSet<String>>,
}

impl Sessions {
    /// No node is watched until [`Sessions::handle`] names it. `pause` is the wait between tries.
    /// Calls wait [`CALL_WAIT`] for their answer.
    pub fn new(pause: Arc<dyn Pause>) -> Self {
        Self::with_call_wait(pause, CALL_WAIT)
    }

    /// As [`Sessions::new`], with the wait for a call's answer chosen by the caller.
    pub fn with_call_wait(pause: Arc<dyn Pause>, call_wait: Duration) -> Self {
        Self {
            pause,
            call_wait,
            monitors: Mutex::new(BTreeMap::new()),
            off: Mutex::new(BTreeSet::new()),
        }
    }

    /// The handle for the node at `address`. Its monitor starts on first use.
    ///
    /// `None` while the node is turned off. Nothing turns a node off by itself, and asking for
    /// its handle does not turn it on.
    pub fn handle(&self, address: &Address) -> Option<Handle> {
        let key = address.to_string();
        if lock(&self.off).contains(&key) {
            return None;
        }
        let mut monitors = lock(&self.monitors);
        let monitor = monitors
            .entry(key)
            .or_insert_with(|| Monitor::start(address.clone(), self.pause.clone(), self.call_wait));
        Some(monitor.handle())
    }

    /// As [`Sessions::handle`], for a node whose key the caller already knows. A monitor that
    /// starts now is pinned to `key` from its first connect: a different node at the address is
    /// refused at once. A monitor that is already running keeps the key it has.
    pub fn handle_pinned(&self, address: &Address, key: [u8; 32]) -> Option<Handle> {
        let key_text = address.to_string();
        if lock(&self.off).contains(&key_text) {
            return None;
        }
        let mut monitors = lock(&self.monitors);
        let monitor = monitors.entry(key_text).or_insert_with(|| {
            Monitor::start_pinned(address.clone(), key, self.pause.clone(), self.call_wait)
        });
        Some(monitor.handle())
    }

    /// A person's act: the node at `address` now means `key`. Its session is pinned to that key
    /// from now on, and starts fresh. Returns `false` if no monitor is watching `address`. Nothing
    /// calls this on its own.
    pub fn accept_key(&self, address: &str, key: [u8; 32]) -> bool {
        lock(&self.monitors)
            .get(address)
            .is_some_and(|monitor| monitor.accept(key))
    }

    /// The handle for a node whose monitor is already running, by its address text. `None` if
    /// it was never asked for, or is turned off.
    pub fn existing(&self, address: &str) -> Option<Handle> {
        lock(&self.monitors).get(address).map(Monitor::handle)
    }

    /// Stops watching the node at `address` until [`Sessions::turn_on`]. Its session is closed
    /// and its monitor stops.
    pub fn turn_off(&self, address: &str) {
        lock(&self.off).insert(address.to_owned());
        let stopped = lock(&self.monitors).remove(address);
        drop(stopped);
    }

    /// Watches the node at `address` again after [`Sessions::turn_off`].
    pub fn turn_on(&self, address: &str) {
        lock(&self.off).remove(address);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
