//! Unix socket paths. A socket's address is `sun_path`: 108 bytes with the NUL on Linux, so at most
//! [`SUN_PATH_MAX`] bytes of path. Every socket Atlas connects to, and every directory a simulated
//! node or a test rig makes for its socket, passes the check here before anything binds: a path
//! that is too long is a typed [`SocketError`] that names the path, never a panic inside a thread.
//!
//! A node's directory is made by [`node_dir`]: `<base>/atlas-t-<hash of the name>-<pid>-<n>`, with the
//! readable name in a `name` file. The base is `$XDG_RUNTIME_DIR` when it is set and leaves room, else
//! the temp dir (`TMPDIR`) when it leaves room, else a typed error. Sockets live under the runtime dir
//! and not under `TMPDIR` because the runtime dir is short by design and `TMPDIR` is whatever the caller
//! gave (on CI, the build path, which is too long for a socket). The directory is a [`NodeDir`] guard:
//! dropped, it removes the directory, panics included. Only directories with the `atlas-t-` prefix are ever
//! removed.

use std::fmt;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The longest path a Unix socket takes: `sun_path` (108 bytes) less the NUL.
pub const SUN_PATH_MAX: usize = 107;

/// The file a node's socket is in, inside its directory. Orion's test stub binds this name.
pub const NODE_SOCKET: &str = "orion.sock";

/// The file a node directory keeps its readable name in. Its presence marks a directory this module made.
pub const NAME_FILE: &str = "name";

/// The prefix of every node directory's name, so nothing of ours is mistaken for anything else in a shared
/// runtime dir.
pub const PREFIX: &str = "atlas-t-";

/// Digits of the name's hash in a node directory's name.
const HASH_DIGITS: usize = 16;
/// Digits of a process id, the widest a `u32` prints.
const PID_DIGITS: usize = 10;
/// Digits of the counter: at most [`MAX_TRIES`] names are tried in one call.
const COUNTER_DIGITS: usize = 4;
/// How many names [`node_dir_in`] tries before it gives up on a base that will not take one.
const MAX_TRIES: u32 = 1000;

/// The most bytes [`node_dir_in`] adds to its base on the way to a node's socket path: `/`, the prefix,
/// the hash, `-`, the pid, `-`, the counter, `/` and `orion.sock`. The gate's room check uses this figure
/// (`SOCKET_SUFFIX_BYTES` in scripts/gate.sh); a unit test in this module keeps them equal.
pub const NODE_SOCKET_SUFFIX_MAX: usize =
    1 + PREFIX.len() + HASH_DIGITS + 1 + PID_DIGITS + 1 + COUNTER_DIGITS + 1 + NODE_SOCKET.len();

static NEXT: AtomicU32 = AtomicU32::new(0);

/// Why a socket path or a node directory cannot be made.
#[derive(Debug)]
pub enum SocketError {
    /// The socket path is longer than a Unix socket takes. Nothing was bound.
    TooLong {
        /// The path that would be bound or connected to.
        path: PathBuf,
        /// Its length in bytes.
        bytes: usize,
    },
    /// The directory could not be made or written.
    Io {
        /// The directory or file that failed.
        path: PathBuf,
        /// What the system said.
        error: io::Error,
    },
}

impl SocketError {
    /// The path the error is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::TooLong { path, .. } | Self::Io { path, .. } => path,
        }
    }
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { path, bytes } => write!(
                f,
                "the socket path {} is {bytes} bytes; a Unix socket takes at most {SUN_PATH_MAX}. \
                 Shorten the directory it is in (a node's socket goes under $XDG_RUNTIME_DIR, else the temp dir)",
                path.display()
            ),
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for SocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TooLong { .. } => None,
            Self::Io { error, .. } => Some(error),
        }
    }
}

/// Passes `path` when a Unix socket can take it, else the typed error naming it. Call this before
/// anything binds or connects to `path`.
///
/// # Errors
///
/// [`SocketError::TooLong`] when the path is longer than [`SUN_PATH_MAX`] bytes.
pub fn check_socket_path(path: &Path) -> Result<&Path, SocketError> {
    let bytes = path.as_os_str().len();
    if bytes > SUN_PATH_MAX {
        Err(SocketError::TooLong {
            path: path.to_path_buf(),
            bytes,
        })
    } else {
        Ok(path)
    }
}

/// A node's directory, made by [`node_dir`] or [`node_dir_in`]. It derefs to the path. Dropped, it removes
/// the directory (and the socket and state in it), so a test that ends, or panics, leaves nothing behind.
/// Keep the guard alive for as long as the node runs in the directory.
#[derive(Debug)]
pub struct NodeDir {
    path: PathBuf,
}

impl Deref for NodeDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for NodeDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for NodeDir {
    fn drop(&mut self) {
        // Only what this module made: the prefix. Tests may clear a node's directory and start again in
        // it, which removes the name file too, so the prefix is the mark. Anything else is left alone.
        let ours = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(PREFIX));
        if ours {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// The candidate bases, in order: `$XDG_RUNTIME_DIR` when it is an absolute path, then the temp dir.
fn bases(xdg: Option<PathBuf>, temp: PathBuf) -> Vec<PathBuf> {
    xdg.filter(|dir| dir.is_absolute())
        .into_iter()
        .chain(std::iter::once(temp))
        .collect()
}

/// A new, empty directory for a node's socket, named for `name`. The base is `$XDG_RUNTIME_DIR` when it
/// is set and leaves room for a socket, else the temp dir when it leaves room, else the typed error of
/// the last try. See [`node_dir_in`].
///
/// # Errors
///
/// [`SocketError::TooLong`] when no base leaves room for a socket, [`SocketError::Io`] otherwise.
pub fn node_dir(name: &str) -> Result<NodeDir, SocketError> {
    node_dir_from(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::temp_dir(),
        name,
    )
}

/// As [`node_dir`], with the runtime dir and the temp dir given (the environment is read by [`node_dir`]
/// alone, so a test can cover both fallbacks without changing the process environment).
///
/// # Errors
///
/// As [`node_dir`].
pub fn node_dir_from(
    xdg: Option<PathBuf>,
    temp: PathBuf,
    name: &str,
) -> Result<NodeDir, SocketError> {
    let mut last = None;
    for base in bases(xdg, temp) {
        match node_dir_in(&base, name) {
            Err(error @ SocketError::TooLong { .. }) => last = Some(error),
            other => return other,
        }
    }
    Err(last.unwrap_or(SocketError::TooLong {
        path: PathBuf::new(),
        bytes: 0,
    }))
}

/// A new, empty directory `<base>/atlas-t-<hash of name>-<pid>-<n>` for a node's socket, with `name` in its
/// `name` file. The directory name has the same length whatever `name` is, so a long test or rig name
/// cannot push the socket path past [`SUN_PATH_MAX`]; the readable name is kept in the file instead. The
/// socket path (`<dir>/orion.sock`) is checked before the directory is made.
///
/// # Errors
///
/// [`SocketError::TooLong`] when `base` is too long for a socket under it, and [`SocketError::Io`] when
/// the directory cannot be made or written.
pub fn node_dir_in(base: &Path, name: &str) -> Result<NodeDir, SocketError> {
    let hash = fnv1a(name.as_bytes());
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..MAX_TRIES {
        let counter = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("{PREFIX}{hash:016x}-{pid}-{counter}"));
        check_socket_path(&dir.join(NODE_SOCKET))?;
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                let guard = NodeDir { path: dir };
                let file = guard.path.join(NAME_FILE);
                std::fs::write(&file, name)
                    .map_err(|error| SocketError::Io { path: file, error })?;
                return Ok(guard);
            }
            // Another directory of this process or an old run has the name: try the next counter.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = Some(dir),
            Err(error) => return Err(SocketError::Io { path: dir, error }),
        }
    }
    let path = last.unwrap_or_else(|| base.to_path_buf());
    Err(SocketError::Io {
        path,
        error: io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("no free name after {MAX_TRIES} tries"),
        ),
    })
}

/// FNV-1a, 64 bits: fixed across builds and platforms, which the directory name needs only for
/// readability and uniqueness, not security.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The base the tests make node dirs under: the runtime dir when set (short), else the temp dir.
    /// The tests make no directories of their own under it: only node dirs, which their guards remove.
    fn base() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(std::env::temp_dir)
    }

    #[test]
    fn a_long_name_gives_a_short_socket_path_and_keeps_the_name() {
        let name = "x".repeat(200);
        let dir = node_dir_in(&base(), &name).expect("a directory for any name");
        let socket = dir.join(NODE_SOCKET);
        assert!(check_socket_path(&socket).is_ok(), "{socket:?}");
        assert!(
            dir.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX))
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(NAME_FILE)).expect("the name file"),
            name
        );
    }

    #[test]
    fn two_names_in_one_process_give_two_directories() {
        let a = node_dir_in(&base(), "same").expect("first");
        let b = node_dir_in(&base(), "same").expect("second");
        assert_ne!(a.to_path_buf(), b.to_path_buf());
    }

    #[test]
    fn dropping_the_guard_removes_the_directory_and_nothing_else() {
        let dir = node_dir_in(&base(), "gone").expect("a directory");
        let path = dir.to_path_buf();
        std::fs::write(path.join(NODE_SOCKET), b"x").expect("a file in it");
        drop(dir);
        assert!(!path.exists(), "the directory is removed on drop");
        // A directory without our marker (no prefix, no name file) is never removed by a guard.
        let foreign = base().join(format!("not-ours-{}", std::process::id()));
        std::fs::create_dir_all(&foreign).expect("a foreign dir");
        drop(NodeDir {
            path: foreign.clone(),
        });
        assert!(foreign.exists(), "a foreign directory is left alone");
        std::fs::remove_dir(&foreign).expect("the test removes its own foreign dir");
    }

    #[test]
    fn a_panic_still_removes_the_directory() {
        let base = base();
        let result = std::panic::catch_unwind(|| {
            let dir = node_dir_in(&base, "panics").expect("a directory");
            let path = dir.to_path_buf();
            std::panic::resume_unwind(Box::new(path));
        });
        let path = result
            .expect_err("the closure panics")
            .downcast::<PathBuf>()
            .expect("the path");
        assert!(!path.exists(), "a panicking rig leaves nothing behind");
    }

    #[test]
    fn a_base_that_is_too_long_is_a_typed_error_naming_the_path() {
        let long = std::env::temp_dir().join("d".repeat(120));
        let error = node_dir_in(&long, "any").expect_err("a too-long base");
        let SocketError::TooLong { path, bytes } = &error else {
            panic!("expected TooLong, got {error:?}");
        };
        assert!(path.starts_with(&long), "{path:?}");
        assert!(*bytes > SUN_PATH_MAX);
        let text = error.to_string();
        assert!(text.contains(&path.display().to_string()), "{text}");
        assert!(text.contains("107"), "{text}");
        assert!(!long.exists(), "nothing was made");
    }

    #[test]
    fn runtime_dir_is_used_when_set_and_fits() {
        let temp = std::env::temp_dir();
        let dir = node_dir_from(Some(base()), temp, "rt").expect("a directory");
        assert!(dir.starts_with(base()), "{:?}", dir.to_path_buf());
    }

    #[test]
    fn falls_back_to_the_temp_dir_when_runtime_is_unset_or_too_long() {
        let temp = std::env::temp_dir();
        let unset = node_dir_from(None, temp.clone(), "unset").expect("a directory");
        assert!(unset.starts_with(&temp));
        let too_long = PathBuf::from("/").join("r".repeat(120));
        let long = node_dir_from(Some(too_long), temp.clone(), "long").expect("a directory");
        assert!(long.starts_with(&temp));
    }

    #[test]
    fn no_base_that_fits_is_a_typed_error_naming_the_last_try() {
        let temp = std::env::temp_dir().join("t".repeat(120));
        let error = node_dir_from(None, temp.clone(), "nowhere").expect_err("no base fits");
        assert!(matches!(error, SocketError::TooLong { .. }), "{error:?}");
        assert!(error.to_string().contains(&temp.display().to_string()));
    }

    #[test]
    fn check_refuses_a_path_one_byte_too_long() {
        let ok = PathBuf::from("/".to_owned() + &"a".repeat(SUN_PATH_MAX - 1));
        assert!(check_socket_path(&ok).is_ok());
        let long = PathBuf::from("/".to_owned() + &"a".repeat(SUN_PATH_MAX));
        assert!(matches!(
            check_socket_path(&long),
            Err(SocketError::TooLong { .. })
        ));
    }

    #[test]
    fn the_suffix_constant_is_the_gate_figure() {
        // scripts/gate.sh: SOCKET_SUFFIX_BYTES. Change both together.
        assert_eq!(NODE_SOCKET_SUFFIX_MAX, 52);
        let base = base();
        let dir = node_dir_in(&base, "suffix").expect("a directory");
        let socket = dir.join(NODE_SOCKET);
        assert!(socket.as_os_str().len() <= base.as_os_str().len() + NODE_SOCKET_SUFFIX_MAX);
    }
}
