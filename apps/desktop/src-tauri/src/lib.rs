//! The Atlas desktop app's Rust side.
//!
//! - [`core`]: the app's state and background work: the inventory (loaded from disk before the
//!   window opens), the driver scan, jobs, settings and the app's own log. Nothing in it waits
//!   on a device.
//! - [`call`]: the one way the UI asks for anything. A call has a name and JSON arguments, is
//!   always async, and every failed one is written to the app's log with what was asked.
//! - [`board`]: the board page's work: one board's readings, logs, actions and slots.
//! - [`view`]: what the UI is sent. State reaches the UI as events carrying whole snapshots, so
//!   the UI never replays steps.
//! - [`drivers`]: the real drivers and the computer's own links.
//! - [`flash`]: the flash flow: the image, the boot ROM loader as a job, and the disk writer.
//! - [`window`]: the Tauri window: one async command (`call`) and one event (`atlas`).
//! - `sim` (feature `sim`): the simulated devices, and the UI bridge that serves the same calls
//!   and events to a browser for the click tests.
//!
//! The UI is in `apps/desktop/src`. It is built from the mockups in `docs/mockups`.

pub mod board;
pub mod call;
pub mod core;
pub mod drivers;
pub mod flash;
pub mod open;
pub mod paths;
pub mod robot;
pub mod settings;
#[cfg(feature = "sim")]
pub mod sim;
pub mod view;
pub mod window;

pub use crate::core::{Core, Mode, Options};

/// The version this build reports (see `build.rs`).
pub const VERSION: &str = env!("ATLAS_APP_VERSION");

/// When this process started, as near as the app can tell: the first time anything asks.
/// `main` asks first.
pub fn started() -> std::time::Instant {
    static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *STARTED.get_or_init(std::time::Instant::now)
}
