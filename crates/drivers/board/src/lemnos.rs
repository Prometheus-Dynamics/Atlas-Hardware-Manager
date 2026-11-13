//! Whether a node's Lemnos answers. Lemnos is the board's own program: a node that runs it and
//! stops answering is faulted and restarting. A node that never ran it is a plain Orion node,
//! which is normal. The four states are data the app shows (the identity's `lemnos` attribute),
//! not text buried in an error.

use orion::actions::Outcome;
use orion::node::Node;

/// The hello facts that say a node runs Lemnos. A node that publishes one of them, or that
/// offers anything, is expected to answer for Lemnos from then on.
const LEMNOS_FACTS: [&str; 6] = [
    "family",
    "serial",
    "offers",
    "version.lemnos",
    "version.slot",
    "version.product",
];

/// The read-only Lemnos call the driver asks a node that is expected to run Lemnos. Its answer
/// is what decides whether Lemnos answers. The name is Lemnos's contract name, not a board's.
pub const PROBE: &str = "system.slots.list";
/// How long the probe may take before it counts as not answering.
pub const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// What a node's Lemnos is doing now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lemnos {
    /// The node never published Lemnos facts or offers: a plain Orion node.
    Absent,
    /// Lemnos answered the probe.
    Answering,
    /// The node is expected to run Lemnos and Lemnos did not answer. Faulted and restarting.
    NotAnswering(String),
    /// Lemnos answered with a refusal that the layout of its client and daemon differ: the
    /// app or the board needs updating. Not a fault.
    RevisionMismatch(String),
}

impl Lemnos {
    /// The word the identity's `lemnos` attribute carries.
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Answering => "answering",
            Self::NotAnswering(_) => "not-answering",
            Self::RevisionMismatch(_) => "revision-mismatch",
        }
    }

    /// The reason behind a state that has one, shown beside it.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::NotAnswering(detail) | Self::RevisionMismatch(detail) => Some(detail),
            Self::Absent | Self::Answering => None,
        }
    }

    /// The state an answer to the probe means. A refusal naming a layout mismatch is its own
    /// state; any other failure is not answering.
    pub fn from_probe(outcome: Outcome, detail: &str) -> Self {
        match outcome {
            Outcome::Succeeded => Self::Answering,
            _ if detail.contains("different Lemnos revisions") => {
                Self::RevisionMismatch(detail.to_owned())
            }
            _ => Self::NotAnswering(detail.to_owned()),
        }
    }
}

/// Whether the hello document carries a fact that says the node runs Lemnos.
pub fn published_by(node: &Node) -> bool {
    node.facts
        .iter()
        .any(|fact| LEMNOS_FACTS.contains(&fact.name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_that_succeeds_is_answering() {
        assert_eq!(
            Lemnos::from_probe(Outcome::Succeeded, ""),
            Lemnos::Answering
        );
    }

    #[test]
    fn a_probe_that_is_not_answered_is_faulted_with_its_reason() {
        let state = Lemnos::from_probe(Outcome::Failed, "-4: not answering");
        assert_eq!(state.word(), "not-answering");
        assert_eq!(state.detail(), Some("-4: not answering"));
    }

    #[test]
    fn a_layout_refusal_is_its_own_state_not_a_fault() {
        let state = Lemnos::from_probe(
            Outcome::Rejected,
            "built from different Lemnos revisions: client 1, daemon 2",
        );
        assert_eq!(state.word(), "revision-mismatch");
    }
}
