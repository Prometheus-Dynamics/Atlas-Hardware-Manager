//! Which files the server offers, under which tokens, and for how long.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use atlas_driver::DriverError;

/// One offered file. The token is the only way to reach it.
pub(crate) struct Entry {
    pub path: PathBuf,
    /// The file name the URL must end in.
    pub name: String,
    pub size: u64,
    /// Lowercase hex SHA-256, served as the strong ETag.
    pub sha256: String,
    last_used: Instant,
    active: Arc<AtomicUsize>,
}

/// What [`Registry::lookup`] hands the request handler.
pub(crate) struct Found {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub active: Arc<AtomicUsize>,
}

#[derive(Default)]
pub(crate) struct Registry {
    entries: HashMap<String, Entry>,
}

/// A registration: the token and the file name that make up its URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    /// 64 lowercase hex characters (32 random bytes).
    pub token: String,
    pub name: String,
}

impl Registered {
    /// `/images/<token>/<name>`, with the name percent-encoded.
    pub fn path(&self) -> String {
        format!(
            "/images/{}/{}",
            self.token,
            crate::http::encode_name(&self.name)
        )
    }

    /// The path with the token cut short, for logs.
    pub(crate) fn logged_path(&self) -> String {
        format!("/images/{}/{}", short(&self.token), self.name)
    }
}

/// The first 8 characters of a token, for logs.
pub(crate) fn short(token: &str) -> String {
    format!("{}…", token.get(..8).unwrap_or(token))
}

fn new_token() -> Result<String, DriverError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        DriverError::Other(format!("no randomness for an image token: {error}"))
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn is_token(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_sha256(text: &str) -> bool {
    is_token(text)
}

impl Registry {
    /// Offers `path` under a new token. The file must exist, be a regular
    /// file, and have `size` bytes; `sha256` becomes its ETag.
    pub fn register(
        &mut self,
        path: &Path,
        sha256: &str,
        size: u64,
    ) -> Result<Registered, DriverError> {
        let sha256 = sha256.trim().to_ascii_lowercase();
        if !is_sha256(&sha256) {
            return Err(DriverError::Other(format!(
                "{} has no valid SHA-256 to offer it with",
                path.display()
            )));
        }
        let path = path
            .canonicalize()
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        let meta = std::fs::metadata(&path)
            .map_err(|error| DriverError::Other(format!("{}: {error}", path.display())))?;
        if !meta.is_file() {
            return Err(DriverError::Other(format!(
                "{} is not a file",
                path.display()
            )));
        }
        if meta.len() != size {
            return Err(DriverError::Other(format!(
                "{} has {} bytes, not the {size} its release lists",
                path.display(),
                meta.len()
            )));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                DriverError::Other(format!("{} has no usable file name", path.display()))
            })?
            .to_string();
        let token = new_token()?;
        self.entries.insert(
            token.clone(),
            Entry {
                path,
                name: name.clone(),
                size,
                sha256,
                last_used: Instant::now(),
                active: Arc::new(AtomicUsize::new(0)),
            },
        );
        Ok(Registered { token, name })
    }

    pub fn revoke(&mut self, token: &str) -> bool {
        self.entries.remove(token).is_some()
    }

    /// Drops registrations unused for longer than `expiry`, unless a
    /// download is still running.
    pub fn prune(&mut self, expiry: Duration) {
        let now = Instant::now();
        self.entries.retain(|_, entry| {
            entry.active.load(Ordering::Relaxed) > 0
                || now.duration_since(entry.last_used) <= expiry
        });
    }

    /// The live registration for `token`, marking it used.
    pub fn lookup(&mut self, token: &str, expiry: Duration) -> Option<Found> {
        self.prune(expiry);
        let entry = self.entries.get_mut(token)?;
        entry.last_used = Instant::now();
        Some(Found {
            path: entry.path.clone(),
            name: entry.name.clone(),
            size: entry.size,
            sha256: entry.sha256.clone(),
            active: entry.active.clone(),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Counts one running download against its token's limit until dropped.
pub(crate) struct DownloadSlot {
    active: Arc<AtomicUsize>,
}

impl DownloadSlot {
    /// None when `limit` downloads of this token already run.
    pub fn acquire(active: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |running| {
                (running < limit).then_some(running + 1)
            })
            .ok()
            .map(|_| Self {
                active: active.clone(),
            })
    }
}

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_random_and_distinct() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert!(is_token(&a) && is_token(&b));
        assert_ne!(a, b);
        assert!(!is_token("../etc"));
        assert!(!is_token(&a.to_ascii_uppercase()));
        assert_eq!(short(&a).chars().count(), 9);
    }

    #[test]
    fn slots_cap_concurrent_downloads() {
        let active = Arc::new(AtomicUsize::new(0));
        let first = DownloadSlot::acquire(&active, 2).unwrap();
        let _second = DownloadSlot::acquire(&active, 2).unwrap();
        assert!(DownloadSlot::acquire(&active, 2).is_none());
        drop(first);
        assert!(DownloadSlot::acquire(&active, 2).is_some());
    }
}
