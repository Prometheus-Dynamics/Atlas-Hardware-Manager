//! `atlas-next time <address> <seconds>`: a bench check that a board takes this computer's time.
//!
//! The session offers the time by itself when it finds the board (`atlas_session::time`), as it
//! does in the app. This command sends nothing of its own for that: it reads what became of the
//! offer, how the board's clock compares with this computer's before and after, and whether the
//! board says its clock is set, and prints each time the board asks.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use atlas_session::{Handle, TimeOffer};
use orion::Address;

use crate::Env;
use crate::commands::{Refusal, load_pins};
use crate::link;
use crate::report::Report;

/// How often the offer's record is read while the command waits.
const POLL: Duration = Duration::from_millis(100);
/// How long the offer may take to be answered before the command gives up on it.
const ANSWER: Duration = Duration::from_secs(10);

/// What the board says of its clock, and how it compares with this computer's.
struct Reading {
    /// The hello fact `clock`: `synced`, `unset`, or nothing from a board that measures no time.
    fact: Option<String>,
    /// The board's clock minus this computer's, in milliseconds.
    offset_ms: f64,
}

async fn read(handle: &Handle) -> Result<Reading, Refusal> {
    handle
        .with_client(|orion| -> Result<Reading, orion::Error> {
            let fact = orion.node()?.clock().map(str::to_owned);
            let clock = orion.clock()?;
            Ok(Reading {
                fact,
                #[allow(clippy::cast_precision_loss)]
                offset_ms: clock.offset_ns as f64 / 1e6,
            })
        })
        .await
        .map_err(|error| Refusal::new(1, format!("the board's clock could not be read: {error}")))
}

fn shown(reading: &Reading) -> String {
    format!(
        "the board says its clock is {}; it is {:+.1} ms from this computer's",
        reading.fact.as_deref().unwrap_or("not measured"),
        reading.offset_ms
    )
}

/// Reads the board's clock, waits for the session's offer, follows it for `seconds`, and reads the
/// clock again. A refusal when the board did not take the time, never asked for it, or does not
/// say `synced` at the end.
pub async fn run<W: Write>(
    r: &mut Report<'_, W>,
    env: &Env,
    state: &Path,
    address: &Address,
    seconds: u64,
    key: &[u8; 32],
) -> Result<(), Refusal> {
    let pins = load_pins(state)?;
    let text = address.to_string();
    let link = link::pinned(env, &pins, &text, address, key).await?;
    let before = read(&link.handle).await?;
    r.say(format!("before: {}", shown(&before)));
    r.field("clock_before", before.fact.clone().unwrap_or_default());
    r.field("offset_before_ms", before.offset_ms);

    let mut offer = link.handle.time_offer();
    let taken = tokio::time::timeout(
        ANSWER,
        offer.wait_for(|offer| *offer != TimeOffer::NotOffered),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .map_or(TimeOffer::NotOffered, |state| state.clone());
    let every = match taken {
        TimeOffer::Taken { every, .. } => every,
        TimeOffer::NotWanted => {
            r.field("offer", "not wanted");
            return Err(Refusal::new(
                1,
                "this board takes no time from operators (`operator` is not among its time sources, or it keeps no time)",
            ));
        }
        TimeOffer::Refused(failure) => {
            r.field("offer", "refused");
            return Err(Refusal::new(
                1,
                format!("the board refused this computer's time: {failure}"),
            ));
        }
        TimeOffer::NotOffered => {
            r.field("offer", "not answered");
            return Err(Refusal::new(
                1,
                format!(
                    "the offer of this computer's time was not answered in {} s",
                    ANSWER.as_secs()
                ),
            ));
        }
    };
    r.field("offer", "taken");
    r.field("every_seconds", every.as_secs_f64());
    r.say(format!(
        "the board takes this computer's time (stratum {}), and asks for it every {} s",
        atlas_session::time::STRATUM,
        every.as_secs_f64()
    ));

    let started = Instant::now();
    let end = started + Duration::from_secs(seconds);
    let mut asked_shown = 0;
    loop {
        let asked = match &*offer.borrow() {
            TimeOffer::Taken { asked, .. } => *asked,
            _ => {
                return Err(Refusal::new(
                    1,
                    "the session ended while the board was taking the time",
                ));
            }
        };
        if asked != asked_shown {
            asked_shown = asked;
            r.progress(format!(
                "t={:.1} s the board asked for the time ({asked})",
                started.elapsed().as_secs_f64()
            ));
        }
        if Instant::now() >= end {
            break;
        }
        tokio::time::sleep(POLL.min(end.saturating_duration_since(Instant::now()))).await;
    }
    let after = read(&link.handle).await?;
    r.say(format!("after {seconds} s: {}", shown(&after)));
    r.field("asked", asked_shown);
    r.field("clock_after", after.fact.clone().unwrap_or_default());
    r.field("offset_after_ms", after.offset_ms);
    r.field("seconds", seconds);
    if asked_shown == 0 {
        return Err(Refusal::new(
            1,
            format!("the board took the offer and did not ask for the time in {seconds} s"),
        ));
    }
    if after.fact.as_deref() != Some("synced") {
        return Err(Refusal::new(
            1,
            "the board asked for the time and does not say its clock is synced",
        ));
    }
    Ok(())
}
