//! The device description call: one function, [`fetch`], that asks a device for its description
//! through the node's session, and the rule for when a description is asked for again.
//!
//! The rule is "fetch once, then follow": a device is asked when this connection first sees it,
//! and again only when its description generation moves (its `<device>.description` topic, the
//! number the description carries). Nothing is asked on a timer, and nothing is asked again while
//! the generation is the same. A manual refresh asks every device again.
//!
//! Paging: Lemnos's `describe` has no paged form at e5c0ff0e (a description over 60 KiB is refused
//! whole by the bridge, by name). When it gets one (Lemnos's `list_page` shape, as M8b gives
//! `system.slots`), [`fetch`] is the one place that changes: it walks the pages and returns the
//! whole text.

use std::io::ErrorKind;
use std::time::Duration;

use orion::actions::{ActionCall, Outcome, Target};
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError, Orion};

use crate::description::DescriptionState;

/// The argument of the call: the word alone. The bridge refuses any other argument to `describe`.
pub const DESCRIBE_ARGS: &[u8] = b"describe";
/// How long the bridge has to answer one description. Reason: a description is read from lemnosd
/// in one round trip; a device that does not answer in this time is cancelled, not waited for.
pub const DESCRIBE_WAIT: Duration = Duration::from_secs(10);

// LOCK: Lemnos M8b's paged form (`list_page`: a cursor in, a `next` cursor out) for `describe`: not
// at e5c0ff0e. The bridge refuses a description over 60 KiB whole, by name; this reader never sees
// a cut text, so nothing here walks pages yet.

/// What the reader holds of a device's description: its state, and the generation it answers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asked {
    /// The description as the last ask left it.
    pub state: DescriptionState,
    /// The generation the state answers: the description's own, or, for a refusal, the topic's
    /// generation at the time of the ask.
    pub generation: Option<u64>,
}

/// Whether a device's description is asked for now. It is asked when nothing is held for the
/// device on this connection, and again when the generation its topic publishes now is not the
/// one held. A topic with no sample is not a change.
#[must_use]
pub(crate) fn is_due(held: Option<&Asked>, topic_generation: Option<u64>) -> bool {
    match held {
        None => true,
        Some(asked) => match topic_generation {
            Some(now) => Some(now) != asked.generation,
            None => false,
        },
    }
}

/// Asks device `device` (a resource name, `lemnos.<device>`) on `node` for its description.
///
/// The outer `Err` is the session. The inner `Err` is the board's refusal, in its own words: a
/// board that does not answer `describe`, a device that is not running, or a reply the bridge
/// refused for its size. A reply is never truncated.
///
/// # Errors
///
/// [`OrionError`] when the session is gone.
pub(crate) fn fetch(
    orion: &Orion,
    device: &str,
    node: &NodeName,
) -> Result<Result<Vec<u8>, String>, OrionError> {
    // LOCK: Lemnos I6h describe (`lemnos.<device>.call describe`, e5c0ff0e). At our lock (d6b9361d)
    // the bridge does not answer describe: the call is refused and the device is shown with
    // "description not available", never with labels or an empty device.
    let name = format!("{device}.call");
    let mut run = orion.run(&ActionCall {
        name: &name,
        target: Target::Node(node.clone()),
        args: DESCRIBE_ARGS,
        deadline_ms: 0,
        detached: false,
    })?;
    match run.finish(DESCRIBE_WAIT) {
        Ok(finished) => Ok(match finished.outcome {
            Outcome::Succeeded => Ok(finished.output),
            _ if finished.detail.trim().is_empty() => {
                Err("the board did not give the description".to_owned())
            }
            _ => Err(finished.detail),
        }),
        Err(ClientError::Refused(error)) => Ok(Err(error.to_string())),
        Err(ClientError::Io(error)) if error.kind() == ErrorKind::TimedOut => {
            // Not finished in time: asked to stop, so it is not left running unobserved.
            let _ = run.cancel();
            Ok(Err(format!(
                "the board did not answer describe within {} s",
                DESCRIBE_WAIT.as_secs()
            )))
        }
        Err(error) => Err(OrionError::Client(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_never_asked_is_due_and_a_held_one_is_due_only_when_the_generation_moves() {
        let held = Asked {
            state: DescriptionState::Pending,
            generation: Some(4),
        };
        assert!(is_due(None, None));
        assert!(!is_due(Some(&held), Some(4)), "same generation: no ask");
        assert!(is_due(Some(&held), Some(5)), "moved: one ask");
        assert!(!is_due(Some(&held), None), "no sample is not a change");
        let unknown = Asked {
            state: DescriptionState::NotAvailable("off".into()),
            generation: None,
        };
        assert!(
            is_due(Some(&unknown), Some(1)),
            "a description with no generation follows the topic"
        );
    }
}
