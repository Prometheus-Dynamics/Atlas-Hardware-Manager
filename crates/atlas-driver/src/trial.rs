//! How an A/B update's trial that never started reads in the board's
//! update state (the Raze writer's update.json, as SSH or Orion report it),
//! so a driver waiting for the trial's outcome stops, or waits for the
//! board's own retry, instead of running into its timeout.

/// How a trial that never started reads in the writer's state.
#[derive(Debug, PartialEq, Eq)]
pub enum TrialStop {
    /// The firmware ignored the tryboot restart; the board restarts into the
    /// trial again by itself: wait for that boot.
    Retrying,
    /// The trial didn't start and won't: the update is staged again (a reset
    /// mid-shutdown, the firmware ignoring tryboot twice) or failed.
    Stopped,
}

/// After the restart: `staged` (or `error`) with a reason means the trial
/// never ran. The writer says "trying again" when it restarts by itself.
pub fn trial_stopped(state: &str, error: &str) -> Option<TrialStop> {
    if error.is_empty() || !matches!(state, "staged" | "error") {
        return None;
    }
    Some(if error.contains("trying again") {
        TrialStop::Retrying
    } else {
        TrialStop::Stopped
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trial_that_never_started_stops_the_wait_or_follows_the_retry() {
        let cut = "the restart into 3.0 was cut short (the board reset before it finished shutting down), so its trial never started; it is still staged: apply it again";
        assert_eq!(trial_stopped("staged", cut), Some(TrialStop::Stopped));
        let ignored = "the firmware ignored the tryboot restart into 3.0 (the kernel asked for it); trying again";
        assert_eq!(trial_stopped("staged", ignored), Some(TrialStop::Retrying));
        assert_eq!(
            trial_stopped("staged", ""),
            None,
            "staged before the restart, no reason"
        );
        assert_eq!(trial_stopped("trying", ignored), None);
        assert_eq!(trial_stopped("rebooting", ""), None);
    }
}
