//! This computer's SSH key, added to a board once it runs after a fresh
//! install. A flash that can't put the key on the boot partition itself (the
//! raze-flasher path writes the image and nothing else) leaves it to this:
//! when the board comes back running and offers `ssh-key.add`, Atlas runs it,
//! over the board's USB console when it is plugged in here, so no key is
//! needed to add one.

use atlas_driver::{DeviceKey, DeviceMode, DriverError};

use crate::atlas::Inner;
use crate::selftest::{AFTER_UPDATE_WINDOW, board_of};
use crate::time::now_ms;
use crate::{Atlas, CoreError, DeviceRecord};

/// The board action that adds this computer's key.
pub const SSH_KEY_ADD: &str = "ssh-key.add";

impl Inner {
    /// After a verified fresh install: add the key once the board runs.
    pub(crate) fn queue_key_install(&self, record: &DeviceRecord) {
        let deadline = now_ms() + AFTER_UPDATE_WINDOW.as_millis() as u64;
        self.state().key_pending.insert(board_of(record), deadline);
    }
}

impl Atlas {
    /// Adds the key to the boards that are due: freshly installed, now
    /// running and offering it. One that can't be reached yet is tried
    /// again on a later scan until the window closes.
    pub(crate) fn start_due_key_installs(&self) {
        let now = now_ms();
        let due: Vec<(DeviceKey, String, u64)> = {
            let mut state = self.inner.state();
            state.key_pending.retain(|_, deadline| *deadline > now);
            if state.key_pending.is_empty() {
                return;
            }
            let mut due = Vec::new();
            for (key, live) in &state.live {
                let Some(record) = state.inventory.get(key) else {
                    continue;
                };
                let offers = live.capabilities.actions.as_ref().is_some_and(|actions| {
                    actions
                        .actions(&record.identity)
                        .iter()
                        .any(|action| action.id == SSH_KEY_ADD)
                });
                if !offers || record.identity.mode != DeviceMode::Normal {
                    continue;
                }
                let board = board_of(record);
                if let Some(deadline) = state.key_pending.get(&board) {
                    due.push((key.clone(), board, *deadline));
                }
            }
            for (_, board, _) in &due {
                state.key_pending.remove(board);
            }
            due
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for (key, board, deadline) in due {
            let atlas = self.clone();
            runtime.spawn(async move {
                if let Err(CoreError::Driver(DriverError::Unreachable(_))) =
                    atlas.run_action(&key, SSH_KEY_ADD).await
                    && now_ms() < deadline
                {
                    atlas.inner.state().key_pending.insert(board, deadline);
                }
            });
        }
    }
}
