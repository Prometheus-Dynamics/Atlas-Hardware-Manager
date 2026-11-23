//! The board's actions, run on its node through the session.
//!
//! Every call goes through the session's [`Handle::with_client`] (a read) or [`Handle::with_bridge`]
//! (a run, or its cancel, which renews the board's lease), so it has the session's call
//! wait and never runs after its caller gave up.
//!
//! Two ways to run an action:
//!
//! - A **short** action is one call ([`BoardActions::run`]): started
//!   and waited for inside a single call, **not detached**. The board cancels a run whose session
//!   ends, so a direct run is never left on the board where nobody can follow it. A direct run
//!   that does not finish within half the call wait (at most [`DIRECT_WAIT`]) is asked to stop
//!   and reported as [`ActionError::NotFinished`]: a longer action is a job.
//! - A **long** action is started **detached** ([`BoardActions::start`]), so the board keeps the
//!   run if the session ends, and is followed in short polls ([`BoardActions::follow`]). A lost
//!   session returns [`ActionError::Lost`] with the run's number; following that number again on
//!   the next session resumes the run instead of starting it again. Only detached runs can be
//!   followed by number, which is why direct runs are not detached: a number reused by a later
//!   direct run never makes a pending job run ambiguous.
//!
//! Retry safety of each call:
//!
//! - [`BoardActions::catalog`] reads what the board offers. It changes nothing, so it may be made again.
//! - A direct [`BoardActions::run`] is never retried by this crate: a call that did not come back
//!   is [`ActionError::Unknown`], and the caller decides.
//! - [`BoardActions::start`] is never retried by this crate: a start that was sent and not
//!   confirmed is [`ActionError::StartInDoubt`], and the job kind does not start it again either
//!   (see [`crate::jobs`]).
//! - [`BoardActions::follow`] reads a run's progress and result. Reading changes nothing, so it
//!   may be made again; it is what a lost session resumes with.
//! - [`BoardActions::cancel`] asks a run to stop. Asking again after it has ended changes
//!   nothing, so it may be made again.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use atlas_session::lease::lapse_words;
use atlas_session::lemnos;
use atlas_session::{Failure, FailureKind, Handle, Hold, Presence, SessionError};
use orion::actions::{ActionCall, Finished, Outcome, RunHandle, Target, Update};
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError, Feature, Orion};

use crate::args;
use crate::catalog::{Action, Catalog, Reported, Route};
use crate::error::ActionError;

/// How long one poll of a detached run waits for news before the session is looked at again.
pub const POLL: Duration = Duration::from_millis(250);
/// The longest a short action may take in one call. Half the call wait is used when that is less.
pub const DIRECT_WAIT: Duration = Duration::from_secs(2);
/// The deadline a detached run is started with: Orion's longest by default
/// (`orion_action_list::MAX_DETACHED_MS`, a node's `actions.max_detached_ms`). A node refuses a
/// detached run with none (`NoDeadline`), so that a run its caller left still ends by itself.
pub const DETACHED_DEADLINE_MS: u64 = 60 * 60 * 1000;

/// What a hold lapse reports as the board's words when the run had not ended yet.
const NOT_FINISHED: &str = "not finished";

/// The sentence a run's lapse shows: the lease, the period it was kept for, and the run it may have
/// cost. `said` is the board's own words when the run had ended with an error, else `not finished`.
fn lapsed_reason(period: Duration, call: u64, said: &str) -> String {
    let words = lapse_words(period, &format!("run {call}"));
    if said == NOT_FINISHED || said.is_empty() {
        words
    } else {
        format!("{words}. The board said: {said}")
    }
}

/// The prefix a board's reply carries once the caller's lease ended and its holds were released:
/// the first reply after the drop begins with it (Lemnos M39-TCP, docs/daemon.md). This is a text
/// match, kept until Lemnos sends the typed reply field (M40); then that field replaces it.
pub const LEASE_ENDED_PREFIX: &str = "lease-ended:";

/// Whether a reply's detail says the board released the caller's holds when its lease ended.
#[must_use]
pub fn lease_ended(detail: &str) -> bool {
    detail.starts_with(LEASE_ENDED_PREFIX)
}

/// A run that ended successfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    /// What the action returned.
    // Lemnos M8a (5dcf1a78) is in the lock (d6b9361d): the bridge's `call` returns a method's out buffer.
    pub output: Vec<u8>,
    /// What the board said about the result, if it said anything.
    pub detail: String,
}

/// How far a long run has got, as the board last reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Percent done, or `None` when the action does not say.
    pub percent: Option<u8>,
    /// What the action is doing now, in its own words.
    pub message: String,
}

/// The board's actions, through one node's session.
#[derive(Clone, Debug)]
pub struct BoardActions {
    handle: Handle,
}

/// What one poll of a detached run saw.
enum Seen {
    Quiet,
    Doing(Progress),
    Ended(Finished),
    Refused(Failure),
}

/// What one direct call saw.
enum Direct {
    Ended(Finished),
    Refused(Failure),
    Late,
    /// Nothing was sent: see [`Missing`].
    Missing(Missing),
}

/// Where a run goes: the name, target and arguments the node's session takes.
struct Aim {
    name: String,
    target: Target,
    args: Vec<u8>,
}

/// Where a run of `name` with `encoded` arguments goes, read on the node's session. A Lemnos method
/// (`system.<contract>.<method>`, see [`lemnos::method_of`]) is the call on the device that carries
/// its contract, built in one place (`atlas_session::lemnos`): the device's resource and the
/// bridge's `TAG:METHOD` form. Any other name runs on the node, as before.
///
/// Refuses, before anything is sent, a word of a Lemnos call that the bridge cannot take. The bridge
/// splits a call's words on whitespace with no quoting or escape (`service/orion/src/calls.rs`,
/// `serve`), so a word that is empty or has whitespace in it would arrive as a different call.
/// Nothing is sent, and the reason names the argument.
fn refuse_unsplit(action: &Action, words: &[String]) -> Result<(), ActionError> {
    if lemnos::method_of(&action.name).is_none() {
        return Ok(());
    }
    for (index, word) in words.iter().enumerate() {
        let problem = if word.is_empty() {
            "empty"
        } else if word.chars().any(char::is_whitespace) {
            "a word with whitespace in it"
        } else {
            continue;
        };
        let arg = match &action.args {
            Reported::Known(args) => args
                .get(index)
                .map_or_else(|| format!("argument {}", index + 1), |arg| arg.name.clone()),
            Reported::NotReported => format!("argument {}", index + 1),
        };
        return Err(ActionError::BadArguments(format!(
            "{arg} is {problem}: the board's call takes its words split on spaces, with no quoting, so it is not sent"
        )));
    }
    Ok(())
}

/// Why a Lemnos call is not sent: no device carries its contract, or the node lists no device.
#[derive(Debug)]
enum Missing {
    Contract(String),
    NoDevices,
    /// The device is not answering yet, or the search did not finish: the sentence for people.
    NotYet(String),
    /// The node turns this client away (`denied`): the failure, with its kind.
    Unanswered(atlas_session::Failure),
}

/// `Err` when nothing is to be sent: no device carries the contract, or there are no devices. With
/// `device`, the call goes to that device's resource and the contract is not looked up again.
fn aim(
    orion: &Orion,
    session: u64,
    name: &str,
    encoded: &[u8],
    node: &NodeName,
    device: Option<&str>,
) -> Result<Result<Aim, Missing>, OrionError> {
    let Some((contract, method)) = lemnos::method_of(name) else {
        return Ok(Ok(Aim {
            name: name.to_owned(),
            target: Target::Node(node.clone()),
            args: encoded.to_vec(),
        }));
    };
    let resource = match device {
        Some(device) => format!("{}{device}", lemnos::RESOURCE_PREFIX),
        None => match lemnos::resource_cached(orion, session, &contract)? {
            lemnos::Lookup::Found(resource) => resource,
            lemnos::Lookup::NoContract => return Ok(Err(Missing::Contract(contract))),
            lemnos::Lookup::NoDevices => return Ok(Err(Missing::NoDevices)),
            lemnos::Lookup::NotYet(text) => return Ok(Err(Missing::NotYet(text))),
            lemnos::Lookup::Unanswered(words) => return Ok(Err(Missing::Unanswered(words))),
        },
    };
    // A resource the node lists under the bridge's prefix is a name the bridge can have.
    let Ok(call) = lemnos::method_call(&resource, &contract, &method, encoded) else {
        return Ok(Err(Missing::Contract(contract)));
    };
    Ok(Ok(Aim {
        name: call.name,
        target: call.target,
        args: call.args,
    }))
}

/// The error a [`Missing`] call gives: nothing was sent.
fn not_sent(missing: Missing) -> ActionError {
    match missing {
        Missing::Contract(contract) => ActionError::NoContract(contract),
        Missing::NoDevices => ActionError::NoDevices,
        // Not final: nothing was sent, and the caller may ask again (a job waits and tries again).
        Missing::NotYet(text) => ActionError::NotYet(text),
        // A node that turns this client away says so as a refusal, which the caller can act on (enrol).
        Missing::Unanswered(failure) if failure.is_denied() => ActionError::Refused(failure),
        Missing::Unanswered(failure) => ActionError::NotAnswering(failure.to_string()),
    }
}

impl BoardActions {
    /// The actions of the node `handle` keeps a session with.
    #[must_use]
    pub const fn new(handle: Handle) -> Self {
        Self { handle }
    }

    /// The session this reaches the node through.
    #[must_use]
    pub const fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Reads what the board offers to run. No action of the board is asked for a list: no board
    /// publishes one. The names come from the node's resource list (its devices, and the
    /// `lemnos.<device>.call` each one takes) and the methods, their arguments and whether each is a
    /// long job come from each device's description, which the session keeps once asked
    /// (`atlas_session::lemnos::offers`). A device that refuses its description (one that is off does)
    /// offers nothing; a node with no devices (a plain Orion node) gives an empty list.
    ///
    /// # Errors
    ///
    /// [`ActionError::NotConnected`] (nothing sent); [`ActionError::Refused`] when every device
    /// refused its description because the node turns this client away, with the node's reason;
    /// [`ActionError::NotAnswering`] when every device refused it for another reason (Lemnos is
    /// stopped on the node), or the node did not answer.
    pub async fn catalog(&self) -> Result<Catalog, ActionError> {
        // Retry: reading the list changes nothing, so the caller may make it again.
        self.node_name()?;
        let session = self.handle.session();
        let mut offers = self
            .handle
            // A read: with the descriptions kept, nothing reaches the bridge, so this is not counted
            // as a request that renews the board's lease.
            .with_client(move |orion| lemnos::offers(orion, session))
            .await
            .map_err(|error| match error {
                SessionError::Gone(reason) => ActionError::NotAnswering(reason),
                SessionError::Failed(failure) => ActionError::NotAnswering(failure.to_string()),
            })?;
        if offers.devices > 0 && offers.unanswered.len() == offers.devices {
            let (_, failure) = offers.unanswered.swap_remove(0);
            return Err(if failure.is_denied() {
                ActionError::Refused(failure)
            } else {
                ActionError::NotAnswering(failure.to_string())
            });
        }
        Ok(Catalog::from_offers(&offers.methods))
    }

    /// Runs a short action in one call and returns its result.
    ///
    /// Only an action the board describes as short runs here; a long one, or one whose length is
    /// not reported, is [`ActionError::NeedsJob`]. The words are checked against the description
    /// first, so a bad one is never sent.
    ///
    /// Never retried by this crate: a call that did not come back is [`ActionError::Unknown`].
    ///
    /// # Errors
    ///
    /// [`ActionError::BadArguments`] and [`ActionError::NeedsJob`] (nothing sent),
    /// [`ActionError::NotConnected`] (nothing sent), [`ActionError::Refused`], [`ActionError::Failed`]
    /// or [`ActionError::Cancelled`] with the board's reason, [`ActionError::NotFinished`], or
    /// [`ActionError::Unknown`].
    pub async fn run(&self, action: &Action, words: &[String]) -> Result<Done, ActionError> {
        if action.route() == Route::Job {
            return Err(ActionError::NeedsJob(action.name.clone()));
        }
        refuse_unsplit(action, words)?;
        args::check(action, words)?;
        self.direct(&action.name, args::encode(words)).await
    }

    /// Starts `action` with `words` **detached**, and returns its run number on the board, so a
    /// long action can be followed. The words are checked first; nothing is sent when they do not
    /// fit.
    ///
    /// Never retried by this crate: a start the node did not confirm is
    /// [`ActionError::StartInDoubt`].
    ///
    /// # Errors
    ///
    /// [`ActionError::BadArguments`] (nothing sent), [`ActionError::NotConnected`] (nothing sent),
    /// [`ActionError::Refused`] with the board's reason, or [`ActionError::StartInDoubt`].
    pub async fn start(&self, action: &Action, words: &[String]) -> Result<Issued, ActionError> {
        refuse_unsplit(action, words)?;
        args::check(action, words)?;
        self.start_encoded(&action.name, args::encode(words), None)
            .await
    }

    /// As [`BoardActions::start`], on device `device` (the id a [`BoardActions::device`] lookup
    /// gave): the start goes to that device's resource, so a push and its install name one device.
    ///
    /// # Errors
    ///
    /// As [`BoardActions::start`].
    pub async fn start_on(
        &self,
        action: &Action,
        words: &[String],
        device: &str,
    ) -> Result<Issued, ActionError> {
        refuse_unsplit(action, words)?;
        args::check(action, words)?;
        self.start_encoded(&action.name, args::encode(words), Some(device))
            .await
    }

    /// The id of the device that carries `contract` now (the id is the `<device>` of its
    /// `lemnos.<device>` resource: a push into its inbox is `lemnos/<device>/`). Nothing is sent to
    /// the device for the lookup beyond the description the session keeps.
    ///
    /// # Errors
    ///
    /// [`ActionError::NoContract`], [`ActionError::NoDevices`], [`ActionError::NotYet`] or
    /// [`ActionError::Refused`]/[`ActionError::NotAnswering`] as [`BoardActions::run`] gives them; the
    /// lookup itself sends nothing that is not counted in [`BoardActions::start`].
    pub async fn device(&self, contract: &str) -> Result<String, ActionError> {
        self.node_name()?;
        let session = self.handle.session();
        let asked = contract.to_owned();
        let found = self
            .handle
            .with_bridge(move |orion| lemnos::resource_cached(orion, session, &asked))
            .await
            .map_err(|error| match error {
                SessionError::Gone(reason) => ActionError::NotAnswering(reason),
                SessionError::Failed(failure) => ActionError::NotAnswering(failure.to_string()),
            })?;
        let missing = match found {
            lemnos::Lookup::Found(resource) => {
                return lemnos::device_of(&resource)
                    .map(str::to_owned)
                    .ok_or_else(|| ActionError::NoContract(contract.to_owned()));
            }
            lemnos::Lookup::NoContract => Missing::Contract(contract.to_owned()),
            lemnos::Lookup::NoDevices => Missing::NoDevices,
            lemnos::Lookup::NotYet(text) => Missing::NotYet(text),
            lemnos::Lookup::Unanswered(failure) => Missing::Unanswered(failure),
        };
        Err(not_sent(missing))
    }

    /// Starts `name` with `encoded` arguments, detached, on `device` when given (else on the device
    /// that carries the name's contract). The arguments must already be checked.
    pub(crate) async fn start_encoded(
        &self,
        name: &str,
        encoded: Vec<u8>,
        device: Option<&str>,
    ) -> Result<Issued, ActionError> {
        let node_name = self.node_name()?;
        let owned = name.to_owned();
        let started = name.to_owned();
        let on = device.map(str::to_owned);
        // Read before the start is sent: a session that changes meanwhile makes the run look
        // older than it is, so a later use treats it as another session's (the safe side).
        let session = self.handle.session();
        let wait = self.direct_wait();
        let sent = self
            .handle
            .with_bridge(move |orion| {
                let aimed = match aim(orion, session, &owned, &encoded, &node_name, on.as_deref())?
                {
                    Ok(aimed) => aimed,
                    Err(missing) => return Ok(Started::Missing(missing)),
                };
                let mut run = orion.run(&ActionCall {
                    name: &aimed.name,
                    target: aimed.target,
                    args: &aimed.args,
                    deadline_ms: DETACHED_DEADLINE_MS,
                    detached: true,
                })?;
                // Orion C53b (level 3, `Feature::RunHandle`): node logic names the run by its
                // handle as it starts. The handle reaches this run alone from any later session;
                // a number reaches only runs of the session it was given on. Read within the
                // direct wait; a refusal (no such action, no deadline, busy) is read here too. A
                // progress or end read meanwhile is given again to whoever attaches.
                let handle = if orion.welcomed().needs(Feature::RunHandle).is_ok() {
                    let until = Instant::now() + wait;
                    loop {
                        if let Some(handle) = run.handle() {
                            break Some(handle);
                        }
                        let left = until.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break None;
                        }
                        match run.wait(left.min(POLL)) {
                            Ok(_) => {}
                            // // Busy: the board had no room and did not take it; the session asks again (`Handle::with_client`).
                            Err(error) if error.is_busy() => return Err(OrionError::Client(error)),
                            Err(ClientError::Refused { error, detail }) => {
                                return Ok(Started::Refused(Failure::of_core(error, detail)));
                            }
                            // The start was sent and the session went before the handle came: the
                            // run is issued by its number, as before handles; the follow decides.
                            Err(_) => break None,
                        }
                    }
                } else {
                    None
                };
                Ok(Started::Issued(Issued {
                    call: run.call(),
                    session,
                    name: started.clone(),
                    handle,
                }))
            })
            .await;
        // Retry: never by this crate. A start that was sent and not answered may already be on
        // the board, so the caller is told and decides.
        match sent {
            Ok(Started::Issued(issued)) => Ok(issued),
            Ok(Started::Refused(reason)) => Err(ActionError::Refused(reason)),
            Ok(Started::Missing(missing)) => Err(not_sent(missing)),
            Err(SessionError::Gone(reason)) => Err(ActionError::StartInDoubt {
                name: name.to_owned(),
                reason,
            }),
            // Busy past the session's bound, or stopped by a person, or held by another: the board
            // never took the start, so it is a refusal, with the board's words.
            Err(SessionError::Failed(reason))
                if matches!(reason.kind(), FailureKind::Busy | FailureKind::Held) =>
            {
                Err(ActionError::Refused(reason))
            }
            Err(SessionError::Failed(reason)) => Err(ActionError::Failed(reason)),
        }
    }

    /// Follows detached run `call`, passing each progress report to `on_progress`, until the
    /// board says how it ended. A lost session returns [`ActionError::Lost`] with the run's
    /// number, so the caller can follow it again once the session is back.
    ///
    /// # Errors
    ///
    /// [`ActionError::Refused`], [`ActionError::Failed`] or [`ActionError::Cancelled`] with the
    /// board's reason when the run ended that way, or [`ActionError::Lost`].
    pub async fn follow(
        &self,
        issued: &Issued,
        contract: Contract,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Followed, ActionError> {
        self.follow_inner(issued, contract, None, on_progress).await
    }

    /// As [`BoardActions::follow`], for a run whose hold the caller keeps. When the session lapsed
    /// while the hold existed, the board may have dropped the run's hold: the follow then ends with
    /// [`ActionError::HoldLapsed`] instead of waiting on a board that may no longer hold the run.
    /// A run that ended with an error after a lapse is reported the same way, with the board's words.
    ///
    /// # Errors
    ///
    /// As [`BoardActions::follow`], and [`ActionError::HoldLapsed`].
    pub async fn follow_while_held(
        &self,
        issued: &Issued,
        contract: Contract,
        hold: &Hold,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Followed, ActionError> {
        self.follow_inner(issued, contract, Some(hold), on_progress)
            .await
    }

    async fn follow_inner(
        &self,
        issued: &Issued,
        contract: Contract,
        hold: Option<&Hold>,
        mut on_progress: impl FnMut(&Progress),
    ) -> Result<Followed, ActionError> {
        let call = issued.call;
        let handle = issued.handle;
        // Retry: following is a read. A lost session returns `Lost` with the run's number, and
        // following that number on the new session resumes it; the run is never started again.
        loop {
            let seen = self
                .handle
                .with_client(move |orion| {
                    let mut run = attach(orion, call, handle)?;
                    match run.wait(POLL) {
                        Ok(update) => Ok(Seen::of(update)),
                        // // Busy: the board had no room and did not take it; the session asks again (`Handle::with_client`).
                        Err(error) if error.is_busy() => Err(OrionError::Client(error)),
                        Err(ClientError::Refused { error, detail }) => {
                            Ok(Seen::Refused(Failure::of_core(error, detail)))
                        }
                        Err(error) => Err(OrionError::Client(error)),
                    }
                })
                .await;
            if let Some(hold) = hold
                && hold.lapsed()
            {
                return Err(ActionError::HoldLapsed {
                    call,
                    reason: lapsed_reason(hold.lease_period(), call, NOT_FINISHED),
                });
            }
            match seen {
                Ok(Seen::Quiet) => {}
                Ok(Seen::Doing(progress)) => on_progress(&progress),
                Ok(Seen::Ended(finished)) => {
                    // The board says the caller's lease ended before this reply: its holds are gone.
                    if lease_ended(&finished.detail) {
                        return Err(ActionError::HoldLapsed {
                            call,
                            reason: lapsed_reason(
                                hold.map_or(atlas_session::LEASE, Hold::lease_period),
                                call,
                                &finished.detail,
                            ),
                        });
                    }
                    let result = followed_end(finished);
                    let on_issuing_session = self.handle.session() == issued.session;
                    let done = match (hold, result) {
                        (Some(hold), Err(error)) if hold.lapsed() => {
                            return Err(ActionError::HoldLapsed {
                                call,
                                reason: lapsed_reason(
                                    hold.lease_period(),
                                    call,
                                    &error.to_string(),
                                ),
                            });
                        }
                        // An end read on a later session is not trusted, failure or cancel alike:
                        // a cancel confirmed from another session's answer is not a confirmed cancel.
                        (_, Err(error)) if !on_issuing_session => {
                            let reason = format!(
                                "the run ended as `{error}`, which the node said on a later session"
                            );
                            let ended =
                                matches!(error, ActionError::Refused(_) | ActionError::Failed(_))
                                    .then(|| Box::new(error));
                            return Err(ActionError::Unconfirmed {
                                call,
                                reason,
                                ended,
                            });
                        }
                        (_, result) => result?,
                    };
                    // The answer is checked against what was started, before anything reads it.
                    if let Err(got) = contract(&done.output) {
                        return Err(ActionError::WrongAnswer {
                            call,
                            started: issued.name.clone(),
                            got,
                        });
                    }
                    return Ok(if self.handle.session() == issued.session {
                        Followed::Confirmed(done)
                    } else {
                        Followed::Reconnected(done)
                    });
                }
                // A run started on an earlier session that the board no longer holds (its end
                // given already, kept no longer, or node logic started again): how it ended is not
                // known here, so the outcome is unknown, never a refusal of the action.
                Ok(Seen::Refused(reason)) if self.handle.session() != issued.session => {
                    return Err(ActionError::Unconfirmed {
                        call,
                        reason: format!(
                            "after a reconnect the board no longer holds the run started on an earlier session ({reason}): its outcome is unknown"
                        ),
                        ended: None,
                    });
                }
                Ok(Seen::Refused(reason)) => return Err(ActionError::Refused(reason)),
                Err(SessionError::Gone(reason)) => {
                    return Err(ActionError::Lost {
                        call,
                        reason: Failure::other(reason),
                    });
                }
                Err(SessionError::Failed(reason)) => {
                    return Err(ActionError::Lost { call, reason });
                }
            }
        }
    }

    /// Asks detached run `call` to stop. How it ended still comes from [`BoardActions::follow`]:
    /// a cancel that was sent is not yet a cancel that took effect, so the caller checks.
    ///
    /// Retry: asking a run that has ended changes nothing, so a cancel may be sent again.
    ///
    /// # Errors
    ///
    /// [`ActionError::Lost`] when the session did not carry the request.
    pub async fn cancel(&self, issued: &Issued) -> Result<(), ActionError> {
        let call = issued.call;
        let handle = issued.handle;
        // A number issued on an earlier session may now name another run on the board. It is not
        // sent a cancel, since a cancel to the wrong run stops someone else's work.
        if self.handle.session() != issued.session {
            return Err(ActionError::Unconfirmed {
                call,
                reason: "the run was started on an earlier session, and its number may now belong to another run: not cancelled blind".to_owned(),
                ended: None,
            });
        }
        self.handle
            .with_bridge(move |orion| {
                let mut run = attach(orion, call, handle)?;
                run.cancel().map_err(OrionError::Client)
            })
            .await
            .map_err(|error| match error {
                SessionError::Gone(reason) => ActionError::Lost {
                    call,
                    reason: Failure::other(reason),
                },
                SessionError::Failed(reason) => ActionError::Lost { call, reason },
            })
    }

    /// Whether the session is still the one `issued` was started on. A stored run number answers
    /// for its own session only; a caller that uses an answer checks this first.
    #[must_use]
    pub fn same_session(&self, issued: &Issued) -> bool {
        self.handle.session() == issued.session
    }

    /// The node's name, from its hello document, as the board's target.
    fn node_name(&self) -> Result<NodeName, ActionError> {
        let node = match self.handle.presence() {
            Presence::Found(node) => node,
            Presence::Lost(reason) => return Err(ActionError::NotConnected(reason)),
            Presence::Searching => {
                return Err(ActionError::NotConnected("not connected yet".into()));
            }
            Presence::Replaced { .. } => {
                return Err(ActionError::NotConnected(
                    "the node at this address was replaced by another node key".into(),
                ));
            }
        };
        node.name.parse().map_err(|_| {
            ActionError::NotAnswering(format!(
                "the node's name {:?} is not a valid name",
                node.name
            ))
        })
    }

    /// How long a direct call may wait for its run inside one call: half the call wait, and at
    /// most [`DIRECT_WAIT`].
    fn direct_wait(&self) -> Duration {
        (self.handle.call_wait() / 2).min(DIRECT_WAIT)
    }

    /// One short call: the action is started and waited for inside the call, not detached.
    async fn direct(&self, name: &str, encoded: Vec<u8>) -> Result<Done, ActionError> {
        let node_name = self.node_name()?;
        let wait = self.direct_wait();
        let owned = name.to_owned();
        let session = self.handle.session();
        let answered = self
            .handle
            .with_bridge(move |orion| {
                let aimed = match aim(orion, session, &owned, &encoded, &node_name, None)? {
                    Ok(aimed) => aimed,
                    Err(missing) => return Ok(Direct::Missing(missing)),
                };
                let mut run = orion.run(&ActionCall {
                    name: &aimed.name,
                    target: aimed.target,
                    args: &aimed.args,
                    deadline_ms: 0,
                    detached: false,
                })?;
                match run.finish(wait) {
                    Ok(finished) => Ok(Direct::Ended(finished)),
                    // // Busy: the board had no room and did not take it; the session asks again (`Handle::with_client`).
                    Err(error) if error.is_busy() => Err(OrionError::Client(error)),
                    Err(ClientError::Refused { error, detail }) => {
                        Ok(Direct::Refused(Failure::of_core(error, detail)))
                    }
                    Err(ClientError::Io(error)) if error.kind() == ErrorKind::TimedOut => {
                        // Not finished in time: asked to stop, so it is not left running unobserved.
                        let _ = run.cancel();
                        Ok(Direct::Late)
                    }
                    Err(error) => Err(OrionError::Client(error)),
                }
            })
            .await;
        match answered {
            Ok(Direct::Ended(finished)) => ended(finished),
            Ok(Direct::Refused(reason)) => Err(ActionError::Refused(reason)),
            Ok(Direct::Missing(found)) => Err(not_sent(found)),
            Ok(Direct::Late) => Err(ActionError::NotFinished {
                name: name.to_owned(),
                wait,
            }),
            Err(SessionError::Gone(reason)) => Err(ActionError::Unknown {
                name: name.to_owned(),
                reason: Failure::other(reason),
            }),
            // Busy past the session's bound, or stopped by a person, or held by another: the board
            // never took the run, so it is a refusal with the board's words, not an outcome nobody knows.
            Err(SessionError::Failed(reason))
                if matches!(reason.kind(), FailureKind::Busy | FailureKind::Held) =>
            {
                Err(ActionError::Refused(reason))
            }
            Err(SessionError::Failed(reason)) => Err(ActionError::Unknown {
                name: name.to_owned(),
                reason,
            }),
        }
    }
}

impl Seen {
    fn of(update: Option<Update>) -> Self {
        match update {
            None => Self::Quiet,
            Some(Update::Doing(doing)) => Self::Doing(Progress {
                percent: (doing.percent != 255).then_some(doing.percent),
                message: doing.message,
            }),
            Some(Update::Finished(finished)) => Self::Ended(finished),
        }
    }
}

/// What a detached start read: the run, or the board's refusal of it.
enum Started {
    Issued(Issued),
    Refused(Failure),
    /// Nothing was sent: see [`Missing`].
    Missing(Missing),
}

/// Comes back to a detached run: by its handle when the node gave one, so that no other run under
/// the same number answers; by its number on a node too old to give handles.
fn attach(
    orion: &orion::Orion,
    call: u64,
    handle: Option<RunHandle>,
) -> Result<orion::actions::Run, OrionError> {
    match handle {
        Some(handle) => orion.attach_handle(handle),
        None => orion.attach(call),
    }
}

/// The words a followed run's `TimedOut` carries: a detached run's only deadline is the one this
/// crate starts it with, so its passing is the board's bound, not the action failing.
pub const DEADLINE_PASSED: &str = "the board stopped the run when its deadline passed (Atlas gives a detached run an hour, the longest an Orion node takes unless set otherwise): that is the board's bound on the run, not the action failing; what the action did is the board's to say";

/// How a followed (detached) run that ended is answered: as [`ended`], except that `TimedOut`
/// says the board's bound passed ([`DEADLINE_PASSED`]), with the board's own words after it.
fn followed_end(finished: Finished) -> Result<Done, ActionError> {
    if finished.outcome != Outcome::TimedOut {
        return ended(finished);
    }
    let code = u16::try_from(finished.code).ok().filter(|code| *code != 0);
    let words = if finished.detail.trim().is_empty() {
        DEADLINE_PASSED.to_owned()
    } else {
        format!("{DEADLINE_PASSED} (the board said: {})", finished.detail)
    };
    Err(ActionError::Failed(Failure::new(
        FailureKind::Other,
        code,
        words,
    )))
}

/// How a run that ended is answered: the board's words for anything that did not succeed. The
/// board's status code is kept on the refusal or failure (zero is none): a Rejected ending carries
/// the lemnosd status it was refused with (BUSY is 16, see [`ActionError::is_lemnos_busy`]).
fn ended(finished: Finished) -> Result<Done, ActionError> {
    let code = u16::try_from(finished.code).ok().filter(|code| *code != 0);
    let detail = finished.detail;
    let reason = |default: &str| {
        if detail.trim().is_empty() {
            default.to_owned()
        } else {
            detail.clone()
        }
    };
    match finished.outcome {
        // Lemnos M8a (in the lock since d6b9361d): a refusal over 60 KiB comes back by name and is passed on verbatim.
        Outcome::Succeeded => Ok(Done {
            output: finished.output,
            detail,
        }),
        // Lemnos M39: a refusal without running is Rejected with the status's code.
        Outcome::Rejected => Err(ActionError::Refused(match code {
            Some(code) => Failure::lemnos(code, reason("the board refused it, without a reason")),
            None => reason("the board refused it, without a reason").into(),
        })),
        Outcome::Cancelled => Err(ActionError::Cancelled(reason("the run was cancelled"))),
        Outcome::Failed => Err(ActionError::Failed(Failure::new(
            FailureKind::Other,
            code,
            reason("the action failed"),
        ))),
        Outcome::TimedOut => Err(ActionError::Failed(Failure::new(
            FailureKind::Other,
            code,
            reason("the action ran out of time"),
        ))),
        // Orion d4010a45: node logic stopped the program and it did not say how it ended. Not done
        // and not "not done": a failure whose reason says the action may or may not have run.
        Outcome::Indeterminate => Err(ActionError::Failed(Failure::new(
            FailureKind::Other,
            code,
            reason(
                "the board stopped the action and could not say how it ended: it may or may not have run",
            ),
        ))),
        Outcome::Unknown(code) => Err(ActionError::Failed(
            reason(&format!("the board ended the action with outcome {code}")).into(),
        )),
    }
}

/// A started run, with the session it was started on. A run number is only known within its
/// session: on a later session the same number may name another run (the node keeps finished
/// answers by number), so a later use of it is checked before it is trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issued {
    call: u64,
    session: u64,
    name: String,
    /// The run's handle, when the node gave one (Orion level 3): it names this run alone.
    handle: Option<RunHandle>,
}

impl Issued {
    /// The run's number on the board, within its session.
    #[must_use]
    pub const fn call(&self) -> u64 {
        self.call
    }

    /// The action the run was started as.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// What a followed run's answer must look like: `Err` says why it is not what was started. A
/// caller states the kind it started (for example a slot id for an install); a list answer for an
/// install is refused by it, never read as a slot.
pub type Contract = fn(&[u8]) -> Result<(), String>;

/// Accepts any answer. Used only where the answer's kind cannot be checked yet, and said so at the
/// call site.
pub fn any_answer(_output: &[u8]) -> Result<(), String> {
    Ok(())
}

/// A followed run's end. `Confirmed`: the run was started on this session, and its answer is the
/// run's own. `Reconnected`: the run was started on an earlier session; its answer is the one the
/// node gave for that number, which may not be this run's, so the caller confirms it by an
/// independent read before it relies on it.
#[derive(Debug)]
pub enum Followed {
    /// The run was started on the session it ended on.
    Confirmed(Done),
    /// The run was started on an earlier session: confirm before using.
    Reconnected(Done),
}

impl Followed {
    /// The run's answer, without the session check. For a caller whose check is not written yet:
    /// each such call site says so where it is.
    #[must_use]
    pub fn into_done(self) -> Done {
        match self {
            Self::Confirmed(done) | Self::Reconnected(done) => done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(outcome: Outcome, code: u32, detail: &str) -> Finished {
        Finished {
            outcome,
            output: Vec::new(),
            detail: detail.to_owned(),
            code,
        }
    }

    #[test]
    fn a_rejected_run_keeps_its_code_and_busy_is_read_from_it() {
        let busy = ended(finished(Outcome::Rejected, 16, "an install is running")).unwrap_err();
        assert!(busy.is_lemnos_busy(), "{busy:?}");
        assert_eq!(busy.board_reason(), Some("an install is running"));
        // The words alone never make it busy.
        let words = ended(finished(
            Outcome::Rejected,
            0,
            "busy: an install is running",
        ))
        .unwrap_err();
        assert!(!words.is_lemnos_busy(), "{words:?}");
        let missing = ended(finished(Outcome::Rejected, 2, "there is no pushed file")).unwrap_err();
        assert!(!missing.is_lemnos_busy());
        assert_eq!(missing.failure().and_then(Failure::code), Some(2));
    }

    #[test]
    fn an_orion_refusal_with_code_16_is_not_a_lemnos_busy() {
        // Orion's DigestMismatch is 16 too: a digest refusal from Orion is not the board being busy.
        let digest = Failure::of(&orion::Error::Client(orion::ClientError::refused(
            orion::types::Error::DigestMismatch,
            "digest does not match",
        )));
        let error = ActionError::Refused(digest);
        assert!(!error.is_lemnos_busy(), "{error:?}");
    }

    #[test]
    fn a_failed_or_cancelled_run_keeps_its_code_when_it_has_one() {
        let failed = ended(finished(Outcome::Failed, 0, "")).unwrap_err();
        assert_eq!(failed.failure().and_then(Failure::code), None);
        let timed = ended(finished(Outcome::TimedOut, 110, "")).unwrap_err();
        assert_eq!(timed.failure().and_then(Failure::code), Some(110));
        assert!(!timed.is_lemnos_busy());
    }

    #[test]
    fn a_followed_run_that_timed_out_says_the_boards_bound_passed() {
        let timed = followed_end(finished(Outcome::TimedOut, 110, "")).unwrap_err();
        assert_eq!(timed.failure().and_then(Failure::code), Some(110));
        assert!(timed.to_string().contains(DEADLINE_PASSED), "{timed}");
        let said = followed_end(finished(Outcome::TimedOut, 110, "stopped")).unwrap_err();
        assert!(
            said.to_string().contains("(the board said: stopped)"),
            "{said}"
        );
        // Every other ending is read as a direct run's.
        let failed = followed_end(finished(Outcome::Failed, 0, "")).unwrap_err();
        assert_eq!(
            failed.to_string(),
            ended(finished(Outcome::Failed, 0, ""))
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn the_lease_note_is_read_by_its_prefix() {
        assert!(lease_ended("lease-ended: holds released"));
        assert!(lease_ended("lease-ended: holds released; cancelled"));
        assert!(!lease_ended("the board says lease-ended"));
    }
}
