//! The simulated Orion node: what a board publishes, answered the way a board answers.
//!
//! - [`node`]: the node. It is an Orion comm core in this process (Orion's own `orion-stubs`),
//!   with node logic that publishes the hello facts a board publishes, registers the I5 slot
//!   and reboot surface, the board's own actions, topics and files, and answers each run. It
//!   can be stopped and started again on the same port. Linux only, as Orion is.
//! - [`slots`]: the root slot rules of Lemnos's slot driver (I5), and [`slots::trial`], the trial
//!   lifecycle (I5f) on the simulated clock. [`bundle`]: the slot bundle and its signature check
//!   (I5g). [`model`]: the board's state (slots, data partition, clock, readings).
//!
//! Test hooks on [`SimNode`] (none of them is a delay; each one changes state at once):
//! - `slot_clock_advance(secs)`: moves the simulated slot clock. A trial confirms when its daemon
//!   has been up for `confirm_uptime_s` and the board is healthy, or falls back at
//!   `confirm_deadline_s`. The node also runs the trial's rules on every tick, so the result is
//!   there once the clock has moved.
//! - `slot_set_healthy(bool)`: the board's health, the one condition the sim models.
//! - `slot_boots_without_daemon(n)`: the next `n` trial boots do not reach the daemon. Each spends
//!   an attempt. A boot is a restart: `trial` makes the first one, and `system.reboot` (or
//!   `slot_power_cut`) makes the next.
//! - `slot_power_cut()`: cuts a trial boot that reached the daemon short. No attempt is spent.
//! - `slot_fail_record_writes(bool)`: the trial record's writes fail: `record_error` is set, and a
//!   fall back keeps its reason in `trial_reason` (the boot partition's `trial-reason`).
//! - `slot_config(|config| ..)`: the I5f config keys (`auto_confirm`, `confirm_uptime_s`,
//!   `confirm_deadline_s`, `reboot_at_deadline`, `trial_attempts`).
//! - `slot_follow_boot(bool)`: the hello's `version.slot` follows the running slot (a boot moves it,
//!   a confirm does not). Off by default.
//! - `runs_started(action)`: how many runs of an action the node took, refused ones included.
//!
//! The file pushes are not counted per byte: the stub's file receiver keeps no per-chunk record.
//! What the shelf holds is `shelf()`.
//! - [`script`]: scripted failures: refuse, fail, stall, restart, and link faults.
//! - [`names`]: the one table of names the node answers to and publishes.
//!
//! `testing/sim-node/README.md` says what the node does not do, and maps each C-series brief
//! to the names it uses. The mock devices that drivers list are in `atlas-mock`.

pub mod access;
pub mod actions;
pub mod bundle;
pub mod describe;
mod lease;
pub mod model;
pub mod names;
#[cfg(target_os = "linux")]
pub mod node;
#[cfg(target_os = "linux")]
pub mod readings;
pub mod script;
pub mod slots;

pub use lease::{PERIOD as LEASE_PERIOD, STALL as LEASE_STALL};
#[cfg(target_os = "linux")]
pub use node::{DEADLINE, Hello, Kind, Mode, NODE_NAME, SimNode, call};
pub use script::{Behaviour, LinkFault, Script};
pub use slots::{Slots, Status};
