//! `atlas-next cycle`: the milestone's slot cycle, run in one fixed order against one board, with
//! ONE scorecard that says what happened step by step, in the board's own words.
//!
//! The steps are the T1-T3 calls in a fixed order (see [`STEPS`]). The run stops at the first
//! unexpected state and repairs nothing: no retry, no extra command, no cancel, no remove. The
//! scorecard is written either way.
//!
//! Each step's block names how the board comes back unattended if that step fails. Those texts are
//! constants beside the steps, checked against the Lemnos texts in the milestone's inputs.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atlas_actions::{Action, BoardActions, Reported};
use atlas_board_slots::{BoardSlots, Ended, Follower, SlotError, TrialStart};
use atlas_session::Handle;
use atlas_trial::{Running, TrialState};
use orion::Address;
use orion::node::Security;
use serde_json::{Value, json};

use crate::args::CycleArgs;
use crate::commands::{self, Refusal};
use crate::identity;
use crate::link::{self, Link};
use crate::pins::Pin;
use crate::report::Report;
use crate::{Env, parse_address};

// board: Lemnos bbb3c420 drivers/system-slots/src/attempt.rs:176 (the boot id, written in `last_attempt` by `info`)
/// Whether the Lemnos this build pins publishes `info.last_attempt.boot`. It does at bbb3c420: the
/// boot id is read before and after a restart, and checked.
pub const INFO_IN_PIN: bool = true;

/// The scorecard's words for a boot id that could not be read.
const NOT_CHECKED: &str = "not checked: the board's info gives no boot id";

/// The Atlas build this tool is part of (no commit is in the build; see the PR text).
const ATLAS_BUILD: &str = env!("CARGO_PKG_VERSION");

/// One step of the cycle: its id, what it does, and how the board comes back unattended if it fails.
#[derive(Clone, Copy, Debug)]
pub struct StepInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub does: &'static str,
    pub if_it_fails: &'static str,
}

/// Lemnos's words for an install that fails (I5i9, `nothing was written, the default keeps booting`).
const INSTALL_FAILS: &str = "an install that fails writes nothing: the default keeps booting (Lemnos: \"nothing was written, the default keeps booting\").";
/// Lemnos's trial rules: `trial_attempts = 3`, `confirm_deadline_s = 600`, `reboot_at_deadline =
/// true` (INPUTS, the `system-slots` config defaults).
const TRIAL_FALLS_BACK: &str = "a trial that does not come up falls back to the default by itself: after 3 boots that never reach the daemon, or at the 600 s deadline, the board restarts into the default slot.";

/// The steps, in the order the run does them.
pub const STEPS: [StepInfo; 12] = [
    StepInfo {
        id: "1",
        title: "this PC's identity",
        does: "Reads this PC's client identity and prints its fingerprint. Makes the identity file once if it is absent.",
        if_it_fails: "Nothing on the board changes. A damaged identity file is refused and left as it is.",
    },
    StepInfo {
        id: "2",
        title: "the board's key fingerprint and security",
        does: "Reads the board's key fingerprint and its security switch, as the board's hello says them. Reads only.",
        if_it_fails: "Reads only. Nothing changes on the board.",
    },
    StepInfo {
        id: "3",
        title: "pin the board's key",
        does: "Pins the board's key for this address when it was never pinned. A different key from the pinned one stops the run.",
        if_it_fails: "The pin is kept on this PC only. A different key: nothing more is sent to the board, and there is no re-pin here (a person runs repin --reflashed after a reflash).",
    },
    StepInfo {
        id: "4",
        title: "the board's slots before",
        does: "Lists the board's slots before the cycle.",
        if_it_fails: "Reads only. Nothing changes on the board.",
    },
    StepInfo {
        id: "5",
        title: "install the good bundle",
        does: "Installs the good bundle, then checks that the board's list shows the slot it installed.",
        if_it_fails: INSTALL_FAILS,
    },
    StepInfo {
        id: "6",
        title: "trial the good slot and follow it",
        does: "Starts a trial of the good slot and follows it to its end. Passes only if the board came back in the trial slot, then checks the board's list.",
        if_it_fails: TRIAL_FALLS_BACK,
    },
    StepInfo {
        id: "7a",
        title: "install the bad bundle",
        does: "Installs the bad bundle: its slot is only ever tried, never made the default.",
        if_it_fails: INSTALL_FAILS,
    },
    StepInfo {
        id: "7b",
        title: "start the bad trial, not followed yet",
        does: "Starts a trial of the bad slot and does NOT follow it. The board restarts into it, and this run waits on purpose.",
        if_it_fails: TRIAL_FALLS_BACK,
    },
    StepInfo {
        id: "7c",
        title: "install the good slot again while the bad trial is pending",
        does: "Asks the board to install the good slot again while the bad trial is pending. Passes only if the board refuses it, naming the pending trial.",
        if_it_fails: "A refused install writes nothing, and the pending trial keeps running. If the board took the install, the run stops and the board is left as it is.",
    },
    StepInfo {
        id: "7d",
        title: "follow the bad trial to its end",
        does: "Follows the bad trial to its end with no command in between. Passes only if the board came back in the previous slot by itself.",
        if_it_fails: "The bad trial falls back to the default by itself (3 boots that never reach the daemon, or the 600 s deadline). No command is sent by this run.",
    },
    StepInfo {
        id: "8a",
        title: "confirm the good slot as the default",
        does: "Makes the good slot the default slot.",
        if_it_fails: "A refused confirm changes nothing: the default stays as it was.",
    },
    StepInfo {
        id: "8b",
        title: "the board's slots after",
        does: "Lists the board's slots after the cycle: the good slot running and default, the bad slot failed.",
        if_it_fails: "Reads only. Nothing changes on the board.",
    },
];

/// What a boot id read gave: the id, or why it was not checked. An absent field is never a pass.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BootRead {
    Read(String),
    NotChecked(String),
}

impl BootRead {
    /// The words for the scorecard: the id, or the full not-checked text.
    fn words(&self) -> String {
        match self {
            Self::Read(id) => id.clone(),
            Self::NotChecked(text) => text.clone(),
        }
    }
}

/// One recorded step.
struct Recorded {
    info: StepInfo,
    passed: bool,
    words: Vec<String>,
    elapsed: Duration,
}

/// The steps that ran, in order.
#[derive(Default)]
struct Card {
    records: Vec<Recorded>,
}

impl Card {
    /// Records one step's result, shows it, and returns the exit status when the run must stop.
    fn close<W: Write>(
        &mut self,
        info: &StepInfo,
        result: Result<(), Refusal>,
        buf: &[u8],
        began: Instant,
        out: &mut W,
    ) -> Option<i32> {
        let mut words: Vec<String> = String::from_utf8_lossy(buf)
            .lines()
            .map(str::to_owned)
            .collect();
        let elapsed = began.elapsed();
        let (passed, stop) = match result {
            Ok(()) => (true, None),
            Err(refusal) => {
                words.extend(refusal.words.lines().map(str::to_owned));
                (false, Some(if refusal.code == 3 { 3 } else { 1 }))
            }
        };
        let _ = writeln!(
            out,
            "Step {}: {} {} ({:.2} s)",
            info.id,
            if passed { "PASS" } else { "FAIL" },
            info.title,
            elapsed.as_secs_f64()
        );
        for word in &words {
            let _ = writeln!(out, "  {word}");
        }
        self.records.push(Recorded {
            info: *info,
            passed,
            words,
            elapsed,
        });
        stop
    }
}

/// The run's state: the connection, the PC's identity, the board's facts, the names the cycle
/// learns (the good and bad slot ids) and the trial follower.
struct Ctx<'a> {
    env: &'a Env,
    address: Address,
    text: String,
    state: PathBuf,
    identity_path: PathBuf,
    good_bundle: PathBuf,
    bad_bundle: PathBuf,
    key: [u8; 32],
    pc: String,
    board_key: [u8; 32],
    board_fingerprint: String,
    /// The board's security switch as its hello says it (Orion C43); None until step 2 reads it.
    security: Option<Security>,
    link: Option<Link>,
    follower: Follower,
    good: String,
    bad: String,
    boot_before_bad: Option<BootRead>,
}

impl Ctx<'_> {
    fn link(&self) -> Result<&Link, Refusal> {
        self.link
            .as_ref()
            .ok_or_else(|| Refusal::new(1, "the board is not connected: its pin was not checked"))
    }

    fn slots(&self) -> Result<BoardSlots, Refusal> {
        slots_of(self.link()?)
    }
}

/// The board's slots through its link, as the board's own offers say (see `board()` in commands).
fn slots_of(link: &Link) -> Result<BoardSlots, Refusal> {
    let info = link::board_info(&link.node);
    if !info.reported {
        return Err(Refusal::new(
            1,
            "this board did not report what it offers, so its slots are not known",
        ));
    }
    if !info.offered {
        return Err(Refusal::new(1, SlotError::NoSlots.to_string()));
    }
    Ok(BoardSlots::new(
        link.handle.clone(),
        info.family.clone().unwrap_or_default(),
        true,
    ))
}

/// The action that reads Lemnos's `system.slots` `info` (Lemnos bbb3c420). Read with no arguments.
fn info_action() -> Action {
    Action {
        name: "system.slots.info".to_owned(),
        label: Reported::NotReported,
        description: Reported::NotReported,
        args: Reported::Known(Vec::new()),
        long_running: Reported::Known(false),
    }
}

/// The board's boot id, from `info.last_attempt.boot` (Lemnos bbb3c420: present in every state). This is
/// the ONLY place the boot id is read. A build whose pin does not publish it (`INFO_IN_PIN` false) reads
/// nothing. An absent field is not checked: never a pass.
async fn boot_id(ctx: &Ctx<'_>) -> BootRead {
    if !ctx.env.info_in_pin {
        return BootRead::NotChecked(NOT_CHECKED.to_owned());
    }
    let link = match ctx.link() {
        Ok(link) => link,
        Err(_) => {
            return BootRead::NotChecked("not checked: the board is not connected".to_owned());
        }
    };
    let actions = BoardActions::new(link.handle.clone());
    match actions.run(&info_action(), &[]).await {
        Ok(done) => match serde_json::from_slice::<Value>(&done.output) {
            Ok(document) => match document
                .pointer("/last_attempt/boot")
                .and_then(Value::as_str)
                .filter(|boot| !boot.is_empty())
            {
                Some(boot) => BootRead::Read(boot.to_owned()),
                None => BootRead::NotChecked(format!(
                    "{NOT_CHECKED}: the board's info has no last_attempt.boot field"
                )),
            },
            Err(error) => BootRead::NotChecked(format!(
                "not checked: the board's info is not JSON: {error}"
            )),
        },
        Err(error) => BootRead::NotChecked(format!(
            "not checked: the board's info could not be read: {error}"
        )),
    }
}

/// Shows the boot id, and fails only when it is read both times and did not change.
fn boot_check<W: Write>(
    r: &mut Report<'_, W>,
    before: &BootRead,
    after: &BootRead,
) -> Result<(), Refusal> {
    r.say(format!("boot id before: {}", before.words()));
    r.say(format!("boot id after: {}", after.words()));
    match (before, after) {
        (BootRead::Read(first), BootRead::Read(second)) if first == second => Err(Refusal::new(
            1,
            format!("the board's boot id did not change ({first}): it did not restart"),
        )),
        (BootRead::Read(_), BootRead::Read(_)) => {
            r.say("the boot id changed: the board really restarted");
            Ok(())
        }
        _ => {
            r.say("the boot id was not checked, so the restart is judged by the slot list only");
            Ok(())
        }
    }
}

/// Checks the board's list shows `running` and `default` as given, and `failed` as failed when
/// one is named.
async fn check_list<W: Write>(
    r: &mut Report<'_, W>,
    ctx: &Ctx<'_>,
    running: &str,
    failed: Option<&str>,
) -> Result<(), Refusal> {
    let list = ctx.slots()?.list().await?;
    for line in commands::list_lines(&list) {
        r.say(line);
    }
    if list.running != running {
        return Err(Refusal::new(
            1,
            format!(
                "the board runs {}, not {running}: it did not come back in the slot the run expects",
                list.running
            ),
        ));
    }
    if list.default != running {
        return Err(Refusal::new(
            1,
            format!("the board's default is {}, not {running}", list.default),
        ));
    }
    if let Some(bad) = failed {
        let shown = list.slots.iter().find(|slot| slot.id == bad);
        if !shown.is_some_and(|slot| slot.failed) {
            return Err(Refusal::new(
                1,
                format!("the board does not show {bad} as failed"),
            ));
        }
    }
    Ok(())
}

/// Installs a bundle through the board's install job and checks the board's list shows the slot.
async fn install_and_check<W: Write>(
    r: &mut Report<'_, W>,
    ctx: &Ctx<'_>,
    bundle: &Path,
) -> Result<String, Refusal> {
    let link = ctx.link()?;
    let info = link::board_info(&link.node);
    let id = commands::install(r, &ctx.state, link, &info, bundle, ctx.env.wait).await?;
    let list = ctx.slots()?.list().await?;
    if !list.slots.iter().any(|slot| slot.id == id) {
        return Err(Refusal::new(
            1,
            format!(
                "the board's list does not show {id} after the install: the run stops here and repairs nothing"
            ),
        ));
    }
    r.say(format!("the board's list shows {id}"));
    Ok(id)
}

/// Follows the watch held for the board until it ends or the person stops the run.
async fn follow(ctx: &Ctx<'_>, slots: &BoardSlots) -> Result<Ended, Refusal> {
    let link = ctx.link()?;
    let node_key = link.node.public_key;
    let stop = async {
        match &ctx.env.stop {
            Some(stop) => stop.notified().await,
            None => std::future::pending().await,
        }
    };
    let ended = tokio::select! {
        ended = ctx.follower.follow(slots, &link.handle, node_key) => ended,
        () = stop => ctx.follower.stop(node_key),
    };
    ended.ok_or_else(|| {
        Refusal::new(
            1,
            "the trial is not followed: no watch is held for the board",
        )
    })
}

/// Starts a trial and returns the handle that follows it, through the board's session.
async fn start(ctx: &Ctx<'_>, id: &str, now_ms: u64) -> Result<Option<String>, Refusal> {
    let slots = ctx.slots()?;
    let handle: &Handle = &ctx.link()?.handle;
    Ok(ctx
        .follower
        .start_trial(
            &slots,
            handle,
            TrialStart {
                id,
                keep_default: false,
                keep_old: false,
                force: false,
                now_ms,
            },
        )
        .await?)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

async fn step_identity(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let loaded =
        identity::load_or_create(&ctx.identity_path).map_err(|why| Refusal::new(1, why))?;
    ctx.key = loaded.secret;
    ctx.pc = identity::fingerprint(&identity::public_key(&ctx.key));
    r.say(if loaded.created {
        "this PC had no client identity: it was made now"
    } else {
        "this PC's client identity was read"
    });
    r.say(format!("this PC's client fingerprint: {}", ctx.pc));
    Ok(())
}

async fn step_fingerprint(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let node = commands::probe_or_refuse(ctx.env, &ctx.address, &ctx.text, &ctx.key).await?;
    ctx.board_key = node.public_key;
    ctx.board_fingerprint.clone_from(&node.fingerprint);
    r.say(format!("the board's key fingerprint: {}", node.fingerprint));
    let security = node.security();
    ctx.security = Some(security);
    r.say(format!(
        "security on the board: {}",
        security_words(security)
    ));
    Ok(())
}

async fn step_pin(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let mut pins = commands::load_pins(&ctx.state)?;
    match pins.get(&ctx.text).cloned() {
        Some(pin) if pin.key == ctx.board_key => {
            r.say(format!(
                "already pinned: {} is the key this address is pinned to",
                pin.fingerprint
            ));
        }
        Some(pin) => {
            return Err(link::replaced(
                &ctx.text,
                &pin,
                Some(ctx.board_fingerprint.clone()),
            ));
        }
        None => {
            pins.set(Pin {
                address: ctx.text.clone(),
                key: ctx.board_key,
                fingerprint: ctx.board_fingerprint.clone(),
            });
            commands::save_pins(&pins)?;
            r.say(format!(
                "pinned {} for {}: first contact, made by this run",
                ctx.board_fingerprint, ctx.text
            ));
        }
    }
    let pins = commands::load_pins(&ctx.state)?;
    ctx.link = Some(link::pinned(ctx.env, &pins, &ctx.text, &ctx.address, &ctx.key).await?);
    Ok(())
}

async fn step_list_before(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let list = ctx.slots()?.list().await?;
    for line in commands::list_lines(&list) {
        r.say(line);
    }
    Ok(())
}

async fn step_install_good(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let bundle = ctx.good_bundle.clone();
    ctx.good = install_and_check(r, ctx, &bundle).await?;
    Ok(())
}

async fn step_trial_good(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let id = ctx.good.clone();
    let before = boot_id(ctx).await;
    r.say(format!("boot id before the trial: {}", before.words()));
    if let Some(note) = start(ctx, &id, now_ms()).await? {
        r.say(note);
    }
    r.say(format!("trial of {id} started; following it to its end"));
    let slots = ctx.slots()?;
    let ended = follow(ctx, &slots).await?;
    if let Err(why) = &ended.saved {
        r.say(format!(
            "note: the outcome is not saved, so the trial may be reported again after an Atlas restart: {why}"
        ));
    }
    let pins = commands::load_pins(&ctx.state)?;
    commands::trial_outcome(r, ctx.env, &pins, &ctx.address, &id, ended.state, &ctx.key).await?;
    let after = boot_id(ctx).await;
    boot_check(r, &before, &after)?;
    check_list(r, ctx, &id, None).await
}

async fn step_install_bad(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let bundle = ctx.bad_bundle.clone();
    ctx.bad = install_and_check(r, ctx, &bundle).await?;
    Ok(())
}

async fn step_trial_bad_not_followed(
    ctx: &mut Ctx<'_>,
    r: &mut Report<'_, Vec<u8>>,
) -> Result<(), Refusal> {
    let id = ctx.bad.clone();
    let before = boot_id(ctx).await;
    r.say(format!("boot id before the bad trial: {}", before.words()));
    ctx.boot_before_bad = Some(before);
    if let Some(note) = start(ctx, &id, now_ms()).await? {
        r.say(note);
    }
    r.say(format!(
        "trial of {id} started and not followed yet: the board restarts into it"
    ));
    Ok(())
}

async fn step_install_good_refused(
    ctx: &mut Ctx<'_>,
    r: &mut Report<'_, Vec<u8>>,
) -> Result<(), Refusal> {
    let link = ctx.link()?;
    let info = link::board_info(&link.node);
    let bundle = ctx.good_bundle.clone();
    match commands::install(r, &ctx.state, link, &info, &bundle, ctx.env.wait).await {
        Ok(id) => Err(Refusal::new(
            1,
            format!(
                "the board took the install of {id} while the trial of {} was pending: the rule did not hold",
                ctx.bad
            ),
        )),
        Err(refusal) if refusal.busy => {
            r.say("refused while the trial is pending, as the board's rule says:");
            r.say(refusal.words);
            Ok(())
        }
        Err(refusal) => Err(refusal),
    }
}

async fn step_trial_bad_followed(
    ctx: &mut Ctx<'_>,
    r: &mut Report<'_, Vec<u8>>,
) -> Result<(), Refusal> {
    let id = ctx.bad.clone();
    let slots = ctx.slots()?;
    let ended = follow(ctx, &slots).await?;
    if let Err(why) = &ended.saved {
        r.say(format!(
            "note: the outcome is not saved, so the trial may be reported again after an Atlas restart: {why}"
        ));
    }
    match ended.state {
        TrialState::DidNotHold(decided) if matches!(&decided.running, Running::Other(slot) if *slot == ctx.good) =>
        {
            r.say(format!(
                "came back in the previous slot {} by itself: the trial of {id} failed. The board's reason: {}",
                ctx.good, decided.reason
            ));
        }
        other => {
            let pins = commands::load_pins(&ctx.state)?;
            commands::trial_outcome(r, ctx.env, &pins, &ctx.address, &id, other, &ctx.key).await?;
            return Err(Refusal::new(
                1,
                format!("the trial of {id} did not fall back to the previous slot"),
            ));
        }
    }
    let before = ctx.boot_before_bad.clone().unwrap_or(BootRead::NotChecked(
        "not checked: no boot id was read before the bad trial".to_owned(),
    ));
    let after = boot_id(ctx).await;
    boot_check(r, &before, &after)?;
    check_list(r, ctx, &ctx.good.clone(), Some(&id)).await
}

async fn step_confirm_good(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let id = ctx.good.clone();
    ctx.slots()?.set_default(&id, false).await?;
    r.say(format!("{id} is now the default slot"));
    Ok(())
}

async fn step_list_after(ctx: &mut Ctx<'_>, r: &mut Report<'_, Vec<u8>>) -> Result<(), Refusal> {
    let good = ctx.good.clone();
    let bad = ctx.bad.clone();
    check_list(r, ctx, &good, Some(&bad)).await
}

/// Runs every step in order and stops at the first that fails. Returns the exit status.
async fn sequence<W: Write>(ctx: &mut Ctx<'_>, card: &mut Card, out: &mut W) -> i32 {
    macro_rules! step {
        ($index:expr, $body:ident) => {{
            let began = Instant::now();
            let mut buf = Vec::new();
            let result = {
                let mut report = Report::new(&mut buf, false);
                $body(ctx, &mut report).await
            };
            if let Some(code) = card.close(&STEPS[$index], result, &buf, began, out) {
                return code;
            }
        }};
    }
    step!(0, step_identity);
    step!(1, step_fingerprint);
    step!(2, step_pin);
    step!(3, step_list_before);
    step!(4, step_install_good);
    step!(5, step_trial_good);
    step!(6, step_install_bad);
    step!(7, step_trial_bad_not_followed);
    step!(8, step_install_good_refused);
    step!(9, step_trial_bad_followed);
    step!(10, step_confirm_good);
    step!(11, step_list_after);
    0
}

/// Writes the run's words for a person: every step in order, a step that did not run said so, the
/// facts at the end, the verdict last.
fn render(ctx: &Ctx<'_>, card: &Card, code: i32, started: SystemTime, ended: SystemTime) -> String {
    let mut text = String::from("atlas-next cycle: scorecard\n\n");
    for info in STEPS.iter() {
        let record = card.records.iter().find(|record| record.info.id == info.id);
        text.push_str(&format!("Step {}: {}\n", info.id, info.title));
        text.push_str(&format!("  what the step does: {}\n", info.does));
        match record {
            Some(record) => {
                text.push_str(&format!(
                    "  result: {} (took {:.2} s)\n",
                    if record.passed { "PASS" } else { "FAIL" },
                    record.elapsed.as_secs_f64()
                ));
                text.push_str("  what the board and the tool said:\n");
                for word in &record.words {
                    text.push_str(&format!("    {word}\n"));
                }
            }
            None => text.push_str("  result: NOT RUN (an earlier step stopped the run)\n"),
        }
        text.push_str(&format!(
            "  if this step fails, the board comes back like this on its own: {}\n\n",
            info.if_it_fails
        ));
    }
    text.push_str("One line per step:\n");
    for info in STEPS.iter() {
        let result = card
            .records
            .iter()
            .find(|record| record.info.id == info.id)
            .map_or(
                "NOT RUN",
                |record| if record.passed { "PASS" } else { "FAIL" },
            );
        text.push_str(&format!("  Step {}: {result}, {}\n", info.id, info.title));
    }
    text.push('\n');
    text.push_str(&format!("Board address: {}\n", ctx.text));
    text.push_str(&format!(
        "Board key fingerprint: {}\n",
        if ctx.board_fingerprint.is_empty() {
            "not read"
        } else {
            &ctx.board_fingerprint
        }
    ));
    text.push_str(&format!(
        "This PC's client fingerprint: {}\n",
        if ctx.pc.is_empty() {
            "not read"
        } else {
            &ctx.pc
        }
    ));
    text.push_str(&format!(
        "Security on the board: {}\n",
        ctx.security.map_or("not read".to_owned(), security_words)
    ));
    text.push_str(&format!("Atlas build: {ATLAS_BUILD}\n"));
    text.push_str(&format!(
        "Run started (this PC's clock, UTC): {}\n",
        utc_stamp(started)
    ));
    text.push_str(&format!(
        "Run ended (this PC's clock, UTC): {}\n",
        utc_stamp(ended)
    ));
    text.push_str(&format!("Verdict: {}\n", verdict(ctx, card, code)));
    text
}

fn verdict(ctx: &Ctx<'_>, card: &Card, code: i32) -> String {
    if code == 0 {
        format!(
            "PASSED: every step passed. The board runs {} and its default is {}.",
            ctx.good, ctx.good
        )
    } else {
        let stopped = card
            .records
            .iter()
            .rev()
            .find(|record| !record.passed)
            .map_or_else(String::new, |record| {
                format!("step {} ({})", record.info.id, record.info.title)
            });
        format!("STOPPED at {stopped}. Nothing was repaired. Exit status {code}.")
    }
}

/// The same facts as JSON, one document beside the text scorecard.
fn render_json(
    ctx: &Ctx<'_>,
    card: &Card,
    code: i32,
    started: SystemTime,
    ended: SystemTime,
) -> String {
    let steps: Vec<Value> = STEPS
        .iter()
        .take(STEPS.len() - 1)
        .map(|info| {
            let record = card.records.iter().find(|record| record.info.id == info.id);
            json!({
                "id": info.id,
                "title": info.title,
                "result": match record {
                    Some(record) if record.passed => "PASS",
                    Some(_) => "FAIL",
                    None => "NOT RUN",
                },
                "seconds": record.map(|record| record.elapsed.as_secs_f64()),
                "words": record.map(|record| record.words.clone()).unwrap_or_default(),
                "if_it_fails": info.if_it_fails,
            })
        })
        .collect();
    json!({
        "steps": steps,
        "board_address": ctx.text,
        "board_fingerprint": ctx.board_fingerprint,
        "pc_fingerprint": ctx.pc,
        "security": ctx.security.map_or("not read", security_token),
        "security_generation": ctx.security.and_then(security_generation),
        "atlas_build": ATLAS_BUILD,
        "started": utc_stamp(started),
        "ended": utc_stamp(ended),
        "verdict": verdict(ctx, card, code),
        "exit": code,
    })
    .to_string()
}

/// A time as UTC text, from this PC's clock (RFC 3339, whole seconds).
#[must_use]
pub fn utc_stamp(time: SystemTime) -> String {
    let secs = i64::try_from(
        time.duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    )
    .unwrap_or(i64::MAX);
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil date from a day count since 1970-01-01 (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// The steps as a plan, with each step's return path. Sends nothing.
fn dry_run<W: Write>(args: &CycleArgs, out: &mut W) -> i32 {
    let _ = writeln!(
        out,
        "atlas-next cycle: dry run. Nothing is sent to any board."
    );
    let _ = writeln!(out, "Board address: {}", args.address);
    let _ = writeln!(out, "Good bundle: {}", args.good.display());
    let _ = writeln!(out, "Bad bundle: {}", args.bad.display());
    let _ = writeln!(out, "Scorecard: {}", args.scorecard.display());
    let _ = writeln!(out);
    for info in STEPS.iter() {
        let _ = writeln!(out, "Step {}: {}", info.id, info.title);
        let _ = writeln!(out, "  does: {}", info.does);
        let _ = writeln!(
            out,
            "  if it fails, the board comes back like this: {}",
            info.if_it_fails
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Exit status: 0 when every step passed; 1 when a step failed; 3 when the pin rule refused the board."
    );
    0
}

/// `atlas-next cycle`: runs the cycle and writes the scorecard. Returns the exit status.
pub async fn run<W: Write>(args: &CycleArgs, env: &Env, out: &mut W) -> i32 {
    let address = match parse_address(&args.address) {
        Ok(address) => address,
        Err(why) => {
            let _ = writeln!(out, "atlas-next: {why}");
            return 2;
        }
    };
    if args.dry_run {
        return dry_run(args, out);
    }
    let Some(state) = args.state.clone().or_else(|| env.state_dir.clone()) else {
        let _ = writeln!(out, "atlas-next: no state directory: give --state <dir>");
        return 2;
    };
    let Some(identity_path) = args
        .identity
        .clone()
        .or_else(|| env.identity_default.clone())
    else {
        let _ = writeln!(out, "atlas-next: no identity file: give --identity <file>");
        return 2;
    };
    for bundle in [&args.good, &args.bad] {
        if !bundle.is_file() {
            let _ = writeln!(out, "atlas-next: no such bundle file: {}", bundle.display());
            return 2;
        }
    }
    let started = SystemTime::now();
    let text = address.to_string();
    let mut ctx = Ctx {
        env,
        address,
        text,
        state: state.clone(),
        identity_path,
        good_bundle: args.good.clone(),
        bad_bundle: args.bad.clone(),
        key: [0; 32],
        pc: String::new(),
        board_key: [0; 32],
        board_fingerprint: String::new(),
        security: None,
        link: None,
        follower: Follower::open(&state).with_recheck(env.recheck),
        good: String::new(),
        bad: String::new(),
        boot_before_bad: None,
    };
    let mut card = Card::default();
    let code = sequence(&mut ctx, &mut card, out).await;
    let ended = SystemTime::now();
    let text = render(&ctx, &card, code, started, ended);
    let json_text = render_json(&ctx, &card, code, started, ended);
    let written = write_scorecard(
        &args.scorecard,
        &text,
        args.json.then_some(json_text.as_str()),
    );
    match written {
        Ok(()) => {
            let _ = writeln!(out, "scorecard written: {}", args.scorecard.display());
            if args.json {
                let _ = writeln!(
                    out,
                    "scorecard JSON written: {}",
                    args.scorecard.with_extension("json").display()
                );
            }
        }
        Err(why) => {
            let _ = writeln!(out, "atlas-next: the scorecard was not written: {why}");
            return 1;
        }
    }
    let _ = writeln!(out, "exit status {code}");
    code
}

/// Writes the text scorecard, and the JSON one beside it when given. A file that exists is replaced
/// whole: the scorecard is one run's record.
fn write_scorecard(path: &Path, text: &str, json: Option<&str>) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    if let Some(json) = json {
        let beside = path.with_extension("json");
        fs::write(&beside, json).map_err(|error| format!("{}: {error}", beside.display()))?;
    }
    Ok(())
}

/// The board's security switch in words for people: step 2 and the scorecard's text.
fn security_words(security: Security) -> String {
    match security {
        Security::NotTold => "the board has not said".to_owned(),
        Security::On { generation } => {
            with_generation("on: a signed session is needed".to_owned(), generation)
        }
        Security::Off { generation } => {
            with_generation("off: a plain session is enough".to_owned(), generation)
        }
        Security::Damaged => {
            "damaged: its security record cannot be read, so the board counts it as on".to_owned()
        }
        _ => "the board has not said".to_owned(),
    }
}

fn with_generation(words: String, generation: Option<u64>) -> String {
    match generation {
        Some(generation) => format!("{words} (record generation {generation})"),
        None => words,
    }
}

/// The security switch as one word, for the scorecard's JSON.
fn security_token(security: Security) -> &'static str {
    match security {
        Security::NotTold => "not told",
        Security::On { .. } => "on",
        Security::Off { .. } => "off",
        Security::Damaged => "damaged",
        _ => "not told",
    }
}

fn security_generation(security: Security) -> Option<u64> {
    match security {
        Security::On { generation } | Security::Off { generation } => generation,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use orion::node::{Fact, Node};

    use super::*;

    fn node_with(facts: &[(&str, &str)]) -> Node {
        Node {
            name: "board".into(),
            cluster: "lab".into(),
            public_key: [0; 32],
            fingerprint: String::new(),
            revision: 1,
            time_ms: 0,
            facts: facts
                .iter()
                .map(|(name, value)| Fact {
                    name: (*name).into(),
                    ty: 4,
                    value: value.as_bytes().to_vec(),
                })
                .collect(),
        }
    }

    #[test]
    fn each_security_state_is_read_from_the_hello_and_worded_for_people() {
        // On, with the record's generation.
        let on = node_with(&[("security", "on"), ("security_generation", "7")]).security();
        assert_eq!(
            security_words(on),
            "on: a signed session is needed (record generation 7)"
        );
        assert_eq!(security_token(on), "on");
        assert_eq!(security_generation(on), Some(7));

        // Off, with no record generation.
        let off = node_with(&[("security", "off")]).security();
        assert_eq!(security_words(off), "off: a plain session is enough");
        assert_eq!(security_token(off), "off");
        assert_eq!(security_generation(off), None);

        // Damaged: the board counts it as on.
        let damaged = node_with(&[("security", "damaged")]).security();
        assert_eq!(
            security_words(damaged),
            "damaged: its security record cannot be read, so the board counts it as on"
        );
        assert_eq!(security_token(damaged), "damaged");

        // An older board says nothing: not told.
        let old = node_with(&[("state", "running")]).security();
        assert_eq!(security_words(old), "the board has not said");
        assert_eq!(security_token(old), "not told");
        assert_eq!(security_generation(old), None);
    }

    #[test]
    fn utc_stamp_reads_a_known_time() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // 2026-11-14T10:20:30Z
        let secs = 1_763_115_630_u64;
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(secs)),
            "2025-11-14T10:20:30Z"
        );
        // 2000-02-29 is a leap day.
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
    }

    #[test]
    fn every_step_names_its_return_path_and_ids_are_unique() {
        let mut ids = std::collections::BTreeSet::new();
        for step in &STEPS {
            assert!(!step.if_it_fails.is_empty(), "{}", step.id);
            assert!(ids.insert(step.id), "{} is listed twice", step.id);
        }
    }

    #[test]
    fn the_pinned_board_publishes_the_boot_id_and_an_absent_one_is_not_checked() {
        // The pin writes `last_attempt.boot` (attempt.rs:176): the boot id is read, not skipped.
        let published = INFO_IN_PIN;
        assert!(published, "the pin publishes info.last_attempt.boot");
        assert_eq!(
            BootRead::NotChecked(NOT_CHECKED.to_owned()).words(),
            "not checked: the board's info gives no boot id"
        );
    }
}
