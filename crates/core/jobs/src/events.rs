use atlas_types::{Candidate, CapabilitySet, Family, Identity, LinkId};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::JobRecord;

/// Everything the background work reports. Subscribers keep up by reading
/// these; a subscriber that falls behind gets a lag notice and should re-read
/// the state it shows (the scan's next round, or [`crate::Jobs::list`]).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Event {
    /// One driver's discovery on one link succeeded. `addresses` is every
    /// candidate it found there, so an empty list means none are present.
    Discovered {
        family: Family,
        link: LinkId,
        addresses: Vec<String>,
    },
    /// A candidate answered identify. The inventory consumes this.
    Identified {
        identity: Identity,
        capabilities: CapabilitySet,
    },
    /// A candidate was found but did not identify. Retried with backoff.
    Unanswered {
        candidate: Candidate,
        error: String,
        timed_out: bool,
    },
    /// A discovery failed or timed out. Retried with backoff; never given up.
    ProbeFailed {
        family: Family,
        link: LinkId,
        error: String,
        timed_out: bool,
        attempt: u32,
        retry_in_ms: u64,
    },
    /// A discovery that had been failing succeeded again.
    ProbeRecovered {
        family: Family,
        link: LinkId,
    },
    /// The scan stopped starting new calls. `None` means every family.
    ScanPaused {
        family: Option<Family>,
    },
    ScanResumed {
        family: Option<Family>,
    },
    /// A job changed. The record is the whole job, after the change was saved.
    Job {
        job: JobRecord,
    },
    /// The job list could not be saved. Work goes on; the next change saves again.
    StoreFailed {
        message: String,
    },
}

/// The event bus. Sending never waits: with no subscriber, events are dropped.
#[derive(Clone, Debug)]
pub struct Events {
    sender: broadcast::Sender<Event>,
}

impl Events {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self { sender }
    }

    pub fn emit(&self, event: Event) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}
