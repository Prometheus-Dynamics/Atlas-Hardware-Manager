use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveUpdaterSession {
    pub(crate) run_id: Option<String>,
    pub(crate) mode: String,
    started_at_epoch_ms: u64,
    cancel_requested: bool,
}

pub(crate) fn normalize_updater_run_id(run_id: Option<&str>) -> Option<String> {
    run_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn normalize_updater_mode(mode: Option<&str>) -> Option<String> {
    mode.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn updater_active_session_slot() -> &'static Mutex<Option<ActiveUpdaterSession>> {
    static ACTIVE_UPDATER_SESSION: OnceLock<Mutex<Option<ActiveUpdaterSession>>> = OnceLock::new();
    ACTIVE_UPDATER_SESSION.get_or_init(|| Mutex::new(None))
}

pub(crate) fn lock_active_updater_session() -> std::sync::MutexGuard<'static, Option<ActiveUpdaterSession>> {
    updater_active_session_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn active_updater_session_snapshot() -> Option<ActiveUpdaterSession> {
    lock_active_updater_session().clone()
}

pub(crate) fn updater_session_matches_exact(
    session: &ActiveUpdaterSession,
    run_id: Option<&str>,
    mode: Option<&str>,
) -> bool {
    let requested_run_id = normalize_updater_run_id(run_id);
    let requested_mode = normalize_updater_mode(mode);

    match (session.run_id.as_deref(), requested_run_id.as_deref()) {
        (Some(active), Some(candidate)) if active == candidate => {}
        (Some(_), _) => return false,
        (None, Some(_)) => return false,
        (None, None) => {}
    }

    if let Some(mode) = requested_mode.as_deref() {
        if session.mode != mode {
            return false;
        }
    }
    true
}

pub(crate) fn updater_session_matches_cancel_target(
    session: &ActiveUpdaterSession,
    run_id: Option<&str>,
    mode: Option<&str>,
) -> bool {
    if let Some(requested_run_id) = normalize_updater_run_id(run_id).as_deref() {
        if session.run_id.as_deref() != Some(requested_run_id) {
            return false;
        }
    }
    if let Some(requested_mode) = normalize_updater_mode(mode).as_deref() {
        if session.mode != requested_mode {
            return false;
        }
    }
    true
}

pub(crate) fn is_update_cancel_requested() -> bool {
    lock_active_updater_session()
        .as_ref()
        .map(|session| session.cancel_requested)
        .unwrap_or(false)
}

pub(crate) fn is_update_cancel_requested_for_session(run_id: Option<&str>, mode: &str) -> bool {
    lock_active_updater_session()
        .as_ref()
        .filter(|session| updater_session_matches_exact(session, run_id, Some(mode)))
        .map(|session| session.cancel_requested)
        .unwrap_or(false)
}

pub(crate) fn set_update_cancel_requested(run_id: Option<&str>, mode: Option<&str>, value: bool) -> bool {
    let mut active_session = lock_active_updater_session();
    let Some(session) = active_session.as_mut() else {
        return false;
    };
    if !updater_session_matches_cancel_target(session, run_id, mode) {
        return false;
    }
    session.cancel_requested = value;
    true
}

pub(crate) fn begin_updater_job(run_id: Option<&str>, mode: &str) -> Result<(), String> {
    let normalized_mode =
        normalize_updater_mode(Some(mode)).unwrap_or_else(|| "flash".to_string());
    let normalized_run_id = normalize_updater_run_id(run_id);

    let mut active_session = lock_active_updater_session();
    if active_session.is_some() || UPDATER_JOB_ACTIVE.load(Ordering::SeqCst) {
        return Err("Another update is already running.".to_string());
    }

    clear_terminal_complete_marker_for_run(normalized_run_id.as_deref());
    *active_session = Some(ActiveUpdaterSession {
        run_id: normalized_run_id,
        mode: normalized_mode,
        started_at_epoch_ms: epoch_ms(),
        cancel_requested: false,
    });
    UPDATER_JOB_ACTIVE.store(true, Ordering::SeqCst);
    Ok(())
}

pub(crate) fn finish_updater_job(run_id: Option<&str>, mode: &str) {
    let mut active_session = lock_active_updater_session();
    let Some(session) = active_session.as_ref() else {
        return;
    };
    if !updater_session_matches_exact(session, run_id, Some(mode)) {
        return;
    }

    let completed_run_id = session.run_id.clone();
    *active_session = None;
    UPDATER_JOB_ACTIVE.store(false, Ordering::SeqCst);
    clear_terminal_complete_marker_for_run(completed_run_id.as_deref());
}

pub(crate) struct UpdateCancelGuard {
    run_id: Option<String>,
    mode: String,
}

impl UpdateCancelGuard {
    pub(crate) fn begin(run_id: Option<&str>, mode: &str) -> Self {
        let normalized_run_id = normalize_updater_run_id(run_id);
        let normalized_mode =
            normalize_updater_mode(Some(mode)).unwrap_or_else(|| "flash".to_string());
        let _ = set_update_cancel_requested(
            normalized_run_id.as_deref(),
            Some(normalized_mode.as_str()),
            false,
        );
        Self {
            run_id: normalized_run_id,
            mode: normalized_mode,
        }
    }
}

impl Drop for UpdateCancelGuard {
    fn drop(&mut self) {
        let _ = set_update_cancel_requested(
            self.run_id.as_deref(),
            Some(self.mode.as_str()),
            false,
        );
    }
}

pub(crate) fn fail_if_update_cancelled(
    app: &tauri::AppHandle,
    run_id: Option<&str>,
    mode: &str,
    step: &str,
) -> Result<(), String> {
    if !is_update_cancel_requested_for_session(run_id, mode) {
        return Ok(());
    }

    let message = "Update canceled by user.".to_string();
    emit_updater_progress(app, run_id, mode, step, "error", message.clone());
    Err(message)
}

#[cfg(test)]
mod updater_state_tests {
    use super::*;
    use std::sync::MutexGuard;

    fn sample_session(run_id: Option<&str>, mode: &str) -> ActiveUpdaterSession {
        ActiveUpdaterSession {
            run_id: run_id.map(str::to_string),
            mode: mode.to_string(),
            started_at_epoch_ms: 123,
            cancel_requested: false,
        }
    }

    fn updater_test_serial_lock() -> &'static Mutex<()> {
        static SERIAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        SERIAL_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn lock_updater_tests() -> MutexGuard<'static, ()> {
        updater_test_serial_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reset_updater_runtime_state_for_test(run_ids: &[&str]) {
        {
            let mut active_session = lock_active_updater_session();
            *active_session = None;
        }
        UPDATER_JOB_ACTIVE.store(false, Ordering::SeqCst);
        for run_id in run_ids {
            clear_terminal_complete_marker_for_run(Some(run_id));
        }
    }

    #[test]
    fn exact_match_requires_same_run_id_and_mode() {
        let session = sample_session(Some("run-a"), "flash");
        assert!(updater_session_matches_exact(
            &session,
            Some("run-a"),
            Some("flash")
        ));
        assert!(!updater_session_matches_exact(
            &session,
            Some("run-b"),
            Some("flash")
        ));
        assert!(!updater_session_matches_exact(&session, None, Some("flash")));
        assert!(!updater_session_matches_exact(
            &session,
            Some("run-a"),
            Some("mount")
        ));
    }

    #[test]
    fn exact_match_uses_mode_when_run_id_is_absent() {
        let session = sample_session(None, "mount");
        assert!(updater_session_matches_exact(&session, None, Some("mount")));
        assert!(!updater_session_matches_exact(&session, None, Some("flash")));
        assert!(!updater_session_matches_exact(
            &session,
            Some("foreign-run"),
            Some("mount")
        ));
    }

    #[test]
    fn cancel_target_allows_missing_filters_but_rejects_mismatch() {
        let session = sample_session(Some("run-a"), "ota");
        assert!(updater_session_matches_cancel_target(&session, None, None));
        assert!(updater_session_matches_cancel_target(
            &session,
            Some("run-a"),
            None
        ));
        assert!(updater_session_matches_cancel_target(
            &session,
            None,
            Some("ota")
        ));
        assert!(!updater_session_matches_cancel_target(
            &session,
            Some("run-b"),
            None
        ));
        assert!(!updater_session_matches_cancel_target(
            &session,
            None,
            Some("flash")
        ));
    }

    #[test]
    fn run_id_normalization_trims_and_drops_empty_values() {
        assert_eq!(normalize_updater_run_id(Some(" run-a ")), Some("run-a".to_string()));
        assert_eq!(normalize_updater_run_id(Some("   ")), None);
        assert_eq!(normalize_updater_run_id(None), None);
    }

    #[test]
    fn stale_finish_does_not_clear_active_newer_session() {
        let _serial = lock_updater_tests();
        reset_updater_runtime_state_for_test(&["run-a", "run-b"]);

        assert!(begin_updater_job(Some("run-a"), "flash").is_ok());
        assert!(UPDATER_JOB_ACTIVE.load(Ordering::SeqCst));
        finish_updater_job(Some("run-b"), "flash");
        assert!(UPDATER_JOB_ACTIVE.load(Ordering::SeqCst));

        finish_updater_job(Some("run-a"), "flash");
        assert!(!UPDATER_JOB_ACTIVE.load(Ordering::SeqCst));

        reset_updater_runtime_state_for_test(&["run-a", "run-b"]);
    }

    #[test]
    fn cancel_flag_is_scoped_to_matching_active_session() {
        let _serial = lock_updater_tests();
        reset_updater_runtime_state_for_test(&["run-a"]);

        assert!(begin_updater_job(Some("run-a"), "ota").is_ok());
        assert!(!set_update_cancel_requested(Some("run-b"), Some("ota"), true));
        assert!(!is_update_cancel_requested_for_session(Some("run-a"), "ota"));

        assert!(set_update_cancel_requested(Some("run-a"), Some("ota"), true));
        assert!(is_update_cancel_requested_for_session(Some("run-a"), "ota"));
        finish_updater_job(Some("run-a"), "ota");

        reset_updater_runtime_state_for_test(&["run-a"]);
    }

    #[test]
    fn cancelled_run_allows_immediate_next_run_without_restart() {
        let _serial = lock_updater_tests();
        reset_updater_runtime_state_for_test(&["run-a", "run-b"]);

        assert!(begin_updater_job(Some("run-a"), "flash").is_ok());
        assert!(set_update_cancel_requested(Some("run-a"), Some("flash"), true));
        finish_updater_job(Some("run-a"), "flash");

        assert!(begin_updater_job(Some("run-b"), "flash").is_ok());
        finish_updater_job(Some("run-b"), "flash");
        assert!(!UPDATER_JOB_ACTIVE.load(Ordering::SeqCst));

        reset_updater_runtime_state_for_test(&["run-a", "run-b"]);
    }

    #[test]
    fn panic_path_simulation_can_release_job_lock_for_followup_run() {
        let _serial = lock_updater_tests();
        reset_updater_runtime_state_for_test(&["run-panic", "run-next"]);

        assert!(begin_updater_job(Some("run-panic"), "mount").is_ok());
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("simulated updater worker panic");
        }));
        assert!(panic_result.is_err());
        finish_updater_job(Some("run-panic"), "mount");

        assert!(begin_updater_job(Some("run-next"), "mount").is_ok());
        finish_updater_job(Some("run-next"), "mount");

        reset_updater_runtime_state_for_test(&["run-panic", "run-next"]);
    }
}
