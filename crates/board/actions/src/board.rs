//! The board's actions, run on its node through the session.
//!
//! Every call goes through the session's [`Handle::with_client`] (a read) or [`Handle::with_bridge`]
//! (a run, or its cancel, which renews the board's lease), so it has the session's call
//! wait and never runs after its caller gave up.
//!
//! Two ways to run an action:
//!
//! - A **short** action is one call ([`BoardActions::run`], [`BoardActions::catalog`]): started
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
//! - [`BoardActions::catalog`] reads the list. It changes nothing, so it may be made again.
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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use atlas_session::{Failure, FailureKind, Handle, Hold, Presence, SessionError};
use orion::actions::{ActionCall, Finished, Outcome, Run, RunHandle, Target, Update};
use orion::types::Error as CoreError;
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError};

use crate::args;
use crate::catalog::{Action, Catalog, Route};
use crate::error::ActionError;

/// The name the board answers its action list to. A provisional name, agreed with Lemnos with the
/// list's format (see `crates/testing/sim-node/src/actions.rs`).
pub const CATALOG_ACTION: &str = "actions.list";
/// How long one poll of a detached run waits for news before the session is looked at again.
pub const POLL: Duration = Duration::from_millis(250);
/// The longest a short action may take in one call. Half the call wait is used when that is less.
pub const DIRECT_WAIT: Duration = Duration::from_secs(2);

/// What a hold lapse reports as the board's words when the run had not ended yet.
const NOT_FINISHED: &str = "not finished";
/// The outcome of a run the board restarted while it ran (Orion's `RunGone`).
pub const RESTARTED_REASON: &str = "the board restarted while this ran; outcome unknown";
/// The words for a run number the board does not have (Orion's `NotFound`).
pub const NO_SUCH_RUN: &str = "no such run";
/// Why a run started on an earlier session is not followed or cancelled by its number.
const NO_HANDLE_REASON: &str = "the run was started on an earlier session and the node did not name it by handle, so its number is not used: the outcome is unknown";

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
    /// The node restarted since the run was started: the run is gone, its outcome is unknown.
    Restarted,
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

    /// Reads the board's action list, as one short call. A board's refusal of the read (a reply
    /// over the bridge's size bound, or a node that lists nothing) comes back as
    /// [`ActionError::Refused`] with the board's reason, verbatim. The simulated node answers it
    /// as Lemnos M8 does.
    ///
    /// # Errors
    ///
    /// [`ActionError::Refused`] (a refusal, with the board's reason; a plain Orion node refuses
    /// the read because it lists nothing); [`ActionError::NotAnswering`] when the node did not
    /// answer; [`ActionError::BadCatalog`] when the list is not one Atlas reads.
    pub async fn catalog(&self) -> Result<Catalog, ActionError> {
        // Retry: reading the list changes nothing, so the caller may make it again.
        let done = self
            .direct(CATALOG_ACTION, Vec::new())
            .await
            .map_err(|error| match error {
                // The board's own words, verbatim: a size refusal is never reported as "no actions".
                refused @ (ActionError::Refused(_)
                | ActionError::Failed(_)
                | ActionError::Cancelled(_)) => refused,
                other => ActionError::NotAnswering(other.to_string()),
            })?;
        // An out-buffer reply through the bridge (Lemnos M8a, in the lock since d6b9361d).
        let text = String::from_utf8(done.output)
            .map_err(|_| ActionError::BadCatalog("the list is not text".into()))?;
        Catalog::parse(&text).map_err(|error| ActionError::BadCatalog(error.to_string()))
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
    /// or [`ActionError::StartInDoubt`].
    pub async fn start(&self, action: &Action, words: &[String]) -> Result<Issued, ActionError> {
        args::check(action, words)?;
        self.start_encoded(&action.name, args::encode(words)).await
    }

    /// Starts `name` with `encoded` arguments, detached. The arguments must already be checked.
    pub(crate) async fn start_encoded(
        &self,
        name: &str,
        encoded: Vec<u8>,
    ) -> Result<Issued, ActionError> {
        let node_name = self.node_name()?;
        let owned = name.to_owned();
        let started = name.to_owned();
        // Read before the start is sent: a session that changes meanwhile makes the run look
        // older than it is, so a later use treats it as another session's (the safe side).
        let session = self.handle.session();
        let sent = self
            .handle
            .with_bridge(move |orion| {
                let mut run = orion.run(&ActionCall {
                    name: &owned,
                    target: Target::Node(node_name),
                    args: &encoded,
                    deadline_ms: 0,
                    detached: true,
                })?;
                let issued = Issued {
                    call: run.call(),
                    session,
                    name: started.clone(),
                    handle: Arc::new(Mutex::new(None)),
                };
                // The handle comes with the node's started frame: wait for it, briefly. A refusal or
                // a silent node leaves it unknown; the follow learns it later, or it stays unknown.
                let until = Instant::now() + START_HANDLE_WAIT;
                while issued.handle().is_none() && Instant::now() < until {
                    if run.wait(Duration::from_millis(10)).is_err() {
                        break;
                    }
                    issued.learn(&run);
                }
                issued.learn(&run);
                Ok(issued)
            })
            .await;
        // Retry: never by this crate. A start that was sent and not answered may already be on
        // the board, so the caller is told and decides.
        sent.map_err(|error| match error {
            SessionError::Gone(reason) => ActionError::StartInDoubt {
                name: name.to_owned(),
                reason,
            },
            SessionError::Failed(reason) => ActionError::Failed(reason),
        })
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
        // Retry: following is a read. A lost session returns `Lost` with the run's number, and
        // following that number on the new session resumes it; the run is never started again.
        loop {
            let known = issued.handle();
            let same_session = self.handle.session() == issued.session;
            // Never by bare number across a session change: the outcome is unknown instead.
            if !may_use_number(known.is_some(), same_session) {
                return Err(ActionError::Unconfirmed {
                    call,
                    reason: NO_HANDLE_REASON.to_owned(),
                    ended: None,
                });
            }
            let learned = issued.clone();
            let seen = self
                .handle
                .with_client(move |orion| {
                    let mut run = match known {
                        Some(handle) => orion.attach_handle(handle)?,
                        None => orion.attach(call)?,
                    };
                    learned.learn(&run);
                    match run.wait(POLL) {
                        Ok(update) => {
                            learned.learn(&run);
                            Ok(Seen::of(update))
                        }
                        Err(ClientError::Refused { error, detail }) => {
                            Ok(refusal_seen(error, detail))
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
                    reason: NOT_FINISHED.to_owned(),
                });
            }
            match seen {
                Ok(Seen::Restarted) => {
                    return Err(ActionError::Unconfirmed {
                        call,
                        reason: RESTARTED_REASON.to_owned(),
                        ended: None,
                    });
                }
                Ok(Seen::Quiet) => {}
                Ok(Seen::Doing(progress)) => on_progress(&progress),
                Ok(Seen::Ended(finished)) => {
                    // The board says the caller's lease ended before this reply: its holds are gone.
                    if lease_ended(&finished.detail) {
                        return Err(ActionError::HoldLapsed {
                            call,
                            reason: finished.detail,
                        });
                    }
                    let result = ended(finished);
                    let on_issuing_session = self.handle.session() == issued.session;
                    let done = match (hold, result) {
                        (Some(hold), Err(error)) if hold.lapsed() => {
                            return Err(ActionError::HoldLapsed {
                                call,
                                reason: error.to_string(),
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
        let known = issued.handle();
        // A number from an earlier session may name another run on the board: with no handle it is
        // not sent a cancel, since a cancel to the wrong run stops someone else's work.
        if !may_use_number(known.is_some(), self.handle.session() == issued.session) {
            return Err(ActionError::Unconfirmed {
                call,
                reason: NO_HANDLE_REASON.to_owned(),
                ended: None,
            });
        }
        let learned = issued.clone();
        let refused: Option<(CoreError, String)> = self
            .handle
            .with_bridge(move |orion| {
                let mut run = match known {
                    Some(handle) => orion.attach_handle(handle)?,
                    None => orion.attach(call)?,
                };
                learned.learn(&run);
                // A handle the node no longer knows is refused on the attach, which comes back on the
                // next read: a cancel is sent only when that read does not refuse it.
                match run.wait(CANCEL_CHECK) {
                    Ok(_) => {}
                    Err(ClientError::Refused { error, detail }) => {
                        return Ok(Some((error, detail)));
                    }
                    Err(error) => return Err(OrionError::Client(error)),
                }
                run.cancel().map_err(OrionError::Client)?;
                Ok(None)
            })
            .await
            .map_err(|error| match error {
                SessionError::Gone(reason) => ActionError::Lost {
                    call,
                    reason: Failure::other(reason),
                },
                SessionError::Failed(reason) => ActionError::Lost { call, reason },
            })?;
        match refused {
            None => Ok(()),
            Some((error, detail)) => Err(cancel_refusal(call, error, detail)),
        }
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
        let answered = self
            .handle
            .with_bridge(move |orion| {
                let mut run = orion.run(&ActionCall {
                    name: &owned,
                    target: Target::Node(node_name),
                    args: &encoded,
                    deadline_ms: 0,
                    detached: false,
                })?;
                match run.finish(wait) {
                    Ok(finished) => Ok(Direct::Ended(finished)),
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
            Ok(Direct::Late) => Err(ActionError::NotFinished {
                name: name.to_owned(),
                wait,
            }),
            Err(SessionError::Gone(reason)) => Err(ActionError::Unknown {
                name: name.to_owned(),
                reason: Failure::other(reason),
            }),
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

/// How a run that ended is answered: the board's words for anything that did not succeed. The
/// board's status code is kept on the refusal or failure (zero is none): a Rejected ending carries
/// the lemnosd status it was refused with (BUSY is 16, see [`ActionError::is_busy`]).
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
            Some(code) => Failure::new(
                FailureKind::Refused,
                Some(code),
                reason("the board refused it, without a reason"),
            ),
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
        Outcome::Unknown(code) => Err(ActionError::Failed(
            reason(&format!("the board ended the action with outcome {code}")).into(),
        )),
    }
}

/// What a follow's refusal from the node means: a run the node restarted past is gone (its outcome is
/// unknown), a number the node does not have is no such run, and any other refusal keeps the board's words.
fn refusal_seen(error: CoreError, detail: String) -> Seen {
    match error {
        CoreError::RunGone => Seen::Restarted,
        CoreError::NotFound => Seen::Refused(Failure::of_core(error, NO_SUCH_RUN.to_owned())),
        _ => Seen::Refused(Failure::of_core(error, detail)),
    }
}

/// What a cancel's refusal from the node means, by the same rule as [`refusal_seen`].
fn cancel_refusal(call: u64, error: CoreError, detail: String) -> ActionError {
    match error {
        CoreError::RunGone => ActionError::Unconfirmed {
            call,
            reason: RESTARTED_REASON.to_owned(),
            ended: None,
        },
        CoreError::NotFound => {
            ActionError::Refused(Failure::of_core(error, NO_SUCH_RUN.to_owned()))
        }
        _ => ActionError::Refused(Failure::of_core(error, detail)),
    }
}

/// How long a start waits for the node's started frame, which names the run's handle. A node that
/// does not name it in time leaves the handle unknown, and a later follow or cancel learns it.
pub const START_HANDLE_WAIT: Duration = Duration::from_millis(200);
/// How long a cancel waits for the node's answer to naming the run by its handle (a refusal such
/// as "the node restarted" comes back to that attach, not to the cancel).
pub const CANCEL_CHECK: Duration = Duration::from_millis(500);

/// Whether a run's number may be used to follow or cancel it. A handle, once the node named it,
/// names the run whatever the session; a number alone names a run only on the session it was
/// started on (the node keeps finished answers by number, so a later session's same number may be
/// another run). This is the one rule every follow and cancel is checked against.
#[must_use]
pub const fn may_use_number(handle_known: bool, same_session: bool) -> bool {
    handle_known || same_session
}

/// A started run, with the session it was started on. A run number is only known within its
/// session: on a later session the same number may name another run (the node keeps finished
/// answers by number), so a later use of it is checked before it is trusted.
///
/// The run's handle (the node names it after its started frame) is kept here too, in memory,
/// beside the number. Every clone shares the one handle, so a job or an install that keeps the
/// `Issued` keeps the handle. Nothing is saved to disk: after an Atlas restart the run cannot be
/// followed again (see docs/prs/PINS-2.md).
#[derive(Clone, Debug)]
pub struct Issued {
    call: u64,
    session: u64,
    name: String,
    handle: Arc<Mutex<Option<RunHandle>>>,
}

impl Issued {
    /// The run's number on the board, within its session.
    #[must_use]
    pub const fn call(&self) -> u64 {
        self.call
    }

    /// The run's handle, once the node has named it. `None` until then, and for a node below the
    /// handle's interface level.
    #[must_use]
    pub fn handle(&self) -> Option<RunHandle> {
        *self.handle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps the handle the run named, if it has named one yet.
    fn learn(&self, run: &Run) {
        if let Some(handle) = run.handle() {
            *self.handle.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
        }
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
        assert!(busy.is_busy(), "{busy:?}");
        assert_eq!(busy.board_reason(), Some("an install is running"));
        // The words alone never make it busy.
        let words = ended(finished(
            Outcome::Rejected,
            0,
            "busy: an install is running",
        ))
        .unwrap_err();
        assert!(!words.is_busy(), "{words:?}");
        let missing = ended(finished(Outcome::Rejected, 2, "there is no pushed file")).unwrap_err();
        assert!(!missing.is_busy());
        assert_eq!(missing.failure().and_then(Failure::code), Some(2));
    }

    #[test]
    fn a_failed_or_cancelled_run_keeps_its_code_when_it_has_one() {
        let failed = ended(finished(Outcome::Failed, 0, "")).unwrap_err();
        assert_eq!(failed.failure().and_then(Failure::code), None);
        let timed = ended(finished(Outcome::TimedOut, 110, "")).unwrap_err();
        assert_eq!(timed.failure().and_then(Failure::code), Some(110));
        assert!(!timed.is_busy());
    }

    #[test]
    fn the_lease_note_is_read_by_its_prefix() {
        assert!(lease_ended("lease-ended: holds released"));
        assert!(lease_ended("lease-ended: holds released; cancelled"));
        assert!(!lease_ended("the board says lease-ended"));
    }

    fn issued(call: u64, session: u64) -> Issued {
        Issued {
            call,
            session,
            name: "install_file".to_owned(),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn a_number_is_used_only_with_a_handle_or_on_its_own_session() {
        // The rule every follow and cancel is checked against: a handle names the run whatever the
        // session; a bare number only on the session it was started on.
        assert!(
            !may_use_number(false, false),
            "no handle, later session: never by number"
        );
        assert!(
            may_use_number(false, true),
            "no handle, same session: the number"
        );
        assert!(
            may_use_number(true, false),
            "a handle: names the run on any session"
        );
        assert!(may_use_number(true, true));
    }

    #[test]
    fn the_handle_is_shared_by_every_clone_of_the_issued_run() {
        // A job or an install keeps a clone of the Issued run: a handle learned through one is seen through all.
        let first = issued(7, 1);
        let kept = first.clone();
        assert_eq!(kept.handle(), None, "unknown until the node names it");
        let handle = RunHandle::said(3, 7).expect("a handle the node named");
        *first.handle.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
        assert_eq!(
            kept.handle(),
            Some(handle),
            "the kept clone has the handle too"
        );
        assert_eq!(kept.call(), 7, "the number is kept beside it");
    }

    #[test]
    fn a_restarted_run_refusal_is_the_typed_unknown_outcome() {
        // Orion's RunGone: the node restarted while the run ran. Never a result, never a failure.
        match refusal_seen(CoreError::RunGone, String::new()) {
            Seen::Restarted => {}
            _ => panic!("RunGone must be Restarted"),
        }
        match cancel_refusal(7, CoreError::RunGone, String::new()) {
            ActionError::Unconfirmed {
                call,
                reason,
                ended,
            } => {
                assert_eq!(call, 7);
                assert_eq!(reason, RESTARTED_REASON);
                assert!(
                    ended.is_none(),
                    "an unknown outcome has no run's own refusal"
                );
            }
            other => panic!("a cancel of a restarted run is unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_run_is_no_such_run_with_its_kind_kept() {
        // Orion's NotFound: the number or handle names no run on the board.
        match refusal_seen(CoreError::NotFound, "detail".to_owned()) {
            Seen::Refused(failure) => assert_eq!(failure.text(), NO_SUCH_RUN),
            _ => panic!("NotFound must be a refusal"),
        }
        match cancel_refusal(7, CoreError::NotFound, "detail".to_owned()) {
            ActionError::Refused(failure) => {
                assert_eq!(failure.text(), NO_SUCH_RUN);
                assert_eq!(
                    failure.kind(),
                    FailureKind::Refused,
                    "NotFound stays a refusal, not a denial"
                );
            }
            other => panic!("a cancel of no run is a refusal, got {other:?}"),
        }
    }
}
