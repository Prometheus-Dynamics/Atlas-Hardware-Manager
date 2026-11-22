//! The Orion side: a flasher node's session as a [`FlasherBoard`], and the [`Flasher`] that finds the node.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_actions::{Action, ActionError, BoardActions, Reported, any_answer};
use atlas_files::{PushError, PushKind, PushSource, Stage};
use atlas_flash_flow::{FirstFlash, Flasher, Say, StepError, Target};
use atlas_jobs::Jobs;
use atlas_session::{Backoff, Handle, Presence};
use orion::node::Node;

use crate::bridge::{BridgeFlash, Bytes, CallError, FlasherBoard};

/// Orion's word in `hello.state` for a board in its flasher image.
pub const FLASHER_STATE: &str = "flasher";
/// The hello fact that names the board (Lemnos's identity, `system.identity/1`).
pub const SERIAL_FACT: &str = "serial";
/// Orion's field type number for text.
const TEXT: u8 = 4;

/// A flasher node through its Orion session: file pushes as `atlas-files` jobs, calls through the bridge.
#[derive(Clone, Debug)]
pub struct OrionBoard {
    handle: Handle,
    kind: Arc<PushKind>,
    jobs: Jobs,
}

impl OrionBoard {
    /// The node `handle` keeps a session with. `kind` is the push kind registered with `jobs`.
    #[must_use]
    pub const fn new(handle: Handle, kind: Arc<PushKind>, jobs: Jobs) -> Self {
        Self { handle, kind, jobs }
    }
}

/// Cancels a push that is dropped before it ended: a flow cancelled mid-write leaves nothing running.
struct Running(Option<atlas_files::Push>);

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(push) = self.0.take() {
            push.cancel();
        }
    }
}

fn action(name: String, job: bool) -> Action {
    Action {
        name,
        label: Reported::NotReported,
        description: Reported::NotReported,
        args: Reported::NotReported,
        long_running: Reported::Known(job),
    }
}

/// Sorts an action error into what the write does next.
fn sort(error: ActionError) -> CallError {
    let words = error.to_string();
    match error {
        ActionError::NotConnected(_) => CallError::NotSent(words),
        ActionError::StartInDoubt { .. }
        | ActionError::Lost { .. }
        | ActionError::NotAnswering(_)
        | ActionError::HoldLapsed { .. }
        | ActionError::Unknown { .. }
        | ActionError::NotFinished { .. }
        | ActionError::Unconfirmed { .. } => CallError::Lost(words),
        _ => CallError::Refused(words),
    }
}

/// Reads the bridge's progress message for a job, `"<done> of <total>"`.
fn bytes_of(message: &str) -> Option<(u64, u64)> {
    let (done, total) = message.split_once(" of ")?;
    Some((done.trim().parse().ok()?, total.trim().parse().ok()?))
}

#[async_trait]
impl FlasherBoard for OrionBoard {
    async fn push(
        &self,
        destination: &str,
        source: Box<dyn PushSource>,
        progress: &Bytes<'_>,
    ) -> Result<String, StepError> {
        let push = atlas_files::submit(&self.kind, &self.jobs, &self.handle, destination, source)
            .await
            .map_err(|error| match error {
                PushError::Destination(_) | PushError::Source(_) | PushError::Job(_) => {
                    StepError::Refused(error.to_string())
                }
            })?;
        let mut changes = push.watch();
        let mut running = Running(Some(push));
        loop {
            let now = changes.borrow_and_update().clone();
            progress(now.sent, now.total);
            match now.stage {
                Stage::Committed => {
                    // Taken out, so the guard does not cancel a push that ended.
                    let answer = running.0.take().and_then(|push| push.answer());
                    return Ok(answer.unwrap_or_default());
                }
                // A push waits through a lost session by itself; it fails only for good.
                Stage::Failed { reason } => return Err(StepError::Refused(reason)),
                Stage::Cancelled => return Err(StepError::Cancelled),
                Stage::Queued
                | Stage::Sending
                | Stage::Resending { .. }
                | Stage::WaitingForBoard { .. } => {}
            }
            if changes.changed().await.is_err() {
                return Err(StepError::Again(
                    "the push's progress ended without an outcome".to_owned(),
                ));
            }
        }
    }

    async fn call(
        &self,
        device: &str,
        words: &[&str],
        job: bool,
        progress: &Bytes<'_>,
    ) -> Result<Vec<u8>, CallError> {
        let actions = BoardActions::new(self.handle.clone());
        let call = action(format!("lemnos.{device}.call"), job);
        let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        if !job {
            return actions
                .run(&call, &words)
                .await
                .map(|done| done.output)
                .map_err(sort);
        }
        let issued = actions.start(&call, &words).await.map_err(sort)?;
        // A lost session leaves the job running on the board: follow it again, never giving up, with the capped
        // backoff.
        let mut backoff = Backoff::default();
        loop {
            match actions
                .follow(&issued, any_answer, |seen| {
                    if let Some((done, total)) = bytes_of(&seen.message) {
                        progress(done, total);
                    }
                })
                .await
            {
                Ok(followed) => return Ok(followed.into_done().output),
                Err(ActionError::Lost { .. }) => tokio::time::sleep(backoff.next_delay()).await,
                Err(error) => return Err(sort(error)),
            }
        }
    }
}

/// Where the node that may be the target's flasher is reached. The way to a node on a board's USB gadget is
/// Orion's, so this is a seam: the caller gives the session.
#[async_trait]
pub trait Reach: Send + Sync {
    /// The session to the node that may be `target`'s flasher. `Err(Again)` while there is none to try.
    async fn reach(&self, target: &Target, say: &Say<'_>) -> Result<Handle, StepError>;
}

/// Finds the flasher over Orion: the node [`Reach`] gives, once its hello says it is the flasher of the target.
#[derive(Debug)]
pub struct OrionFlasher<R> {
    reach: R,
    kind: Arc<PushKind>,
    jobs: Jobs,
}

impl<R: Reach> OrionFlasher<R> {
    /// The flasher reached through `reach`. `kind` is the push kind registered with `jobs`.
    pub const fn new(reach: R, kind: Arc<PushKind>, jobs: Jobs) -> Self {
        Self { reach, kind, jobs }
    }
}

fn fact_text<'a>(node: &'a Node, name: &str) -> Option<&'a str> {
    let fact = node.facts.iter().find(|fact| fact.name == name)?;
    (fact.ty == TEXT)
        .then(|| std::str::from_utf8(&fact.value).ok())
        .flatten()
}

/// Whether a node's hello makes it `target`'s flasher. Another board is refused (it does not change by waiting); a
/// node not yet in the flasher state is waited for.
///
/// # Errors
///
/// [`StepError::Refused`] for another board, [`StepError::Again`] for one not yet the flasher.
pub fn is_flasher_of(
    state: Option<&str>,
    serial: Option<&str>,
    target: &Target,
) -> Result<(), StepError> {
    let wanted = target
        .key
        .split_once(':')
        .map_or(target.key.as_str(), |(_, serial)| serial);
    match serial {
        Some(serial) if !serial.eq_ignore_ascii_case(wanted) => {
            return Err(StepError::Refused(format!(
                "the flasher that answers is board {serial}, not {wanted}"
            )));
        }
        Some(_) => {}
        None => {
            return Err(StepError::Again(
                "the node has not said which board it is yet".to_owned(),
            ));
        }
    }
    match state {
        Some(FLASHER_STATE) => Ok(()),
        Some(other) => Err(StepError::Again(format!(
            "board {wanted}'s node is {other}, not the flasher yet"
        ))),
        None => Err(StepError::Again(format!(
            "board {wanted}'s node does not say its state; a flasher says {FLASHER_STATE}"
        ))),
    }
}

#[async_trait]
impl<R: Reach> Flasher for OrionFlasher<R> {
    async fn find(&self, target: &Target, say: &Say<'_>) -> Result<Arc<dyn FirstFlash>, StepError> {
        let handle = self.reach.reach(target, say).await?;
        let node = match handle.presence() {
            Presence::Found(node) => node,
            Presence::Lost(reason) => {
                return Err(StepError::Again(format!(
                    "no flasher answers yet: {reason}"
                )));
            }
            other => {
                return Err(StepError::Again(format!(
                    "no flasher answers yet: {other:?}"
                )));
            }
        };
        is_flasher_of(node.state(), fact_text(&node, SERIAL_FACT), target)?;
        say(format!(
            "the flasher of board {} answers as {}",
            target.key, node.name
        ));
        Ok(Arc::new(BridgeFlash::new(OrionBoard::new(
            handle,
            Arc::clone(&self.kind),
            self.jobs.clone(),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target {
            key: "rpi:a317bcbee5226d57".into(),
            board: "reference".into(),
            family: "rpi".into(),
            port: "3-4.2.1.4.4.2".into(),
        }
    }

    #[test]
    fn only_the_target_s_node_in_the_flasher_state_is_taken() {
        assert_eq!(
            is_flasher_of(Some("flasher"), Some("a317bcbee5226d57"), &target()),
            Ok(())
        );
        assert_eq!(
            is_flasher_of(Some("flasher"), Some("A317BCBEE5226D57"), &target()),
            Ok(())
        );
        assert!(matches!(
            is_flasher_of(Some("flasher"), Some("0123456789abcdef"), &target()),
            Err(StepError::Refused(_))
        ));
        for state in [Some("running"), Some("recovery"), None] {
            assert!(matches!(
                is_flasher_of(state, Some("a317bcbee5226d57"), &target()),
                Err(StepError::Again(_))
            ));
        }
        assert!(matches!(
            is_flasher_of(Some("flasher"), None, &target()),
            Err(StepError::Again(_))
        ));
    }

    #[test]
    fn a_job_s_bytes_are_read_from_the_bridge_s_message() {
        assert_eq!(bytes_of("4096 of 8388608"), Some((4096, 8_388_608)));
        assert_eq!(bytes_of("verifying"), None);
    }

    #[test]
    fn a_lost_answer_is_told_apart_from_one_never_sent() {
        assert!(matches!(
            sort(ActionError::NotConnected("x".into())),
            CallError::NotSent(_)
        ));
        assert!(matches!(
            sort(ActionError::NotAnswering("x".into())),
            CallError::Lost(_)
        ));
        assert!(matches!(
            sort(ActionError::StartInDoubt {
                name: "n".into(),
                reason: "r".into()
            }),
            CallError::Lost(_)
        ));
        assert!(matches!(
            sort(ActionError::BadArguments("x".into())),
            CallError::Refused(_)
        ));
    }
}
