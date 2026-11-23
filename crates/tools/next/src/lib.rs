//! `atlas-next slots`: the slot cycle on a board that runs a `next` image, from the computer,
//! with nothing but the board's Orion address.
//!
//! This is the milestone's test tool, not the product CLI. It adds no board logic: every command
//! is a call into the crates that already do the work (`atlas-board-slots` for the slots, the
//! trial follower and the install job; `atlas-session` for the connection).
//!
//! The pin rule lives here, in front of every command except `fingerprint` and `pin`:
//!
//! 1. A board never seen before refuses every command and shows its key fingerprint.
//! 2. `pin` records the fingerprint for that address. Nothing else trusts a board.
//! 3. A board that answers with another key refuses, naming both fingerprints.
//! 4. `repin --reflashed` is the only way to replace a pin.
//!
//! [`execute`] is what the binary runs, and what the tests run. Its output goes to any `Write`.

mod args;
pub mod bench_key;
mod commands;
mod cycle;
mod hold;
pub mod identity;
mod link;
mod offers;
mod pins;
mod report;
pub mod usb_boot;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use atlas_session::{Pause, RealPause};
use orion::Address;
use tokio::sync::Notify;

pub use pins::PINS_FILE;

/// How long a command waits for a node to answer before it says so.
pub const WAIT: Duration = Duration::from_secs(15);

/// The usage text.
pub const USAGE: &str = "usage: atlas-next slots <address> <command> [options]
       atlas-next bench-key <directory> [--json]

  <address>   an Orion address as Orion reads it: orion+tcp://host[:port],
              orion+tcp://[::1]:9200, orion+tcp://[fe80::1%eth0]:9200, or a socket path

  fingerprint                 show the board's key fingerprint (no pin needed)
  pin                         trust the board's key for this address
  repin --reflashed           replace the pin after the board was reflashed
  list                        the board's slots
  install <bundle file>       send a slot bundle and install it; prints the stages
  trial <slot id>             start a trial and follow it to its outcome (Ctrl-C stops following)
  confirm <slot id>           make the slot the default
  remove <slot id>            remove a slot (never a starred or the default one)

  enrol <address>             ask the board to know this PC's identity (no proof: the owner approves it)
  identity                    this PC's client identity: made once, prints its fingerprint and file

  cycle <address> --good <bundle> --bad <bundle> --scorecard <file> [--json] [--dry-run]
                              the slot cycle in one run: install, trial, a forced bad trial that falls
                              back by itself, confirm; one scorecard file says each step in plain words
                              (--json also writes <file>.json; --dry-run lists the steps and sends nothing)
  usb-boot (--agent <host:port> | --serve-dir <new dir>) --port <usb path> --bundle <file>
           --boot-partition <file> --pubkey <file> [--bootfiles <dir>] [--timeout <s>] [--dry-run]
           [--bay-exact] [--control padded --base <file>]
                              USB boot a board through the SystemCore agent with a built image's
                              boot files (never writes the eMMC; refuses before sending on any check).
                              --bay-exact serves the partition image byte for byte as boot.img
                              (boot_ramdisk=1); --control padded --base <A> serves option R's
                              control C: A's partition with only its bay padded back (with
                              --bay-exact; any other difference is refused);
                              --serve-dir writes the checked BAY-EXACT set and the second stage
                              (2712/) for `rpiboot -v -d <dir> -p <usb path>` on this PC

  bench-key <directory>       make the throwaway bench signing key (bench boards only, never a
                              released image): bench.seed, bench.pub, bench.keys in a new directory

  hold <address> <seconds>    hold: bench check only. Takes the session's hold on a pinned board through the same hold the app's terminal and jobs use, sends nothing else, and prints each lease renewal and any lapse. Exit 0 when the lease was kept, 1 on a lapse or no renewal.

  offers <address> --out <file.json>
                              offers: bench only. Records what the board's node offers (facts, resources and each device's describe, topics) into one JSON file, for the sim's fixture. Reads only; exit 0 when the file was written, 1 otherwise.

options:
  --state <dir>      where pins and the trial's saved watches live
                     (default: $XDG_DATA_HOME/atlas-next, else ~/.local/share/atlas-next)
  --identity <file>  the PC's identity file (default: <config>/atlas/identity.key, mode 0600)
  --json             one JSON document on stdout instead of text

exit status: 0 done; 1 the board refused or the outcome is a failure; 2 usage;
             3 the pin rule refused the board; 4 the outcome is unknown after a reconnect (what was
             checked is said; running it again is your choice)";

/// What a run needs from its surroundings. The binary builds it from the real clock and the
/// real waits; a test builds it with a yielding pause and short polls.
#[derive(Debug)]
pub struct Env {
    /// The state directory when `--state` is not given. `None` means there is none: the command
    /// then refuses, unless `--state` is given.
    pub state_dir: Option<PathBuf>,
    /// The identity file when `--identity` is not given. `None` means there is none: the command
    /// then refuses, unless `--identity` is given. The binary sets the user's default; a test sets
    /// a file under its own scratch directory, never the real config directory.
    pub identity_default: Option<PathBuf>,
    /// The wait between a connection's tries.
    pub pause: Arc<dyn Pause>,
    /// How long a command waits for a node to answer.
    pub wait: Duration,
    /// How often a settling trial's records are read again while the board stays connected.
    pub recheck: Duration,
    /// Set by the binary when Ctrl-C should stop a trial's following. `None` means it never does.
    pub stop: Option<Arc<Notify>>,
    /// Whether the Lemnos this build pins publishes the boot id (`info.last_attempt.boot`). The binary
    /// takes `cycle::INFO_IN_PIN`; a test sets it false for a board without the field.
    pub info_in_pin: bool,
}

impl Env {
    /// The real environment: real waits, the default poll, no Ctrl-C handler (the binary sets one
    /// for a trial only).
    #[must_use]
    pub fn new(state_dir: Option<PathBuf>) -> Self {
        Self {
            state_dir,
            identity_default: None,
            pause: Arc::new(RealPause),
            wait: WAIT,
            recheck: atlas_board_slots::trial::RECHECK,
            stop: None,
            info_in_pin: cycle::INFO_IN_PIN,
        }
    }
}

/// Runs one command line (the arguments after the program name). Returns the exit status.
pub async fn execute<W: Write>(args: &[String], env: &Env, out: &mut W) -> i32 {
    match args::parse(args) {
        args::Parsed::Help => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        args::Parsed::Usage(why) => {
            let _ = writeln!(out, "atlas-next: {why}\n\n{USAGE}");
            2
        }
        args::Parsed::Run(command) => commands::run(command, env, out).await,
        args::Parsed::BenchKey { directory, json } => bench_key::run(&directory, json, out),
        args::Parsed::Cycle(cycle_args) => cycle::run(&cycle_args, env, out).await,
        args::Parsed::UsbBoot(options) => usb_boot::run(&options, env, out).await,
        args::Parsed::Identity {
            identity: file,
            json,
        } => match file.or_else(|| env.identity_default.clone()) {
            Some(path) => identity::run(&path, json, out),
            None => {
                let _ = writeln!(out, "atlas-next: no identity file: give --identity <file>");
                2
            }
        },
    }
}

/// Whether the command line is a trial or a `usb-boot`: the commands Ctrl-C stops (the binary
/// only listens for Ctrl-C then, so every other command keeps the default Ctrl-C).
#[must_use]
pub fn is_trial(args: &[String]) -> bool {
    matches!(
        args::parse(args),
        args::Parsed::UsbBoot(_)
            | args::Parsed::Run(args::Command {
                verb: args::Verb::Trial(_),
                ..
            })
    )
}

/// An Orion address as Orion reads it. Nothing is added to the text: a bare `127.0.0.1:9200` is
/// refused by Orion, and the refusal is returned as Orion words it.
///
/// # Errors
///
/// Orion's own words when it does not read the address.
pub fn parse_address(text: &str) -> Result<Address, String> {
    text.parse::<Address>()
}

/// Where the state of the tool goes when `--state` is not given: `$XDG_DATA_HOME/atlas-next`,
/// else `~/.local/share/atlas-next`. `None` when neither variable names a directory.
#[must_use]
pub fn default_state_dir(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg.filter(|value| !value.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.filter(|value| !value.is_empty())?).join(".local/share"),
    };
    Some(base.join("atlas-next"))
}
