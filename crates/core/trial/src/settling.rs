//! What the device list shows while a trial is pending on a board.
//!
//! While the board's own `trial` record is set, the device shows "update still settling" beside
//! its name, and its health does not turn red. Power cycles during a trial are normal: the board
//! goes away and comes back, and absence is not a fault (the same rule as a robot's power cycle).
//! The predicate is a pure function of the last slot list and the device's presence.

use atlas_inventory::Health;
use atlas_slots::SlotList;

use crate::watch::Seen;

/// Whether the device shows "update still settling": the board's last slot list has a pending
/// `trial`, and the device at this address is still the board the trial was started on. A
/// [`Seen::Replaced`] device is another board, so nothing of this trial is shown on it.
#[must_use]
pub fn settling(list: Option<&SlotList>, seen: &Seen) -> bool {
    !matches!(seen, Seen::Replaced { .. }) && list.is_some_and(|list| list.trial.is_some())
}

/// The health the device list shows. While the device is settling, a fault is shown as away: a
/// trial's restarts are not a fault. Ok and away are unchanged.
#[must_use]
pub const fn health_while_settling(health: Health, settling: bool) -> Health {
    match (health, settling) {
        (Health::Fault, true) => Health::Away,
        (health, _) => health,
    }
}
