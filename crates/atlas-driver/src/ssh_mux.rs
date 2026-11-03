//! One SSH connection per device, shared by every command Atlas runs on it.
//!
//! Atlas runs the system's OpenSSH client once per command, and an update
//! polls every few seconds. Each of those used to be a new connection; a
//! controller's sshd counts a burst of them against this computer and
//! refuses it for minutes (OpenSSH's PerSourcePenalties). With these
//! options the first command opens a master connection that stays up for
//! a while after the last one, and the rest ride on it.
//!
//! The socket is named by a hash of the device (its known_hosts alias and
//! address), never by the address alone: boards share their USB gadget
//! address, and one board's connection mustn't carry another's commands.
//! Windows' OpenSSH has no connection sharing, so there it's one connection
//! per command, as before.

use std::path::PathBuf;

/// How long a master connection stays up after its last command.
const PERSIST_SECS: u32 = 120;

/// The `-o` options that share one connection to `device`; none where the
/// client can't share, or no private directory can be made for the socket.
pub fn options(device: &str) -> Vec<String> {
    if !cfg!(unix) {
        return Vec::new();
    }
    let Some(dir) = socket_dir() else {
        return Vec::new();
    };
    let path = dir.join(format!("{:016x}", fnv1a(device.as_bytes())));
    vec![
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", path.display()),
        "-o".into(),
        format!("ControlPersist={PERSIST_SECS}"),
    ]
}

/// A directory only this user can reach, short enough for a socket path
/// (about 100 bytes on Linux and macOS).
fn socket_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("atlas-ssh-{}", user_tag()));
    make_private(&dir).then_some(dir)
}

/// The user Atlas runs as (its home's owner), so users sharing /tmp don't
/// share a directory.
#[cfg(unix)]
fn user_tag() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::env::var_os("HOME")
        .and_then(|home| std::fs::metadata(home).ok())
        .map(|meta| meta.uid())
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn user_tag() -> u32 {
    0
}

#[cfg(unix)]
fn make_private(dir: &std::path::Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let _ = std::fs::DirBuilder::new().mode(0o700).create(dir);
    // Someone else's directory, or one others can reach: no sharing.
    match std::fs::symlink_metadata(dir) {
        Ok(meta) => {
            meta.is_dir()
                && meta.permissions().mode() & 0o077 == 0
                && std::env::var_os("HOME")
                    .and_then(|home| std::fs::metadata(home).ok())
                    .is_none_or(|home| home.uid() == meta.uid())
        }
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn make_private(_dir: &std::path::Path) -> bool {
    false
}

/// FNV-1a: stable across runs and builds, so a later Atlas finds the
/// connection an earlier one left.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_device_gets_its_own_socket() {
        let a = options("atlas-raze-a317 172.31.209.217");
        let b = options("atlas-raze-0708 172.31.209.217");
        if cfg!(unix) && !a.is_empty() {
            assert_eq!(a[1], "ControlMaster=auto");
            assert!(a[3].starts_with("ControlPath="));
            assert_ne!(a[3], b[3], "two boards on one address share nothing");
            assert_eq!(a, options("atlas-raze-a317 172.31.209.217"));
            assert!(a[3].len() < 100, "{} is too long for a socket", a[3]);
        }
    }
}
