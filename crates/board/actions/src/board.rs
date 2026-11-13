//! The board's actions, run on its node through the session.
//!
//! Every call goes through the session's [`Handle::with_client`], so it has the session's call
//! wait and never runs after its caller gave up. Each action is started **detached**, so the
//! board keeps the run if the session ends, and the run is followed in short polls, each one a
//! call of its own. That is what makes a lost session survivable: the run is on the board under
//! its call number, and [`BoardActions::follow`] comes back to it instead of starting it again.
//!
//! Retry safety of each call:
//!
//! - [`BoardActions::catalog`] reads the list. It changes nothing, so it may be made again.
//! - [`BoardActions::start`] starts an action. It is **never retried by this crate**: when the
//!   node does not confirm the start, the error is [`ActionError::StartInDoubt`], and the caller
//!   decides. The job kind does not start again either (see [`crate::jobs`]).
//! - [`BoardActions::follow`] reads a run's progress and result. Reading changes nothing, so it
//!   may be made again; it is what a lost session resumes with.
//! - [`BoardActions::cancel`] asks a run to stop. Asking again after it has ended changes
//!   nothing, so it may be made again.

use std::time::Duration;

use atlas_session::{Handle, Presence, SessionError};
use orion::actions::{ActionCall, Finished, Outcome, Target, Update};
use orion::types::NodeName;
use orion::{ClientError, Error as OrionError};
use tokio::time::Instant;

use crate::args;
use crate::catalog::{Action, Catalog, Route};
use crate::error::ActionError;

/// The name the board answers its action list to. LOCK: a provisional name, agreed with Lemnos
/// with the list's format (see `crates/testing/sim-node/src/actions.rs`).
pub const CATALOG_ACTION: &str = "actions.list";
/// How long one poll of a run waits for news before the session is looked at again.
pub const POLL: Duration = Duration::from_millis(250);
/// How long reading the board's list may take.
pub const CATALOG_LIMIT: Duration = Duration::from_secs(30);

/// A run that ended successfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    /// What the action returned. LOCK: until Lemnos M8 a method that returns through an out
    /// buffer is refused by the bridge, so an output that comes from one is not readable on a
    /// real board yet.
    pub output: Vec<u8>,
    /// What the board said about the result, if it said anything.
    pub detail: String,
}

/// How far a run has got, as the board last reported it.
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

/// What one poll of a run saw.
enum Seen {
    Quiet,
    Doing(Progress),
    Ended(Finished),
    Refused(String),
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

    /// Reads the board's action list.
    ///
    /// LOCK: until Lemnos M8 the bridge's `call` cannot call a method that returns through an out
    /// buffer, so a real board refuses this read (`INVALID_ARGUMENT`) and it shows as
    /// [`ActionError::NoCatalog`]. The simulated node answers it as M8 will.
    ///
    /// # Errors
    ///
    /// [`ActionError::NoCatalog`] when the node lists no actions (a plain Orion node, or a refusal
    /// with the node's reason); [`ActionError::NotAnswering`] when the node did not answer; and
    /// [`ActionError::BadCatalog`] when the list is not one Atlas reads.
    pub async fn catalog(&self) -> Result<Catalog, ActionError> {
        // Retry: reading the list changes nothing, so the caller may make it again.
        // LOCK: until Lemnos M8 the bridge refuses this read: the list comes back through an out buffer.
        let call = self
            .start_encoded(CATALOG_ACTION, Vec::new())
            .await
            .map_err(|error| ActionError::NotAnswering(error.to_string()))?;
        let done = self
            .follow_until(call, Some(CATALOG_LIMIT), |_| {})
            .await
            .map_err(|error| match error {
                ActionError::Refused(reason)
                | ActionError::Failed(reason)
                | ActionError::Cancelled(reason) => ActionError::NoCatalog(reason),
                ActionError::Lost { reason, .. } => ActionError::NotAnswering(reason),
                other => other,
            })?;
        // LOCK: until Lemnos M8 the output of this read is the out buffer the bridge refuses.
        let text = String::from_utf8(done.output)
            .map_err(|_| ActionError::BadCatalog("the list is not text".into()))?;
        Catalog::parse(&text).map_err(|error| ActionError::BadCatalog(error.to_string()))
    }

    /// Starts `action` with `words`, detached, and returns its run number on the board.
    ///
    /// The words are checked against the action's description first; nothing is sent when they
    /// do not fit. The start is never retried here: see the module's rules.
    ///
    /// # Errors
    ///
    /// [`ActionError::BadArguments`] (nothing sent), [`ActionError::NotConnected`] (nothing sent),
    /// or [`ActionError::StartInDoubt`] when the node did not confirm the start.
    pub async fn start(&self, action: &Action, words: &[String]) -> Result<u64, ActionError> {
        args::check(action, words)?;
        self.start_encoded(&action.name, args::encode(words)).await
    }

    /// Starts `name` with `encoded` arguments, as [`BoardActions::start`] does. The arguments
    /// must already be checked.
    pub(crate) async fn start_encoded(
        &self,
        name: &str,
        encoded: Vec<u8>,
    ) -> Result<u64, ActionError> {
        // Retry: never by this crate. A start that was sent and not answered is in doubt: the
        // run may already be on the board, so the caller is told and decides.
        let node = match self.handle.presence() {
            Presence::Found(node) => node,
            Presence::Lost(reason) => return Err(ActionError::NotConnected(reason)),
            Presence::Searching => {
                return Err(ActionError::NotConnected("not connected yet".into()));
            }
        };
        let node_name: NodeName = node.name.parse().map_err(|_| {
            ActionError::NotAnswering(format!(
                "the node's name {:?} is not a valid name",
                node.name
            ))
        })?;
        let owned = name.to_owned();
        let sent = self
            .handle
            .with_client(move |orion| {
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
        sent.map_err(|error| match error {
            // A call that did not come back may still have reached the node.
            SessionError::Gone(reason) => ActionError::StartInDoubt {
                name: name.to_owned(),
                reason,
            },
            SessionError::Failed(reason) => ActionError::Failed(reason),
        })
    }

    /// Follows run `call` until the board says how it ended, passing each progress report to
    /// `on_progress`. A lost session returns [`ActionError::Lost`] with the run's number, so the
    /// caller can follow it again once the session is back.
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
        self.follow_until(call, None, on_progress).await
    }

    async fn follow_until(
        &self,
        call: u64,
        limit: Option<Duration>,
        mut on_progress: impl FnMut(&Progress),
    ) -> Result<Done, ActionError> {
        let started = Instant::now();
        // Retry: following is a read. A lost session returns `Lost` with the run's number, and
        // following that number again on the new session resumes it; the run is never started again.
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
            match seen {
                Ok(Seen::Quiet) => {}
                Ok(Seen::Doing(progress)) => on_progress(&progress),
                Ok(Seen::Ended(finished)) => return ended(finished),
                Ok(Seen::Refused(reason)) => return Err(ActionError::Refused(reason)),
                Err(SessionError::Gone(reason) | SessionError::Failed(reason)) => {
                    return Err(ActionError::Lost { call, reason });
                }
            }
            if limit.is_some_and(|limit| started.elapsed() >= limit) {
                return Err(ActionError::Lost {
                    call,
                    reason: "no result within the limit; the run is still on the board".into(),
                });
            }
        }
    }

    /// Asks run `call` to stop. How it ended still comes from [`BoardActions::follow`]: a cancel
    /// that was sent is not yet a cancel that took effect, so the caller checks.
    ///
    /// # Errors
    ///
    /// [`ActionError::Lost`] when the session did not carry the request.
    pub async fn cancel(&self, call: u64) -> Result<(), ActionError> {
        // Retry: asking a run that has ended changes nothing, so a cancel may be sent again.
        self.handle
            .with_client(move |orion| {
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

    /// Runs a short action directly and waits for its result, within `limit`.
    ///
    /// Only an action the board describes as short runs here; a long one, or one whose length
    /// is not reported, is [`ActionError::NeedsJob`]. A run that is cut short is returned as
    /// [`ActionError::Lost`] with its number: it is still on the board.
    ///
    /// # Errors
    ///
    /// As [`BoardActions::start`] and [`BoardActions::follow`].
    pub async fn run(
        &self,
        action: &Action,
        words: &[String],
        limit: Duration,
    ) -> Result<Done, ActionError> {
        if action.route() == Route::Job {
            return Err(ActionError::NeedsJob(action.name.clone()));
        }
        let call = self.start(action, words).await?;
        self.follow_until(call, Some(limit), |_| {}).await
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
        // LOCK: until Lemnos M8 an action whose output comes from an out buffer is refused by the bridge.
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
