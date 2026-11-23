//! The device description call: one function, [`fetch`], that asks a device for its description
//! through the node's session, and the rule for when a description is asked for again.
//!
//! The rule is "fetch once, then follow": a device is asked when this connection first sees it,
//! and again only when its description generation moves (its `<device>.description` topic, the
//! number the description carries). Nothing is asked on a timer, and nothing is asked again while
//! the generation is the same.
//!
//! A manual refresh asks every device once. Its answers belong to the refresh, not to the
//! connection: when a read fails part way through a refresh, the answers it has received still
//! answer that refresh, and the retry asks only the devices that have not answered it, plus any
//! whose generation has moved since.
//!
//! Paging: Lemnos's `describe` has no paged form at e5c0ff0e (a description over 60 KiB is refused
//! whole by the bridge, by name). When it gets one (Lemnos's `list_page` shape, as M8b gives
//! `system.slots`), [`fetch`] is the one place that changes: it walks the pages and returns the
//! whole text.

use std::collections::BTreeMap;
use std::time::Duration;

use atlas_session::lemnos;
use orion::{Error as OrionError, Orion};

use crate::description::DescriptionState;

/// How long the bridge has to answer one description (the session's wait: see `atlas_session::lemnos`).
pub const DESCRIBE_WAIT: Duration = lemnos::DESCRIBE_WAIT;

// LOCK: Lemnos 6c8568d0 (service/orion/src/calls.rs:1138-1142): `describe` is still refused whole. The paged
// form (`list_page`, a cursor in and a `next` cursor out) exists only for `system.slots`, not for `describe`. The bridge refuses a description over 60 KiB whole, by name; this reader never sees
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

/// One refresh in progress (see [`crate::Hardware::refresh_descriptions`]): the answer each device
/// has given it so far. A session that fails and comes back takes back none of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Refresh {
    /// This refresh's number: a read serves the refresh it was started for, and no other.
    pub id: u64,
    /// The answers received in this refresh, by device resource name.
    pub answered: BTreeMap<String, Asked>,
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

/// Asks device `device` (a resource name, `lemnos.<device>`) for its description, through the session's
/// describe answers (`atlas_session::lemnos::described`): a device the session has already described is
/// not asked again, and one whose generation moved is asked after [`forget`].
///
/// The outer `Err` is the session. The inner `Err` is the board's refusal, in its own words: a
/// board that does not answer `describe`, a device that is not running, or a reply the bridge
/// could not give.
///
/// # Errors
///
/// [`OrionError`] when the session is gone.
pub(crate) fn fetch(
    orion: &Orion,
    session: u64,
    device: &str,
) -> Result<lemnos::Answer, OrionError> {
    lemnos::described(orion, session, device)
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
