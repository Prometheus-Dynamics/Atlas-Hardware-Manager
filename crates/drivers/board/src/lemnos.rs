//! Whether a node's Lemnos answers. Lemnos is the board's own program: a node that runs it and
//! stops answering is faulted and restarting. A node that never ran it is a plain Orion node,
//! which is normal. The four states are [`LemnosState`] on the identity, not text in an error.

use atlas_types::LemnosState;
use orion::Orion;
use orion::actions::{ActionCall, Outcome, Target};
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

/// The state an answer to the probe means. A refusal naming a layout mismatch is its own
/// state; any other failure is not answering.
pub fn from_probe(outcome: Outcome, detail: &str) -> LemnosState {
    match outcome {
        Outcome::Succeeded => LemnosState::Answering,
        _ if detail.contains("different Lemnos revisions") => LemnosState::RevisionMismatch {
            reason: detail.to_owned(),
        },
        _ => LemnosState::NotAnswering {
            reason: detail.to_owned(),
        },
    }
}

/// Asks Lemnos its read-only question. A session that breaks during it is an error; any other
/// failure is Lemnos not answering, with the reason.
pub fn probe(orion: &Orion, node: &Node) -> Result<LemnosState, orion::Error> {
    let Ok(name) = node.name.parse() else {
        return Ok(LemnosState::NotAnswering {
            reason: format!("the node's name {:?} is not a valid name", node.name),
        });
    };
    let answered = orion
        .run(&ActionCall {
            name: PROBE,
            target: Target::Node(name),
            args: b"",
            deadline_ms: 0,
            detached: false,
        })
        .and_then(|mut run| Ok(run.finish(PROBE_WAIT)?));
    let finished = match answered {
        Ok(finished) => finished,
        Err(error) if error.is_lost() => return Err(error),
        Err(error) => {
            return Ok(LemnosState::NotAnswering {
                reason: error.to_string(),
            });
        }
    };
    Ok(from_probe(finished.outcome, &finished.detail))
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
        assert_eq!(from_probe(Outcome::Succeeded, ""), LemnosState::Answering);
    }

    #[test]
    fn a_probe_that_is_not_answered_is_not_answering_with_its_reason() {
        assert_eq!(
            from_probe(Outcome::Failed, "-4: not answering"),
            LemnosState::NotAnswering {
                reason: "-4: not answering".into()
            }
        );
    }

    #[test]
    fn a_layout_refusal_is_its_own_state_not_a_fault() {
        let state = from_probe(
            Outcome::Rejected,
            "built from different Lemnos revisions: client 1, daemon 2",
        );
        assert!(
            matches!(state, LemnosState::RevisionMismatch { .. }),
            "{state:?}"
        );
    }
}
