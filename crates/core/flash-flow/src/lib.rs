//! Flash a blank board (D6). One flow for every chip family, decided 2026-11-13:
//!
//! 1. **Image.** The first-flash image is read before anything is sent (`atlas-image`, D15). The flow takes what was
//!    read as an [`ImagePlan`] and refuses an image built for another board here, before a byte reaches the board.
//! 2. **Board in its boot ROM.** The [`Target`]: the board's key, its family and its USB port.
//! 3. **Loader.** The family's [`Loader`] starts the flasher image from RAM. The Pi has no loader in D6's sense: its
//!    boot files start the flasher directly through `atlas-rpiboot`. Rockchip runs the maskrom download and then
//!    fastboot.
//! 4. **Flasher running.** [`Flasher::find`] waits for the board's Orion node in the flasher state
//!    (`hello.state = flasher`, D6 step 4) and hands back its [`FirstFlash`].
//! 5. **Write.** [`FirstFlash::write`]: the image goes to the board, Lemnos's `system.flash/1` writes and verifies it
//!    on the board, and progress comes back in the board's own `indicator/1` states ([`WritePhase`]). Atlas writes
//!    nothing to the board's storage itself.
//! 6. **First boot.** [`FirstFlash::reboot`], then [`SlotWatch::booted`]: the flow ends when the board's node says
//!    `hello.version.slot` is the slot just written.
//!
//! The failure rule (AGENTS.md): a step never gives up on its own. A failure that can change by itself is tried again
//! after a wait that doubles from 1 s up to [`MAX_BACKOFF`]. A refusal that cannot change by itself (the wrong
//! board, an image the board refuses) waits for the person, so it is never retried in a loop. The person can stop
//! the retries, try again now, or cancel, through [`Control`].
//!
//! A step that has written to the board is never retried blindly:
//! - A write that fails starts again from the manifest. A full write again is safe: the board writes from offset 0
//!   and verifies.
//! - The reboot is asked for once. After that the flow only watches for the slot.
//!
//! Everything the page shows is one [`FlowView`], published whole on every change through the observer the caller
//! passes. The page names its steps after [`StepKey`], and its write states after the board's indicator states.
//!
//! This crate does no I/O and names no family, board or USB id. The seams are implemented by the family drivers (for
//! the Pi, `atlas-rpi` over `atlas-rpiboot`) and by the Orion side of the first-flash write.

mod control;
mod flow;
mod seams;
mod view;

pub use control::Control;
pub use flow::{Flow, FlowEnd, MAX_BACKOFF};
pub use seams::{
    FirstFlash, Flasher, ImagePlan, Loaded, Loader, Say, SlotWatch, StepError, Stream, Target,
    WriteProgress, WriteReport,
};
pub use view::{FlowView, LogLine, StepKey, StepState, StepView, WritePhase};
