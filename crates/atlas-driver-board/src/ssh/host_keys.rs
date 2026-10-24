//! The SSH host keys Atlas pins for boards (its own known_hosts, one entry
//! per board under `HostKeyAlias=board-<device key>`), and when to let go of
//! one.
//!
//! A board's host key changes when its OS does: a fresh install makes new
//! keys, and so does the first boot of a slot whose OS kept its own keys
//! before the per-board key store. Atlas then refused every SSH with
//! "REMOTE HOST IDENTIFICATION HAS CHANGED" and nothing in the app could
//! clear it. Now:
//! - after Atlas itself changes what a board runs (a slot install that boots
//!   it, boot once, set default, roll back), a changed key is expected for a
//!   while ([`expect_new_key`]): the first refusal forgets the old pin and the
//!   command runs again, pinning the new key;
//! - "Trust its new SSH key" (an action) and forgetting the device drop it;
//! - entries are written unhashed (`HashKnownHosts=no`; Debian's default
//!   hashes them, so nothing could find a board's entry to drop it), and
//!   hashed ones left in Atlas's own file from before go once.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex, Once};
use std::time::{Duration, Instant};

/// How long after Atlas changes a board's OS a new host key is expected.
const EXPECT_FOR: Duration = Duration::from_secs(30 * 60);

static EXPECTED: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The `HostKeyAlias` for a board's device key.
pub(crate) fn alias(key: &str) -> String {
    format!("board-{key}").replace(':', "-")
}

/// A new host key from the board with this alias is expected (Atlas just
/// changed its OS).
pub(crate) fn expect_new_key(alias: &str) {
    if let Ok(mut map) = EXPECTED.lock() {
        map.insert(alias.to_owned(), Instant::now());
    }
}

/// Whether a changed key from this board is one Atlas expects.
pub(crate) fn new_key_expected(alias: &str) -> bool {
    EXPECTED
        .lock()
        .ok()
        .and_then(|map| map.get(alias).copied())
        .is_some_and(|at| at.elapsed() < EXPECT_FOR)
}

/// Drops the pinned key for `alias` from `known_hosts`, hashed or not
/// (`ssh-keygen -R`). True when the file has no entry for it any more.
pub fn forget(known_hosts: &Path, alias: &str) -> bool {
    if !known_hosts.exists() {
        return true;
    }
    let ok = std::process::Command::new("ssh-keygen")
        .arg("-R")
        .arg(alias)
        .arg("-f")
        .arg(known_hosts)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    // ssh-keygen keeps the previous file as <file>.old.
    let mut old = known_hosts.as_os_str().to_owned();
    old.push(".old");
    let _ = std::fs::remove_file(old);
    ok
}

/// Forgets the board with device key `key` (its display form,
/// `family:serial`).
pub fn forget_board(known_hosts: &Path, key: &str) -> bool {
    forget(known_hosts, &alias(key))
}

/// Once a run: hashed entries in Atlas's own known_hosts (written before
/// `HashKnownHosts=no`) go, since nothing can tell which board they are;
/// each board is pinned again at its next connection.
pub(crate) fn drop_hashed_once(known_hosts: &Path) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = drop_hashed(known_hosts);
    });
}

fn drop_hashed(known_hosts: &Path) -> std::io::Result<usize> {
    let text = match std::fs::read_to_string(known_hosts) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !line.starts_with("|1|"))
        .collect();
    let gone = text.lines().count() - kept.len();
    if gone > 0 {
        let tmp = known_hosts.with_extension("tmp");
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, known_hosts)?;
    }
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_expectations() {
        assert_eq!(
            alias("raze:0708c973b00a37bc"),
            "board-raze-0708c973b00a37bc"
        );
        let a = alias("raze:test-expect");
        assert!(!new_key_expected(&a));
        expect_new_key(&a);
        assert!(new_key_expected(&a));
    }

    #[test]
    fn hashed_entries_go_plain_ones_stay() {
        let dir = std::env::temp_dir().join(format!("atlas-hostkeys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("known_hosts");
        std::fs::write(
            &file,
            "|1|abc=|def= ssh-ed25519 AAAA1\nboard-raze-1234 ssh-ed25519 AAAA2\n",
        )
        .unwrap();
        assert_eq!(drop_hashed(&file).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "board-raze-1234 ssh-ed25519 AAAA2\n"
        );
        // ssh-keygen -R drops a plain entry by its alias (when ssh-keygen is here).
        if std::process::Command::new("ssh-keygen")
            .arg("-?")
            .output()
            .is_ok()
        {
            assert!(forget(&file, "board-raze-1234"));
            assert!(
                !std::fs::read_to_string(&file)
                    .unwrap()
                    .contains("board-raze-1234")
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
