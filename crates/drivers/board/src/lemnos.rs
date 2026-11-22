//! Whether a node's Lemnos answers. Lemnos is the board's own program: a node that runs it and
//! stops answering is faulted and restarting. A node that never ran it is a plain Orion node,
//! which is normal. The four states are [`LemnosState`] on the identity, not text in an error.

use atlas_session::lemnos;
use atlas_types::LemnosState;
use orion::Orion;
use orion::actions::{ActionCall, Outcome};
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

/// What one probe found: the state, and whether it is final. A final state is kept for the connection;
/// a state that is not final (the device is not answering yet, or the search did not finish) is asked
/// again on the next read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The state the probe means.
    pub state: LemnosState,
    /// Whether the state is final: `Answering`, a revision mismatch, or a board with no such contract
    /// from a complete answer.
    pub settled: bool,
}

/// Asks Lemnos its read-only question: `system.slots/1`'s `list`, on the device that carries that
/// contract (the names are Lemnos's, not a board's). A node with no
/// such device is not answering, and nothing is sent. A session that breaks during it is an error;
/// any other failure is Lemnos not answering, with the reason, and is not final.
///
/// # Errors
///
/// [`orion::Error`] when the session is gone.
pub fn probe(orion: &Orion, session: u64) -> Result<Probe, orion::Error> {
    let resource = match lemnos::resource_for(orion, session, lemnos::SLOTS)? {
        lemnos::Lookup::Found(resource) => resource,
        // A complete answer: no device carries the contract. That is final.
        lemnos::Lookup::NoContract => {
            return Ok(Probe {
                state: LemnosState::NotAnswering {
                    reason: format!("this board has no {}", lemnos::SLOTS),
                },
                settled: true,
            });
        }
        lemnos::Lookup::NotYet(reason) => return Ok(not_yet(reason)),
        lemnos::Lookup::Unanswered(reason) => return Ok(not_yet(reason.to_string())),
        lemnos::Lookup::NoDevices => {
            return Ok(not_yet(
                "this board lists no Lemnos devices: Lemnos is up but has published none".into(),
            ));
        }
    };
    let Ok(call) = lemnos::method_call(&resource, lemnos::SLOTS, "list", b"") else {
        return Ok(Probe {
            state: LemnosState::NotAnswering {
                reason: format!("this board has no {}", lemnos::SLOTS),
            },
            settled: true,
        });
    };
    let answered = orion
        .run(&ActionCall {
            name: &call.name,
            target: call.target,
            args: &call.args,
            deadline_ms: 0,
            detached: false,
        })
        .and_then(|mut run| Ok(run.finish(PROBE_WAIT)?));
    let finished = match answered {
        Ok(finished) => finished,
        Err(error) if error.is_lost() => return Err(error),
        Err(error) => return Ok(not_yet(error.to_string())),
    };
    let state = from_probe(finished.outcome, &finished.detail);
    let settled = matches!(
        state,
        LemnosState::Answering | LemnosState::RevisionMismatch { .. }
    );
    Ok(Probe { state, settled })
}

/// A probe that is not final: Lemnos is not answering yet, with the reason.
fn not_yet(reason: String) -> Probe {
    Probe {
        state: LemnosState::NotAnswering { reason },
        settled: false,
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
