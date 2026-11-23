//! Following a restart the board was asked for.
//!
//! A restart takes the session, and often the program that ran the call, with it. Its answer is
//! then lost, or ends as failed ("the program that runs it went away"), even though the board did
//! as asked. Neither the answer nor its words tell the two apart, so the board's state decides: a
//! restart is seen when the session ends and the board answers again on a new one. A board whose
//! session stays up for a whole lease did not restart.

use std::time::{Duration, Instant};

use atlas_session::{Handle, Presence};

use crate::ActionError;

/// What the board did after it was asked to restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restart {
    /// The session ended and the board answered again on a new one. `away` is when the end was
    /// seen (if a change was seen in between), `back` when it answered again; both from the ask.
    Back {
        away: Option<Duration>,
        back: Duration,
    },
    /// The session stayed up for `stayed` (the quiet time given): the board did not restart.
    Stayed(Duration),
    /// The session's monitor ended, so nothing more is seen.
    Unwatched,
}

impl Restart {
    /// The outcome in words, with its times.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Back {
                away: Some(away),
                back,
            } => format!(
                "the board restarted: it went away {:.1} s after the ask and answered again after {:.1} s",
                away.as_secs_f64(),
                back.as_secs_f64()
            ),
            Self::Back { away: None, back } => format!(
                "the board restarted: it answered again on a new session after {:.1} s",
                back.as_secs_f64()
            ),
            Self::Stayed(stayed) => format!(
                "the board did not restart: its session stayed up for {:.0} s after the ask",
                stayed.as_secs_f64()
            ),
            Self::Unwatched => {
                "the board's session is no longer watched, so its restart is not known".to_owned()
            }
        }
    }
}

/// Whether a restart call's error leaves it open whether the board restarts: the call may have
/// run and its answer was lost, or it ended as failed after it was sent (a restart ends the run
/// that asked for it). A refusal, or a call that was never sent, is the call's answer.
#[must_use]
pub const fn restart_in_doubt(error: &ActionError) -> bool {
    matches!(
        error,
        ActionError::Failed(_)
            | ActionError::Unknown { .. }
            | ActionError::NotFinished { .. }
            | ActionError::Lost { .. }
            | ActionError::StartInDoubt { .. }
            | ActionError::Unconfirmed { .. }
            | ActionError::NotAnswering(_)
            | ActionError::HoldLapsed { .. }
    )
}

/// Follows the board after a restart was asked on session `asked` (the [`Handle::session`] read
/// just before the ask) at `since`, until it answers again on a new session, or its session stays
/// up for `quiet` from the ask (callers pass [`Handle::lease_period`]: a board that restarts ends
/// its session within its lease). A board that is away is waited for, without end.
pub async fn follow_restart(
    handle: &Handle,
    asked: u64,
    since: Instant,
    quiet: Duration,
) -> Restart {
    let lease = quiet;
    let mut presence = handle.watch();
    let mut away = None;
    loop {
        let found = matches!(*presence.borrow_and_update(), Presence::Found(_));
        let session = handle.session();
        if found && session != asked {
            return Restart::Back {
                away,
                back: since.elapsed(),
            };
        }
        if !found && away.is_none() {
            away = Some(since.elapsed());
        }
        let quiet = lease.saturating_sub(since.elapsed());
        if found && away.is_none() && quiet.is_zero() {
            return Restart::Stayed(since.elapsed());
        }
        let wait = tokio::time::sleep(quiet);
        tokio::pin!(wait);
        tokio::select! {
            changed = presence.changed() => {
                if changed.is_err() {
                    return Restart::Unwatched;
                }
            }
            () = &mut wait, if found && away.is_none() => {}
        }
    }
}
