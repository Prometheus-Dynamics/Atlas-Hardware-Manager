//! What the page reads: the flow's whole state as one value, published on every change.

use serde::Serialize;

use crate::seams::{ImagePlan, Loaded, Target, WriteProgress, WriteReport};

/// The six steps, in order. The page names its steps after these keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepKey {
    /// The image was read and is for this board.
    Image,
    /// The board is in its boot ROM on the port.
    Board,
    /// The family's loader started the flasher from RAM.
    Loader,
    /// The flasher's node answered with the board's identity.
    Flasher,
    /// The board wrote the image and verified it.
    Write,
    /// The board came back running the slot just written.
    Boot,
}

impl StepKey {
    /// Every step, in the order the flow runs them.
    pub const ALL: [Self; 6] = [
        Self::Image,
        Self::Board,
        Self::Loader,
        Self::Flasher,
        Self::Write,
        Self::Boot,
    ];
}

/// A write's phase, as the board's `indicator/1` names it (Lemnos: `idle`, `receiving`, `writing`, `verifying`,
/// `done`, `failed`). Only the three a running write passes through are phases here; done and failed are the step's
/// own end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WritePhase {
    Receiving,
    Writing,
    Verifying,
}

/// Where one step is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum StepState {
    /// Not reached yet.
    Pending,
    /// Running, attempt `attempt` (from 1).
    Running {
        attempt: u32,
        detail: String,
        /// The board's phase and bytes, for the write.
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<WriteProgress>,
    },
    /// It failed in a way that can change by itself, and runs again once the wait is over.
    Retrying {
        attempt: u32,
        error: String,
        /// The wait before the next try, and when it ends, in ms since the flow started.
        retry_in_ms: u64,
        retry_at_ms: u64,
    },
    /// The board or the host refused, and it will not change by itself: it waits for the person (try again, or
    /// cancel).
    Refused { attempt: u32, error: String },
    /// The person stopped the retries; it waits for them to try again.
    Stopped { attempt: u32, error: String },
    /// Done.
    Done { detail: String },
    /// The person cancelled the flow here.
    Cancelled,
}

/// One step on the page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StepView {
    pub key: StepKey,
    #[serde(flatten)]
    pub state: StepState,
}

/// One line of the flash's own log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// Ms since the flow started.
    pub at_ms: u64,
    pub step: StepKey,
    pub text: String,
}

/// How the flow ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "end", rename_all = "kebab-case")]
pub enum FlowEndView {
    /// The board runs the slot just written.
    Booted { slot: String, words: String },
    /// Refused before anything was sent to the board (the image is for another board, or no loader for the family).
    Refused { step: StepKey, words: String },
    /// The person cancelled.
    Cancelled { step: StepKey },
}

/// The whole flow, as the page reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlowView {
    pub target: Target,
    pub image: ImagePlan,
    /// What the boot ROM is called (the loader's label).
    pub boot_rom: String,
    pub steps: Vec<StepView>,
    /// What the loader said about the board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded: Option<Loaded>,
    /// How the write ended on the board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<WriteReport>,
    /// The newest lines last; at most [`crate::flow::LOG_LINES`].
    pub log: Vec<LogLine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<FlowEndView>,
}

impl FlowView {
    /// The state of `key`.
    #[must_use]
    pub fn step(&self, key: StepKey) -> &StepState {
        &self
            .steps
            .iter()
            .find(|step| step.key == key)
            .expect("every step is in the view")
            .state
    }
}
