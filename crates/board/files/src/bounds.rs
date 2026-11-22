//! How long a push may wait on the board for its answer, and what a job allows for that.
//!
//! A push's commit is the board's check of the whole file (its digest), which takes it as long as
//! the file is large. The session waits for that answer as long as the node is heard from, up to
//! [`BOARD_COMMIT_CEILING`], and no job step is given less than the ceiling plus a margin, so an
//! Atlas job never ends before the board's own bound does.

use std::time::Duration;

use atlas_jobs::JobTiming;

/// The longest a push waits for the board to answer one request (its commit above all) while the
/// board keeps answering pings. Mirrors the backstop of Lemnos's job wait,
/// `service/orion/src/stream.rs:44` (`JOB_WAIT`, "a backstop, not a pace", the same at Lemnos
/// a6251ecf and d9bea297). It is a sanity ceiling, not the pace: a board that answers within it
/// is never cut, and one that is silent for the node's idle time is lost before it.
pub const BOARD_COMMIT_CEILING: Duration = Duration::from_secs(900);

/// What a job step allows over [`BOARD_COMMIT_CEILING`], so the bound on the board ends first.
pub const STEP_MARGIN: Duration = Duration::from_secs(60);

/// The job step timeout the jobs engine uses for every job: [`BOARD_COMMIT_CEILING`] plus
/// [`STEP_MARGIN`]. Derived from the ceiling, so there is no second number for the bound.
pub const JOB_STEP_TIMEOUT: Duration = BOARD_COMMIT_CEILING.saturating_add(STEP_MARGIN);

/// The job timing the CLI and the app run jobs with: the default backoff, and a step timeout that
/// lets a push run to the board's own bound.
#[must_use]
pub fn job_timing() -> JobTiming {
    JobTiming {
        step_timeout: JOB_STEP_TIMEOUT,
        ..JobTiming::default()
    }
}

/// The words for a push whose wait for the board ran past [`BOARD_COMMIT_CEILING`] while the board
/// was still answering. `sent` of `total` bytes were held when it ran out: all of them means the
/// commit ran past the bound, otherwise the push did before all its bytes were in.
#[must_use]
pub fn ceiling_reason(sent: u64, total: u64) -> String {
    let bound = format!("{} s", BOARD_COMMIT_CEILING.as_secs());
    if sent >= total {
        format!(
            "the board was still answering, but its commit of all {total} bytes ran past {bound}; the push ended without the board's answer"
        )
    } else {
        format!(
            "the board was still answering, but the push ran past {bound} before all {total} bytes were in ({sent} held)"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_step_outlasts_the_board_bound() {
        assert!(JOB_STEP_TIMEOUT > BOARD_COMMIT_CEILING);
        assert_eq!(job_timing().step_timeout, JOB_STEP_TIMEOUT);
    }

    #[test]
    fn the_ceiling_words_name_the_bound_and_say_what_ran_past_it() {
        let commit = ceiling_reason(10, 10);
        assert!(commit.contains("900 s"), "{commit}");
        assert!(commit.contains("still answering"), "{commit}");
        assert!(commit.contains("commit"), "{commit}");
        let bytes = ceiling_reason(4, 10);
        assert!(bytes.contains("900 s"), "{bytes}");
        assert!(bytes.contains("before all 10 bytes"), "{bytes}");
        assert!(!bytes.contains("timed out"), "{bytes}");
    }
}
