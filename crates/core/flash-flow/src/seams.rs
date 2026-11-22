//! What the flow is given: the board, the image, and the seams each family and the board's side implement.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;

use crate::view::WritePhase;

/// A line for the flash's own log. The flow adds the time and the step.
pub type Say<'a> = dyn Fn(String) + Send + Sync + 'a;

/// The board being flashed, as the device list found it in its boot ROM.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    /// The device key, `family:serial`. Flashing needs a board identity (AGENTS.md).
    pub key: String,
    /// The board id the image must be built for (the board file's `[board] id`).
    pub board: String,
    /// The driver family whose [`Loader`] runs (the family of the key).
    pub family: String,
    /// The USB port the board's boot ROM is on, as the device list shows it.
    pub port: String,
}

/// The first-flash image, as `atlas-image` read it in one pass (D15). Nothing is sent before it is read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImagePlan {
    /// The file, as the person picked it.
    pub file: String,
    /// The board the image's slot names.
    pub board: String,
    /// The slot the image installs; the first boot is watched for it.
    pub slot: String,
    /// The image's raw size in bytes, once decoded.
    pub raw_size: u64,
    /// The raw image's SHA-256, lowercase hex.
    pub sha256: String,
}

/// What a loader run did, for the page.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Loaded {
    /// One line: what was sent, and that the board took it.
    pub words: String,
    /// What the board said about itself while loading, as label and value.
    pub facts: Vec<(String, String)>,
}

/// Where a write is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WriteProgress {
    /// The board's own state (`indicator/1`).
    pub phase: WritePhase,
    /// Bytes done in this phase.
    pub done: u64,
    /// Bytes this phase covers.
    pub total: u64,
}

/// How a write ended on the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WriteReport {
    /// The raw bytes the board wrote.
    pub written: u64,
    /// The bytes the board read back and checked.
    pub verified: u64,
    /// What the board said about the image's signature, in its words.
    pub signature: String,
}

/// Why a step stopped this time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    /// It can change by itself (no answer yet, a lost link, a timeout). Tried again after the backoff.
    Again(String),
    /// It will not change by itself (the wrong board, the image refused, an action the board does not have). The
    /// flow waits for the person, so it is never retried in a loop.
    Refused(String),
    /// The person cancelled.
    Cancelled,
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Again(words) | Self::Refused(words) => f.write_str(words),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for StepError {}

/// One chip family's way from its boot ROM to the flasher running in RAM. Nothing is written to the board's storage.
/// Safe to run again after any failure: what it sends lives in the board's memory.
#[async_trait]
pub trait Loader: Send + Sync {
    /// The driver family whose boards this loads.
    fn family(&self) -> &str;
    /// What the family's boot ROM is called on the page.
    fn label(&self) -> &str;
    /// Starts the flasher on the target.
    async fn load(&self, target: &Target, say: &Say<'_>) -> Result<Loaded, StepError>;
}

/// Finds the flasher once it runs: the board's Orion node in the flasher state, with the board's identity.
#[async_trait]
pub trait Flasher: Send + Sync {
    /// Waits for the target's flasher node and hands back its first-flash write. A node with another identity is
    /// never taken.
    async fn find(&self, target: &Target, say: &Say<'_>) -> Result<Arc<dyn FirstFlash>, StepError>;
}

/// The board's first-flash write, through its flasher node (Lemnos `system.flash/1`).
#[async_trait]
pub trait FirstFlash: Send + Sync {
    /// Sends the image, has the board write and verify it, and reports the board's phases as they come. A call that
    /// fails may be made again: the board writes from offset 0.
    async fn write(
        &self,
        image: &ImagePlan,
        progress: &(dyn Fn(WriteProgress) + Send + Sync),
        say: &Say<'_>,
    ) -> Result<WriteReport, StepError>;
    /// Restarts the board into the slot it wrote. The flow asks once.
    async fn reboot(&self, say: &Say<'_>) -> Result<(), StepError>;
}

/// Watches the board come back as an ordinary node.
#[async_trait]
pub trait SlotWatch: Send + Sync {
    /// Returns once the board's node says `hello.version.slot` is `slot`, with the node's words for the page.
    async fn booted(&self, target: &Target, slot: &str, say: &Say<'_>)
    -> Result<String, StepError>;
}
