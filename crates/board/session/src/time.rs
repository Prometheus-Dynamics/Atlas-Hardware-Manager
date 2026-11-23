//! The operator's time, offered to the board.
//!
//! A board with no clock battery starts with no time. It takes its time from what it is connected
//! to: when this session finds the node, it offers this computer's clock once (Orion's
//! `offer_time`), and from then on the board asks for it on this session whenever it polls. The
//! session answers as it is pumped, which the monitor does all the time, so nothing else is
//! needed for as long as the session lasts. A new session offers again.
//!
//! The offer never ends a session and never fails a call: a board that takes no time from
//! operators refuses it, and that is recorded here for whoever shows the board
//! ([`crate::Handle::time_offer`]).

use std::time::Duration;

use orion::Orion;
use orion::types::Error as CoreError;

use crate::failure::Failure;

/// The NTP stratum this computer's clock is offered at. Orion's rule is the stratum of the
/// computer's own time server plus one, and a desktop kept by a public pool (stratum 2) is 3.
/// Atlas does not ask the system whether or by what its clock is kept, so this is one fixed
/// number, the same on every system Atlas runs on.
pub const STRATUM: u8 = 3;

/// What became of this session's offer of the operator's time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TimeOffer {
    /// No session has offered yet, or the node is not connected.
    #[default]
    NotOffered,
    /// The board took the offer: it asks for the time every `every`, and has asked `asked` times
    /// on this session.
    Taken {
        /// How often the board asks.
        every: Duration,
        /// How many times it has asked so far.
        asked: u64,
    },
    /// The board takes no time from operators (Orion's `unsupported`: `operator` is not among its
    /// time sources, or it keeps no time).
    NotWanted,
    /// The board refused the offer for another reason, or did not answer it: its words.
    Refused(Failure),
}

/// Offers this computer's clock on `orion`.
pub(crate) fn offer(orion: &Orion) -> TimeOffer {
    match orion.offer_time(STRATUM) {
        Ok(every) => TimeOffer::Taken {
            every,
            asked: orion.times_asked(),
        },
        Err(error) => {
            let failure = Failure::of(&error);
            if failure.code() == Some(CoreError::Unsupported.code()) {
                TimeOffer::NotWanted
            } else {
                TimeOffer::Refused(failure)
            }
        }
    }
}

/// `was` with the count of times the board has asked brought up to date; `None` when nothing
/// changed.
pub(crate) fn asked(was: &TimeOffer, orion: &Orion) -> Option<TimeOffer> {
    let TimeOffer::Taken { every, asked } = was else {
        return None;
    };
    let now = orion.times_asked();
    (now != *asked).then_some(TimeOffer::Taken {
        every: *every,
        asked: now,
    })
}
