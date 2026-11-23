//! `atlas-next hold <address> <seconds>`: a bench check that the session's hold keeps a board's lease
//! alive. It takes the hold through `Handle::hold`, the same hold the app's terminal and jobs use, sends
//! nothing else, and prints each lease renewal and any lapse.
//!
//! `ATLAS_BENCH_NO_RENEW=1` gives the session a lease that renews once and never again
//! (`Lease::keep_renewing` off), so a hold that runs past the board's lease shows the lapse. This is the
//! only place that variable is read.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use atlas_session::{Handle, Lease, Renewals};
use orion::Address;

use crate::Env;
use crate::commands::{Refusal, load_pins};
use crate::link;
use crate::report::Report;

/// The variable that makes the session renew once and then never (read only here).
const NO_RENEW: &str = "ATLAS_BENCH_NO_RENEW";

/// How often the session's renewal record and lapse count are read while the hold is held.
const POLL: Duration = Duration::from_millis(100);

/// What the hold saw over its run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub renewed: u64,
    pub failed: u64,
    pub lapses: u64,
}

/// Runs `hold` on the pinned board at `address` for `seconds`, and returns what the lease did as a
/// refusal when it was not kept (a lapse, or no renewal that reached the board).
pub async fn run<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    seconds: u64,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let renew_once = std::env::var(NO_RENEW).is_ok_and(|value| value == "1");
    if renew_once {
        r.say("ATLAS_BENCH_NO_RENEW: renewing once, then never (the lapse check)");
    }
    let lease = Lease {
        keep_renewing: !renew_once,
        ..Lease::DEFAULT
    };
    let pins = load_pins(state)?;
    let text = address.to_string();
    let link = link::pinned_with(env, &pins, &text, address, key, lease).await?;
    let hold = link.handle.hold();
    let tally = follow(r, &link.handle, Duration::from_secs(seconds)).await;
    drop(hold);
    r.say(format!(
        "renewed {}, failed {}, lapses {}",
        tally.renewed, tally.failed, tally.lapses
    ));
    r.field("renewed", tally.renewed);
    r.field("failed", tally.failed);
    r.field("lapses", tally.lapses);
    r.field("seconds", seconds);
    r.field("renew_once", renew_once);
    r.field("lease_seconds", link.handle.lease_period().as_secs());
    if tally.lapses > 0 {
        return Err(Refusal::new(
            1,
            "the board's lease lapsed while the hold was held: the board may have dropped the hold",
        ));
    }
    if tally.renewed == 0 {
        return Err(Refusal::new(
            1,
            "no lease renewal reached the board, so the hold was not kept alive",
        ));
    }
    Ok(())
}

/// Holds the session for `wait` and prints each renewal attempt and each lapse as it happens, with the
/// time since the hold began. A renewal attempt is printed once; if several happened between two reads
/// (not possible at the renew period), only the last is shown.
pub async fn follow<W: Write>(r: &mut Report<'_, W>, handle: &Handle, wait: Duration) -> Tally {
    let started = Instant::now();
    let renewals = handle.renewals();
    let lapses = handle.lapses();
    // The session is fresh for this command and no hold existed before this one, so its renewal record and
    // lapse count start at zero. The first renewal happens as the hold is taken, so it must not be read
    // as a baseline: it is printed like the others.
    let base = Renewals::default();
    let base_lapses = 0;
    let mut attempts_shown = 0;
    let mut lapses_shown = 0;
    let end = started + wait;
    loop {
        let counts = *renewals.borrow();
        let attempts = counts.renewed + counts.failed;
        if attempts > attempts_shown {
            attempts_shown = attempts;
            if let Some(last) = counts.last {
                let when = format!("t={:.1} s", started.elapsed().as_secs_f64());
                if last.renewed {
                    let period = handle.lease_period().as_secs();
                    r.progress(format!(
                        "{when} renewal {attempts}: renewed (board lease {period} s)"
                    ));
                } else {
                    r.progress(format!(
                        "{when} renewal {attempts}: not renewed (retried on the backoff)"
                    ));
                }
            }
        }
        let now_lapses = *lapses.borrow();
        while lapses_shown < now_lapses {
            lapses_shown += 1;
            let when = format!("t={:.1} s", started.elapsed().as_secs_f64());
            r.say(format!(
                "{when} LAPSE: Atlas sent nothing to this board's bridge for {} s, the board's lease; it may have dropped the hold",
                handle.lease_period().as_secs()
            ));
        }
        if Instant::now() >= end {
            break;
        }
        tokio::time::sleep(POLL.min(end.saturating_duration_since(Instant::now()))).await;
    }
    let counts = *renewals.borrow();
    Tally {
        renewed: counts.renewed.saturating_sub(base.renewed),
        failed: counts.failed.saturating_sub(base.failed),
        lapses: (*lapses.borrow()).saturating_sub(base_lapses),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use atlas_session::{Clock, Handle, Lease, Pause, Presence, Sessions};
    use atlas_sim_node::{Hello, Kind, Mode, SimNode};
    use orion::Address;

    use super::follow;
    use crate::report::Report;

    /// A session's wait between tries: it only yields, so a node that is gone is not spun on.
    #[derive(Debug)]
    struct NoPause;

    impl Pause for NoPause {
        fn pause(&self, _delay: Duration, _stop: &AtomicBool) {
            std::thread::yield_now();
        }
    }

    fn lemnos() -> Kind {
        Kind::WithLemnos(Hello {
            family: "test-family".into(),
            serial: "0001".into(),
            mode: Mode::Running,
            model: "test-model".into(),
            ..Hello::default()
        })
    }

    /// A connected session with a short renew period, on a manual clock the test moves.
    async fn connected(
        keep_renewing: bool,
    ) -> (SimNode, Sessions, Handle, Clock, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let node = SimNode::start_with(dir.path(), &lemnos(), None).expect("the node starts");
        let clock = Clock::manual();
        let sessions = Sessions::new(Arc::new(NoPause))
            .with_lease(Lease {
                period: Duration::from_secs(60),
                renew: Duration::from_secs(10),
                in_force: true,
                keep_renewing,
            })
            .with_clock(clock.clone());
        let address =
            Address::from_str(&format!("orion+tcp://127.0.0.1:{}", node.network().port())).unwrap();
        let handle = sessions.handle(&address).expect("the node is on");
        let mut presence = handle.watch();
        tokio::time::timeout(
            Duration::from_secs(20),
            presence.wait_for(|presence| matches!(presence, Presence::Found(_))),
        )
        .await
        .expect("the node is connected")
        .expect("the monitor is running");
        (node, sessions, handle, clock, dir)
    }

    /// The clock moves in steps while `follow` waits in real time: each step is one renew period, or
    /// `step` on the clock, `count` times, `every` apart in real time.
    fn advance(
        clock: Clock,
        step: Duration,
        count: u32,
        every: Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            for _ in 0..count {
                tokio::time::sleep(every).await;
                clock.advance(step);
            }
        })
    }

    #[tokio::test]
    async fn a_hold_over_more_than_two_periods_prints_renewals_and_is_kept() {
        let (_node, _sessions, handle, clock, _dir) = connected(true).await;
        let moving = advance(
            clock,
            Duration::from_secs(10),
            8,
            Duration::from_millis(150),
        );
        let mut out = Vec::new();
        let tally = {
            let _hold = handle.hold();
            let mut r = Report::new(&mut out, false);
            follow(&mut r, &handle, Duration::from_secs(2)).await
        };
        moving.await.expect("the clock moved");
        let text = String::from_utf8(out).expect("text");
        assert!(text.contains("renewal 1: renewed"), "{text}");
        assert!(tally.renewed >= 3, "more than two periods: {tally:?}");
        assert_eq!(tally.lapses, 0, "{text}");
        assert!(!text.contains("LAPSE"), "{text}");
    }

    #[tokio::test]
    async fn a_hold_that_renews_once_lapses_past_the_board_lease_and_says_so() {
        let (_node, _sessions, handle, clock, _dir) = connected(false).await;
        // Steps of 20 s on the clock: 120 s in all, past the 60 s lease with no renewal after the first.
        let moving = advance(
            clock,
            Duration::from_secs(20),
            6,
            Duration::from_millis(150),
        );
        let mut out = Vec::new();
        let tally = {
            let _hold = handle.hold();
            let mut r = Report::new(&mut out, false);
            follow(&mut r, &handle, Duration::from_secs(2)).await
        };
        moving.await.expect("the clock moved");
        let text = String::from_utf8(out).expect("text");
        assert_eq!(tally.renewed, 1, "renewed once, then never: {text}");
        assert!(tally.lapses >= 1, "{text}");
        assert!(text.contains("LAPSE: Atlas sent nothing"), "{text}");
        assert!(text.contains("for 60 s, the board's lease"), "{text}");
    }
}
