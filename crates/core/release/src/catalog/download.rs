//! Downloading a remote release into the cache. The bytes go to a `.partial`
//! file that is resumed after a drop or a restart. The size and SHA-256 are
//! checked as they arrive, and the file takes its final name only when it is
//! whole and right. A file that fails its check is deleted.

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

use super::*;

/// A download with nothing arriving for this long has stalled.
const STALL: Duration = Duration::from_secs(60);
/// Retries wait this long at first, then double up to the cap. A download is
/// never given up on (the failure rule), so the wait stays short.
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(5);
const HASH_READ: usize = 1024 * 1024;

/// One request's outcome.
enum Attempt {
    /// The file is whole: nothing more to fetch.
    Done,
    /// The request failed or the connection dropped: try again.
    Retry,
}

/// The `.partial` file for `target`, with the bytes it holds so far.
struct Partial {
    path: PathBuf,
    file: tokio::fs::File,
    hasher: Sha256,
    offset: u64,
}

impl Partial {
    /// Opens (or creates) the partial file, hashing what is already in it.
    async fn open(path: PathBuf, size: u64) -> Result<Self, ReleaseError> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .await
            .map_err(|error| ReleaseError::io(&path, error))?;
        let mut partial = Self {
            path,
            file,
            hasher: Sha256::new(),
            offset: 0,
        };
        let mut remaining = partial
            .file
            .metadata()
            .await
            .map_err(|error| ReleaseError::io(&partial.path, error))?
            .len();
        if remaining > size {
            partial
                .file
                .set_len(0)
                .await
                .map_err(|error| ReleaseError::io(&partial.path, error))?;
            remaining = 0;
        }
        let mut buffer = vec![0u8; HASH_READ];
        while remaining > 0 {
            let want = usize::try_from(remaining.min(HASH_READ as u64)).unwrap_or(HASH_READ);
            let read = partial
                .file
                .read(&mut buffer[..want])
                .await
                .map_err(|error| ReleaseError::io(&partial.path, error))?;
            if read == 0 {
                break;
            }
            partial.hasher.update(&buffer[..read]);
            partial.offset += read as u64;
            remaining -= read as u64;
        }
        partial
            .file
            .seek(SeekFrom::Start(partial.offset))
            .await
            .map_err(|error| ReleaseError::io(&partial.path, error))?;
        Ok(partial)
    }

    /// Throws away what is there and starts from byte zero.
    async fn restart(&mut self) -> Result<(), ReleaseError> {
        self.file
            .set_len(0)
            .await
            .map_err(|error| ReleaseError::io(&self.path, error))?;
        self.file
            .seek(SeekFrom::Start(0))
            .await
            .map_err(|error| ReleaseError::io(&self.path, error))?;
        self.hasher = Sha256::new();
        self.offset = 0;
        Ok(())
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), ReleaseError> {
        self.file
            .write_all(bytes)
            .await
            .map_err(|error| ReleaseError::io(&self.path, error))?;
        self.hasher.update(bytes);
        self.offset += bytes.len() as u64;
        Ok(())
    }
}

impl ReleaseCatalog {
    /// Downloads a remote release into the cache. Under [`Security::On`] a
    /// release without a good trusted signature is refused before any
    /// bytes are fetched.
    pub async fn download(
        &self,
        id: &str,
        progress: impl Fn(DownloadProgress) + Send + Sync,
    ) -> Result<ReleaseEntry, ReleaseError> {
        let mut entry = self
            .entry(id)
            .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
        let ReleaseOrigin::Remote { url, .. } = entry.origin.clone() else {
            return Ok(entry);
        };
        if entry.path.as_ref().is_some_and(|path| path.exists()) {
            return Ok(entry);
        }
        if self.security() == Security::On && !entry.signed {
            return Err(ReleaseError::Untrusted {
                name: entry.artifact_name,
                reason: "its manifest has no good signature from a trusted key".into(),
            });
        }

        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| ReleaseError::io(&self.cache_dir, error))?;
        let target = self.cached_target(&entry);
        let size = entry.size_bytes;
        let partial_path = PathBuf::from(format!("{}.partial", target.display()));
        let mut partial = Partial::open(partial_path, size).await?;

        let mut backoff = BACKOFF_FIRST;
        while partial.offset < size {
            let before = partial.offset;
            match self.stream(&url, &mut partial, size, &progress).await? {
                Attempt::Done => {}
                Attempt::Retry if partial.offset > before => backoff = BACKOFF_FIRST,
                Attempt::Retry => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(BACKOFF_CAP);
                }
            }
        }

        let Partial {
            path, file, hasher, ..
        } = partial;
        file.sync_all()
            .await
            .map_err(|error| ReleaseError::io(&path, error))?;
        drop(file);
        let actual = crate::hash::to_hex(&hasher.finalize());
        if !entry.sha256.is_empty() && actual != entry.sha256 {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(ReleaseError::HashMismatch {
                path: target,
                expected: entry.sha256,
                actual,
            });
        }
        tokio::fs::rename(&path, &target)
            .await
            .map_err(|error| ReleaseError::io(&target, error))?;
        entry.path = Some(target);
        self.upsert(entry.clone())?;
        Ok(entry)
    }

    /// One request from the partial's offset to the end. Any failure short of
    /// a fatal one (a 4xx answer, too many bytes) is [`Attempt::Retry`].
    async fn stream(
        &self,
        url: &str,
        partial: &mut Partial,
        size: u64,
        progress: &impl Fn(DownloadProgress),
    ) -> Result<Attempt, ReleaseError> {
        let reply = match self.fetch.get(url, partial.offset).await {
            Ok(reply) => reply,
            Err(_) => return Ok(Attempt::Retry),
        };
        match reply.status {
            200..=299 => {}
            416 => {
                // The server doesn't have that range: start over.
                partial.restart().await?;
                return Ok(Attempt::Retry);
            }
            429 | 500..=599 => return Ok(Attempt::Retry),
            status => {
                return Err(download_error(url, format!("the server answered {status}")));
            }
        }
        // A whole file again, when the server ignored the range: start over.
        if partial.offset > 0 && reply.status != 206 {
            partial.restart().await?;
        }
        let mut body = reply.body;
        loop {
            if partial.offset >= size {
                return Ok(Attempt::Done);
            }
            let piece = match tokio::time::timeout(STALL, body.next_chunk()).await {
                Err(_) | Ok(Err(_)) | Ok(Ok(None)) => return Ok(Attempt::Retry),
                Ok(Ok(Some(piece))) => piece,
            };
            if partial.offset + piece.len() as u64 > size {
                let actual = partial.offset + piece.len() as u64;
                let _ = tokio::fs::remove_file(&partial.path).await;
                return Err(ReleaseError::SizeMismatch {
                    path: partial.path.clone(),
                    expected: size,
                    actual,
                });
            }
            partial.write(&piece).await?;
            progress(DownloadProgress {
                downloaded: partial.offset,
                total: Some(size),
            });
        }
    }
}
