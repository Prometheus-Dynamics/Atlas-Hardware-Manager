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
use std::time::Duration;

use atlas_session::{Handle, Hold, Presence, SessionError};
use orion::actions::{ActionCall, Finished, Outcome, Target, Update};
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
    Refused(String),
}

/// What one direct call saw.
enum Direct {
    Ended(Finished),
    Refused(String),
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
    pub async fn start(&self, action: &Action, words: &[String]) -> Result<u64, ActionError> {
        args::check(action, words)?;
        self.start_encoded(&action.name, args::encode(words)).await
    }

    /// Starts `name` with `encoded` arguments, detached. The arguments must already be checked.
    pub(crate) async fn start_encoded(
        &self,
        name: &str,
        encoded: Vec<u8>,
    ) -> Result<u64, ActionError> {
        let node_name = self.node_name()?;
        let owned = name.to_owned();
        let sent = self
            .handle
            .with_bridge(move |orion| {
                let run = orion.run(&ActionCall {
                    name: &owned,
                    target: Target::Node(node_name),
                    args: &encoded,
                    deadline_ms: 0,
                    detached: true,
                })?;
                Ok(run.call())
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
        call: u64,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Done, ActionError> {
        self.follow_inner(call, None, on_progress).await
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
        call: u64,
        hold: &Hold,
        on_progress: impl FnMut(&Progress),
    ) -> Result<Done, ActionError> {
        self.follow_inner(call, Some(hold), on_progress).await
    }

    async fn follow_inner(
        &self,
        call: u64,
        hold: Option<&Hold>,
        mut on_progress: impl FnMut(&Progress),
    ) -> Result<Done, ActionError> {
        // Retry: following is a read. A lost session returns `Lost` with the run's number, and
        // following that number on the new session resumes it; the run is never started again.
        loop {
            let seen = self
                .handle
                .with_client(move |orion| {
                    let mut run = orion.attach(call)?;
                    match run.wait(POLL) {
                        Ok(update) => Ok(Seen::of(update)),
                        Err(ClientError::Refused(error)) => Ok(Seen::Refused(error.to_string())),
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
                Ok(Seen::Quiet) => {}
                Ok(Seen::Doing(progress)) => on_progress(&progress),
                Ok(Seen::Ended(finished)) => {
                    let result = ended(finished);
                    return match (hold, result) {
                        (Some(hold), Err(error)) if hold.lapsed() => Err(ActionError::HoldLapsed {
                            call,
                            reason: error.to_string(),
                        }),
                        (_, result) => result,
                    };
                }
                Ok(Seen::Refused(reason)) => return Err(ActionError::Refused(reason)),
                Err(SessionError::Gone(reason) | SessionError::Failed(reason)) => {
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
    pub async fn cancel(&self, call: u64) -> Result<(), ActionError> {
        self.handle
            .with_bridge(move |orion| {
                let mut run = orion.attach(call)?;
                run.cancel().map_err(OrionError::Client)
            })
            .await
            .map_err(|error| match error {
                SessionError::Gone(reason) | SessionError::Failed(reason) => {
                    ActionError::Lost { call, reason }
                }
            })
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
                    Err(ClientError::Refused(error)) => Ok(Direct::Refused(error.to_string())),
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
            Err(SessionError::Gone(reason) | SessionError::Failed(reason)) => {
                Err(ActionError::Unknown {
                    name: name.to_owned(),
                    reason,
                })
            }
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

/// How a run that ended is answered: the board's words for anything that did not succeed.
fn ended(finished: Finished) -> Result<Done, ActionError> {
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
        Outcome::Rejected => Err(ActionError::Refused(reason(
            "the board refused it, without a reason",
        ))),
        Outcome::Cancelled => Err(ActionError::Cancelled(reason("the run was cancelled"))),
        Outcome::Failed => Err(ActionError::Failed(reason("the action failed"))),
        Outcome::TimedOut => Err(ActionError::Failed(reason("the action ran out of time"))),
        Outcome::Unknown(code) => Err(ActionError::Failed(reason(&format!(
            "the board ended the action with outcome {code}"
        )))),
    }
}
