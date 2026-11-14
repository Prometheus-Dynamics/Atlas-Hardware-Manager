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
pub mod identity;
mod link;
mod pins;
mod report;

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

  bench-key <directory>       make the throwaway bench signing key (bench boards only, never a
                              released image): bench.seed, bench.pub, bench.keys in a new directory

options:
  --state <dir>      where pins and the trial's saved watches live
                     (default: $XDG_DATA_HOME/atlas-next, else ~/.local/share/atlas-next)
  --identity <file>  the PC's identity file (default: <config>/atlas/identity.key, mode 0600)
  --json             one JSON document on stdout instead of text

exit status: 0 done; 1 the board refused or the outcome is a failure; 2 usage;
             3 the pin rule refused the board";

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

/// Whether the command line is a trial, which is the one command Ctrl-C stops (the binary only
/// listens for Ctrl-C then, so every other command keeps the default Ctrl-C).
#[must_use]
pub fn is_trial(args: &[String]) -> bool {
    matches!(
        args::parse(args),
        args::Parsed::Run(args::Command {
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
