//! Device self-tests: run on request, and once by itself after a board was
//! flashed or updated, when it comes back running. The last result per
//! physical board (keyed by its board serial) is kept with the inventory and
//! shown in the device's History.

use std::time::Duration;

use atlas_driver::{
    DeviceKey, DeviceMode, DriverError, SelfTestReport,
    attributes::{BOARD_SERIAL, normalize_board_serial},
};
use serde::{Deserialize, Serialize};

use crate::activity::{ActivityEntry, ActivityKind, ActivityLevel};
use crate::atlas::Inner;
use crate::time::now_ms;
use crate::{Atlas, CoreError, DeviceRecord, Event};

/// How long after a flash or update Atlas waits for the board to come back
/// with its self-test before giving up on running it.
pub(crate) const AFTER_UPDATE_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelfTestTrigger {
    /// Someone asked for it.
    Manual,
    /// Atlas ran it when the board came back from a flash or an update.
    AfterUpdate,
}

/// One self-test run, as Atlas keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfTestRecord {
    /// The board serial results are kept by (Atlas's rule: the last 8 hex
    /// digits), the same in USB boot and running.
    pub board_serial: String,
    /// The device it ran on.
    pub device: DeviceKey,
    pub at_ms: u64,
    pub trigger: SelfTestTrigger,
    /// What the board reported; `None` when the run itself failed.
    #[serde(default)]
    pub report: Option<SelfTestReport>,
    /// Why the run failed (SSH, timeout, no report).
    #[serde(default)]
    pub error: Option<String>,
}

impl SelfTestRecord {
    /// Every check ran and none failed.
    pub fn passed(&self) -> bool {
        self.report.as_ref().is_some_and(|report| report.ok)
    }
}

/// The board serial a device's results are kept under.
pub(crate) fn board_of(record: &DeviceRecord) -> String {
    record
        .identity
        .attributes
        .get(BOARD_SERIAL)
        .cloned()
        .or_else(|| normalize_board_serial(&record.key.serial.0))
        .unwrap_or_else(|| record.key.serial.0.to_ascii_lowercase())
}

fn activity_for(record: &DeviceRecord, run: &SelfTestRecord) -> ActivityEntry {
    let name = record.display_name();
    let after = match run.trigger {
        SelfTestTrigger::AfterUpdate => " after its update",
        SelfTestTrigger::Manual => "",
    };
    let (level, message) = match (&run.report, &run.error) {
        (Some(report), _) if report.ok => (
            ActivityLevel::Success,
            format!("{name} passed its self-test{after}: {}", report.summary()),
        ),
        (Some(report), _) => (
            ActivityLevel::Warning,
            format!("{name} failed its self-test{after}: {}", report.summary()),
        ),
        (None, error) => (
            ActivityLevel::Warning,
            format!(
                "{name}'s self-test{after} couldn't run: {}",
                error.as_deref().unwrap_or("no report")
            ),
        ),
    };
    ActivityEntry::about(record, ActivityKind::SelfTest, level, message)
}

impl Inner {
    /// After a verified flash or update: run the self-test once the board
    /// is back, running, with one.
    pub(crate) fn queue_selftest(&self, record: &DeviceRecord) {
        let deadline = now_ms() + AFTER_UPDATE_WINDOW.as_millis() as u64;
        self.state()
            .selftest_pending
            .insert(board_of(record), deadline);
    }

    /// Keeps a finished run, records it in the history and tells the hosts.
    fn finish_selftest(&self, record: &DeviceRecord, run: SelfTestRecord) {
        self.state()
            .selftests
            .insert(run.board_serial.clone(), run.clone());
        self.record_activity(vec![activity_for(record, &run)]);
        self.events.emit(Event::SelfTest {
            record: Box::new(run),
        });
        self.persist();
    }
}

impl Atlas {
    /// Runs a device's self-test now. The result is kept and returned also
    /// when the board's checks failed or the run couldn't finish; an error
    /// means it didn't start.
    pub async fn run_selftest(&self, key: &DeviceKey) -> Result<SelfTestRecord, CoreError> {
        self.selftest_now(key, SelfTestTrigger::Manual)
            .await
            .map(|(record, _)| record)
    }

    /// The last self-test of this device's board, whichever way the board
    /// was seen when it ran.
    pub fn selftest(&self, key: &DeviceKey) -> Option<SelfTestRecord> {
        let state = self.inner.state();
        let record = state.inventory.get(key)?;
        state.selftests.get(&board_of(record)).cloned()
    }

    /// The last self-test of every board, newest first.
    pub fn selftests(&self) -> Vec<SelfTestRecord> {
        let mut all: Vec<SelfTestRecord> = self.inner.state().selftests.values().cloned().collect();
        all.sort_by_key(|run| std::cmp::Reverse(run.at_ms));
        all
    }

    /// Runs the test and keeps the result. Also says whether the board
    /// wasn't reachable, so an automatic run can try again.
    async fn selftest_now(
        &self,
        key: &DeviceKey,
        trigger: SelfTestTrigger,
    ) -> Result<(SelfTestRecord, bool), CoreError> {
        let (live, record) = self.live_device(key)?;
        let selftest = live
            .capabilities
            .selftest
            .ok_or_else(|| CoreError::Unsupported {
                device: key.clone(),
                what: "a self-test",
            })?;
        let board = board_of(&record);
        if !self.inner.state().selftest_running.insert(board.clone()) {
            return Err(CoreError::SelfTestRunning(key.clone()));
        }
        let result = selftest.run_selftest(&record.identity).await;
        self.inner.state().selftest_running.remove(&board);
        let unreachable = matches!(result, Err(DriverError::Unreachable(_)));
        let run = SelfTestRecord {
            board_serial: board,
            device: key.clone(),
            at_ms: now_ms(),
            trigger,
            error: result.as_ref().err().map(ToString::to_string),
            report: result.ok(),
        };
        if !(unreachable && trigger == SelfTestTrigger::AfterUpdate) {
            self.inner.finish_selftest(&record, run.clone());
        }
        Ok((run, unreachable))
    }

    /// Starts the self-tests that are due: boards flashed or updated
    /// recently that are now running with a self-test. A board that can't be
    /// reached yet (SSH still starting) is tried again on a later scan until
    /// the window closes; then the failure is recorded.
    pub(crate) fn start_due_selftests(&self) {
        let now = now_ms();
        let due: Vec<(DeviceKey, String, u64)> = {
            let mut state = self.inner.state();
            state.selftest_pending.retain(|_, deadline| *deadline > now);
            if state.selftest_pending.is_empty() {
                return;
            }
            let mut due = Vec::new();
            for (key, live) in &state.live {
                let Some(record) = state.inventory.get(key) else {
                    continue;
                };
                if live.capabilities.selftest.is_none()
                    || record.identity.mode != DeviceMode::Normal
                {
                    continue;
                }
                let board = board_of(record);
                if let Some(deadline) = state.selftest_pending.get(&board) {
                    due.push((key.clone(), board, *deadline));
                }
            }
            for (_, board, _) in &due {
                state.selftest_pending.remove(board);
            }
            due
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        for (key, board, deadline) in due {
            let atlas = self.clone();
            runtime.spawn(async move {
                match atlas.selftest_now(&key, SelfTestTrigger::AfterUpdate).await {
                    Ok((_, true)) if now_ms() < deadline => {
                        atlas.inner.state().selftest_pending.insert(board, deadline);
                    }
                    Ok((run, true)) => {
                        // The window closed: show why it never ran.
                        if let Some(record) = atlas.device(&key) {
                            atlas.inner.finish_selftest(&record, run);
                        }
                    }
                    Ok(_) | Err(_) => {}
                }
            });
        }
    }
}
