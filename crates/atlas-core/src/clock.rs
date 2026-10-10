//! Keeping boards on this computer's clock. A Raze has no RTC battery and
//! often no NTP over its USB link, so its clock can be hours off, and then
//! every time it records (its event log, its update state) misleads. While
//! Atlas watches, a board whose clock is off (its identity's
//! `clock_offset_s`) and that offers `set-clock` (Orion's `clock.set`, or
//! over SSH) gets this computer's time, by the [`ClockSync`] policy: at most
//! once every [`SET_EVERY`] per board, so a board that won't take it isn't
//! asked on every scan.

use std::time::Duration;

use atlas_driver::{LinkKind, attributes};
use serde::{Deserialize, Serialize};

use crate::Atlas;
use crate::selftest::board_of;
use crate::time::now_ms;

/// Which boards get this computer's time when their clock is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockSync {
    /// Boards attached over USB (the default: nothing else sets their time).
    #[default]
    Usb,
    /// Every board Atlas reaches.
    All,
    /// None: offered as a control only.
    Off,
}

pub(crate) const SET_EVERY: Duration = Duration::from_secs(600);
/// The action that sets a board's clock (atlas-driver-board, Orion).
const SET_CLOCK: &str = "set-clock";

impl Atlas {
    /// After a scan, while watching: sets the clock of the boards `policy`
    /// covers whose clock is off.
    pub(crate) fn sync_clocks(&self, policy: ClockSync) {
        if policy == ClockSync::Off {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let now = now_ms();
        let due: Vec<_> = {
            let mut state = self.inner.state();
            let candidates: Vec<_> = state
                .live
                .iter()
                .filter_map(|(key, live)| {
                    let record = state.inventory.get(key)?;
                    let off = record
                        .identity
                        .attributes
                        .contains_key(attributes::CLOCK_OFFSET_S);
                    let covered = policy == ClockSync::All
                        || matches!(record.link_kind, LinkKind::UsbNetwork);
                    let offers = live.capabilities.actions.as_ref().is_some_and(|actions| {
                        actions
                            .actions(&record.identity)
                            .iter()
                            .any(|action| action.id == SET_CLOCK)
                    });
                    (off && covered && offers).then(|| (key.clone(), board_of(record)))
                })
                .collect();
            let due: Vec<_> = candidates
                .into_iter()
                .filter(|(_, board)| {
                    let last = state.clock_set_at.get(board).copied().unwrap_or(0);
                    now.saturating_sub(last) >= SET_EVERY.as_millis() as u64
                })
                .collect();
            for (_, board) in &due {
                state.clock_set_at.insert(board.clone(), now);
            }
            due.into_iter().map(|(key, _)| key).collect()
        };
        for key in due {
            let atlas = self.clone();
            runtime.spawn(async move {
                // Logged in the fleet history like any action; a rescan then
                // drops the offset.
                if atlas.run_action(&key, SET_CLOCK).await.is_ok() {
                    atlas.inner.changes.notify_one();
                }
            });
        }
    }
}
