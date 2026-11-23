//! How long a push may wait on the board for its answer, and what a job allows for that.
//!
//! A push's commit is the board's check of the whole file (its digest), which takes it as long as
//! the file is large. The session waits for that answer as long as the node is heard from, up to
//! [`BOARD_COMMIT_CEILING`], and no job step is given less than the ceiling plus a margin, so an
//! Atlas job never ends before the board's own bound does.

use std::time::Duration;

use atlas_jobs::JobTiming;
use atlas_session::{AnswerBound, words};

/// The longest a push waits for the board to answer one request (its commit above all) while the
/// board keeps answering pings. Mirrors the backstop of Lemnos's job wait,
/// `service/orion/src/stream.rs:48` at Lemnos 6c8568d0 (`JOB_WAIT`, "a backstop, not a pace", 900 s
/// there as at a6251ecf and d9bea297). It is a sanity ceiling, not the pace: a board that answers within it
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

/// How many of the session's bounds on one answer Orion's push gives the board's commit when no
/// ceiling is set: "twelve timeouts" (LOCK: Orion b029f602, `crates/client/files/src/push.rs:397`,
/// `self.timeout * 12` under `Reply::Timeout`).
pub const COMMIT_WAIT_IN_BOUNDS: u32 = 12;

/// The words for a push whose wait for one of the board's answers timed out, naming the bound that
/// ran out and its value. `sent` of `total` bytes were held when it ran out: all of them means the
/// board's answer to the commit was the one waited for, otherwise the push did not have all its
/// bytes in yet.
///
/// - [`AnswerBound::Alive`]: the board was still heard from, and one answer took longer than the
///   commit ceiling ([`BOARD_COMMIT_CEILING`], unless the session set another).
/// - [`AnswerBound::Each`]: the session's own bound on each answer ran out (a step that waits under
///   the short bound, not the ceiling).
#[must_use]
pub fn ceiling_reason(sent: u64, total: u64, bound: AnswerBound) -> String {
    match bound {
        AnswerBound::Alive { ceiling } => {
            let ceiling = words::duration(ceiling);
            if sent >= total {
                format!(
                    "the board was still answering, but its commit of all {total} bytes took longer than the commit ceiling of {ceiling}; the push ended without the board's answer"
                )
            } else {
                format!(
                    "the board was still answering, but one of its answers took longer than the commit ceiling of {ceiling} before all {total} bytes were in ({sent} held)"
                )
            }
        }
        AnswerBound::Each(bound) => {
            let each = words::duration(bound);
            if sent >= total {
                let commit = words::duration(bound.saturating_mul(COMMIT_WAIT_IN_BOUNDS));
                format!(
                    "the board did not answer the commit of all {total} bytes within {commit} ({COMMIT_WAIT_IN_BOUNDS} times {each}, the bound on each answer); the push ended without the board's answer"
                )
            } else {
                format!(
                    "the board did not answer within {each}, the bound on each answer, before all {total} bytes were in ({sent} held)"
                )
            }
        }
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
    fn the_words_name_the_bound_that_ran_out_and_its_value() {
        let alive = AnswerBound::Alive {
            ceiling: BOARD_COMMIT_CEILING,
        };
        let commit = ceiling_reason(10, 10, alive);
        assert!(commit.contains("commit ceiling of 15 min"), "{commit}");
        assert!(commit.contains("still answering"), "{commit}");
        assert!(commit.contains("commit of all 10 bytes"), "{commit}");
        let bytes = ceiling_reason(4, 10, alive);
        assert!(bytes.contains("commit ceiling of 15 min"), "{bytes}");
        assert!(bytes.contains("before all 10 bytes"), "{bytes}");
        assert!(!bytes.contains("timed out"), "{bytes}");
        // The session's short bound ran out, not the ceiling: the words say so, with its value.
        let each = AnswerBound::Each(Duration::from_millis(400));
        let short = ceiling_reason(10, 10, each);
        assert!(
            short.contains("within 4.8 s (12 times 400 ms, the bound on each answer)"),
            "{short}"
        );
        assert!(
            !short.contains("15 min") && !short.contains("ceiling"),
            "{short}"
        );
        let early = ceiling_reason(4, 10, each);
        assert!(
            early.contains("within 400 ms") && early.contains("(4 held)"),
            "{early}"
        );
    }
}
