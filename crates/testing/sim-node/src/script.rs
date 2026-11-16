//! Scripted failures for the simulated node.
//!
//! A script is a queue per action: the next run of that action does what its entry says,
//! then the queue moves on. Anything not scripted answers normally. Link faults apply to
//! the next connection that comes through the node's link. Nothing here is timed by chance:
//! a test says exactly which run fails.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// What one run of an action does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// Runs normally.
    Answer,
    /// Refused without running: the outcome is `Rejected` and nothing changes.
    Refuse(String),
    /// Runs and fails: the outcome is `Failed` and nothing changes.
    Fail(String),
    /// Starts, then never makes progress or finishes until it is cancelled.
    Stall,
    /// The service restarts as this run starts: every running job is lost (`-4`), and this
    /// run is answered as lost too. Nothing resumes.
    RestartDaemon,
}

/// What happens to one connection through the node's link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkFault {
    /// The connection is cut once this many bytes have gone from the client to the node.
    Drop { after_bytes: u64 },
    /// Nothing moves in either direction for this long, then the link carries on.
    Stall { pause: Duration },
}

/// The queues of scripted failures.
#[derive(Clone, Debug, Default)]
pub struct Script {
    actions: BTreeMap<String, VecDeque<Behaviour>>,
    links: VecDeque<LinkFault>,
}

impl Script {
    /// Nothing scripted: everything answers normally.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `behaviour` for the next run of `action` that no earlier entry covers.
    pub fn push_action(&mut self, action: &str, behaviour: Behaviour) {
        self.actions
            .entry(action.to_string())
            .or_default()
            .push_back(behaviour);
    }

    /// Takes what the next run of `action` does.
    pub fn take_action(&mut self, action: &str) -> Behaviour {
        self.actions
            .get_mut(action)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Behaviour::Answer)
    }

    /// Queues a fault for the next connection through the link.
    pub fn push_link(&mut self, fault: LinkFault) {
        self.links.push_back(fault);
    }

    /// Takes the fault for the next connection, if one is queued.
    pub fn take_link(&mut self) -> Option<LinkFault> {
        self.links.pop_front()
    }
}
