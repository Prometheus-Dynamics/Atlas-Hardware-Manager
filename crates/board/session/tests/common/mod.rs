//! Scratch directories for the session tests, one place for every file that starts a node.
//!
//! A node's socket is `<dir>/orion.sock`, and Orion's stub panics when that path is not shorter
//! than `SUN_LEN` (108 bytes with the NUL, so at most 107 here). So a scratch dir is under the
//! system temp dir with a short name, never under `CARGO_TARGET_TMPDIR`: on CI that is a long
//! build path, and the socket would not fit.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The longest socket path a Unix socket takes: `SUN_LEN` less the NUL.
pub const SUN_PATH_MAX: usize = 107;

/// Every name the session test files give [`scratch`]. The test `every_socket_path_fits_sun_len`
/// checks each one; [`scratch`] checks the name it is given at run time as well.
pub const NAMES: &[&str] = &[
    // session.rs
    "live",
    "failed",
    "denied-kind",
    "not-denied-kind",
    "gone",
    "again",
    "timeout",
    "stale",
    // ways_in.rs
    "bound",
    // long.rs
    "short",
    "stream",
    "cancel",
    "lost",
    "replace-a",
    "replace-b",
    "limit",
    // lease.rs
    "no-hold",
    "renewed",
    "traffic",
    "two",
    "replaced-a",
    "replaced-b",
    "plain",
    "long",
    "lapse",
    "per-board-a",
    "per-board-b",
    "off",
    // pin.rs
    "same",
    "queued-a",
    "queued-b",
    "accept-a",
    "accept-b",
    "known-a",
    "known-b",
    "back-a",
    "back-b",
];

/// A scratch directory for one test: `atl-s<pid>-<name>` under the system temp dir.
///
/// # Panics
///
/// When the node's socket path in it would not fit `SUN_LEN` (a long `TMPDIR`, or a long name).
#[must_use]
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atl-s{}-{name}", std::process::id()));
    let socket = socket_in(&dir);
    assert!(
        socket.as_os_str().len() <= SUN_PATH_MAX,
        "{name}: the node's socket {socket:?} is {} bytes; SUN_LEN allows {SUN_PATH_MAX}. \
         Shorten TMPDIR or the name.",
        socket.as_os_str().len()
    );
    dir
}

/// The path of a node's socket in `dir`, as Orion's stub makes it.
#[must_use]
pub fn socket_in(dir: &Path) -> PathBuf {
    dir.join("orion.sock")
}

/// The longest socket path [`scratch`] can give for `longest` (the longest name), in the temp dir
/// as it is now, with the widest process id a `u32` prints.
#[must_use]
pub fn worst_socket_len(longest: usize) -> usize {
    let widest_pid = u32::MAX.to_string().len();
    let temp = std::env::temp_dir();
    temp.as_os_str().len() + "/atl-s".len() + widest_pid + 1 + longest + "/orion.sock".len()
}
