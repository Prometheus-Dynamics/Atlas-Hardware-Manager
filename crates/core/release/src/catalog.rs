use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_types::Family;
use serde::{Deserialize, Serialize};

use crate::fetch::{Fetch, read_all};
use crate::hash::normalize_sha256;
use crate::{
    IMAGE_SUFFIX, Keyring, Manifest, PublicKey, ReleaseError, Security, SignatureStatus,
    check_file, now_ms, sha256_file,
};

mod download;

/// Unpinned local images kept in the list; older ones drop off (the files
/// themselves are never touched).
const KEEP_LOCAL: usize = 5;
/// The largest release index Atlas will read.
const INDEX_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Stable,
    Beta,
    /// A file the user added from disk; signed when a trusted key's
    /// `<file>.minisig` is next to it.
    Local,
}

impl Channel {
    fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "stable" => Self::Stable,
            _ => Self::Beta,
        }
    }
}

/// Where a release came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ReleaseOrigin {
    LocalFile,
    /// From a signed manifest in a remote index.
    Remote {
        source: String,
        url: String,
    },
}

/// One first-flash image, ready to write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseEntry {
    pub id: String,
    pub family: Family,
    pub version: String,
    pub channel: Channel,
    pub origin: ReleaseOrigin,
    pub artifact_name: String,
    /// Empty when unknown (an unsigned remote release that lists no digest).
    pub sha256: String,
    pub size_bytes: u64,
    /// The file on disk: the user's file for local releases, the cache for
    /// downloaded ones. `None` until a remote release is downloaded.
    pub path: Option<PathBuf>,
    /// True when a trusted, unexpired key signed the manifest (a remote
    /// release) or the file (a local one).
    pub signed: bool,
    /// A local file's signature: who signed it and who owns the key, or why
    /// that can't be told. `None` for remote releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<SignatureStatus>,
    pub boards: Vec<String>,
    pub notes_url: Option<String>,
    pub added_ms: u64,
    /// Kept when older unpinned local images are pruned.
    #[serde(default)]
    pub pinned: bool,
}

/// A remote index: a JSON array of signed manifests at `index_url`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSource {
    pub name: String,
    pub index_url: String,
    /// `ed25519:<hex>` keys whose signatures are trusted for this source.
    pub public_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// An image that passed its checks, ready to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct CatalogFile {
    version: u32,
    entries: Vec<ReleaseEntry>,
    #[serde(default)]
    sources: Vec<RemoteSource>,
}

/// The first-flash images Atlas knows about, saved as JSON next to a
/// download cache.
pub struct ReleaseCatalog {
    file: PathBuf,
    cache_dir: PathBuf,
    state: Mutex<CatalogFile>,
    fetch: Arc<dyn Fetch>,
    keyring: Keyring,
    security: Security,
}

impl fmt::Debug for ReleaseCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReleaseCatalog")
            .field("file", &self.file)
            .field("cache_dir", &self.cache_dir)
            .field("security", &self.security)
            .finish_non_exhaustive()
    }
}

fn short(sha256: &str) -> &str {
    sha256.get(..12).unwrap_or(sha256)
}

/// The cache file name for a remote release: the digest's start, or, for a
/// release listed without one, its version, so two versions of one artifact
/// name never share a file.
fn cache_name(entry: &ReleaseEntry) -> String {
    let tag = if entry.sha256.is_empty() {
        format!("v{}", safe_file_name(&entry.version))
    } else {
        short(&entry.sha256).to_owned()
    };
    format!("{tag}-{}", safe_file_name(&entry.artifact_name))
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "artifact".into()
    } else {
        cleaned
    }
}

fn download_error(url: &str, message: String) -> ReleaseError {
    ReleaseError::Download {
        url: url.to_string(),
        message,
    }
}

impl ReleaseCatalog {
    /// Opens the catalog at `file`, downloading through `fetch` into
    /// `cache_dir`. `keyring` is the trusted keys for image files; `security`
    /// decides whether signatures refuse or only report.
    pub fn open(
        file: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        fetch: Arc<dyn Fetch>,
        keyring: Keyring,
        security: Security,
    ) -> Result<Self, ReleaseError> {
        let file = file.into();
        let state = match std::fs::read(&file) {
            Ok(data) => serde_json::from_slice(&data).map_err(|error| ReleaseError::Catalog {
                path: file.clone(),
                message: error.to_string(),
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => CatalogFile {
                version: 1,
                ..CatalogFile::default()
            },
            Err(error) => return Err(ReleaseError::io(&file, error)),
        };
        Ok(Self {
            file,
            cache_dir: cache_dir.into(),
            state: Mutex::new(state),
            fetch,
            keyring,
            security,
        })
    }

    pub fn security(&self) -> Security {
        self.security
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CatalogFile> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn save(&self) -> Result<(), ReleaseError> {
        let data = {
            let state = self.state();
            serde_json::to_vec_pretty(&*state).map_err(|error| ReleaseError::Catalog {
                path: self.file.clone(),
                message: error.to_string(),
            })?
        };
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ReleaseError::io(parent, error))?;
        }
        let temp = self.file.with_extension("json.tmp");
        std::fs::write(&temp, data).map_err(|error| ReleaseError::io(&temp, error))?;
        std::fs::rename(&temp, &self.file).map_err(|error| ReleaseError::io(&self.file, error))
    }

    /// All releases, grouped by family, newest first within each family.
    pub fn entries(&self) -> Vec<ReleaseEntry> {
        let mut entries = self.state().entries.clone();
        entries.reverse();
        entries.sort_by(|a, b| a.family.cmp(&b.family));
        entries
    }

    pub fn entry(&self, id: &str) -> Option<ReleaseEntry> {
        self.state()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }

    pub fn sources(&self) -> Vec<RemoteSource> {
        self.state().sources.clone()
    }

    /// Adds or replaces a remote source. Keys are checked before saving.
    pub fn set_source(&self, source: RemoteSource) -> Result<(), ReleaseError> {
        for key in &source.public_keys {
            PublicKey::parse(key)?;
        }
        {
            let mut state = self.state();
            state
                .sources
                .retain(|existing| existing.name != source.name);
            state.sources.push(source);
        }
        self.save()
    }

    pub fn remove_source(&self, name: &str) -> Result<(), ReleaseError> {
        self.state().sources.retain(|source| source.name != name);
        self.save()
    }

    async fn signature_of(&self, path: &Path) -> SignatureStatus {
        let path = path.to_path_buf();
        let ring = self.keyring.clone();
        tokio::task::spawn_blocking(move || check_file(&path, &ring))
            .await
            .unwrap_or_else(|error| SignatureStatus::Bad {
                reason: error.to_string(),
            })
    }

    /// Adds an image file from disk. It is hashed and its signature read
    /// now, and checked again before use. Under [`Security::On`] a file
    /// without a good, trusted signature is refused and not added.
    pub async fn add_local_file(
        &self,
        path: &Path,
        family: Family,
        version: String,
    ) -> Result<ReleaseEntry, ReleaseError> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.ends_with(IMAGE_SUFFIX) {
            return Err(ReleaseError::NotAnImage(name));
        }
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| ReleaseError::io(path, error))?;
        let sha256 = sha256_file(path).await?;
        let signature = self.signature_of(path).await;
        signature.judge(&name, self.security)?;
        let entry = ReleaseEntry {
            id: format!("local-{}", short(&sha256)),
            family,
            version,
            channel: Channel::Local,
            origin: ReleaseOrigin::LocalFile,
            artifact_name: name,
            sha256,
            size_bytes: metadata.len(),
            path: Some(path.to_path_buf()),
            signed: signature.trusted(),
            signature: Some(signature),
            boards: Vec::new(),
            notes_url: None,
            added_ms: now_ms(),
            pinned: false,
        };
        {
            let mut state = self.state();
            // Re-adding the same file keeps its pin, and a rebuilt file at
            // the same path replaces its old entry instead of piling up.
            let pinned = state.entries.iter().any(|existing| {
                existing.pinned && (existing.id == entry.id || existing.path == entry.path)
            });
            state.entries.retain(|existing| {
                existing.id != entry.id
                    && !(existing.channel == Channel::Local && existing.path == entry.path)
            });
            state.entries.push(ReleaseEntry {
                pinned,
                ..entry.clone()
            });
            prune_local(&mut state.entries);
        }
        self.save()?;
        Ok(self.entry(&entry.id).unwrap_or(entry))
    }

    /// Pins a release so pruning keeps it, or unpins it.
    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), ReleaseError> {
        {
            let mut state = self.state();
            let entry = state
                .entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
            entry.pinned = pinned;
            if !pinned {
                prune_local(&mut state.entries);
            }
        }
        self.save()
    }

    fn upsert(&self, entry: ReleaseEntry) -> Result<(), ReleaseError> {
        {
            let mut state = self.state();
            state.entries.retain(|existing| existing.id != entry.id);
            state.entries.push(entry);
        }
        self.save()
    }

    /// Removes a release. A cached download is deleted; a user's own file
    /// is left alone.
    pub fn remove(&self, id: &str) -> Result<(), ReleaseError> {
        let removed = {
            let mut state = self.state();
            let index = state
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
            state.entries.remove(index)
        };
        if let (ReleaseOrigin::Remote { .. }, Some(path)) = (&removed.origin, &removed.path) {
            let _ = std::fs::remove_file(path);
        }
        self.save()
    }

    /// Adds the releases in a source's manifests. A manifest that isn't an
    /// image, or has a bad digest, is skipped. A manifest without a good
    /// signature is listed as unsigned under [`Security::Off`] and refused
    /// under [`Security::On`]. Returns a warning for each one not listed as
    /// signed.
    pub fn add_manifests(&self, source: &RemoteSource, manifests: Vec<Manifest>) -> Vec<String> {
        let keys: Vec<PublicKey> = source
            .public_keys
            .iter()
            .filter_map(|key| PublicKey::parse(key).ok())
            .collect();
        let mut warnings = Vec::new();
        for manifest in manifests {
            let label = format!(
                "{}: {} {}",
                source.name, manifest.body.family, manifest.body.version
            );
            if let Err(error) = manifest.body.check_image() {
                warnings.push(format!("{label} skipped: {error}"));
                continue;
            }
            let checked = manifest.verify(&keys);
            let body = manifest.body;
            let signed = match checked {
                Ok(()) => true,
                Err(error) if self.security == Security::On => {
                    warnings.push(format!("{label} refused: {error}"));
                    continue;
                }
                Err(error) => {
                    warnings.push(format!("{label} listed as unsigned: {error}"));
                    false
                }
            };
            let raw = body.artifact.sha256.trim();
            let sha256 = if raw.is_empty() {
                String::new()
            } else if let Some(digest) = normalize_sha256(raw) {
                digest
            } else {
                warnings.push(format!("{label} skipped: its SHA-256 is not a digest"));
                continue;
            };
            let id_suffix = if sha256.is_empty() {
                "nohash"
            } else {
                short(&sha256)
            };
            let id = format!("{}-{}-{}", body.family, body.version, id_suffix);
            let existing_path = self
                .entry(&id)
                .and_then(|entry| entry.path)
                .filter(|path| path.exists());
            let entry = ReleaseEntry {
                id,
                family: Family::new(body.family),
                version: body.version,
                channel: Channel::parse(&body.channel),
                origin: ReleaseOrigin::Remote {
                    source: source.name.clone(),
                    url: body.artifact.url,
                },
                artifact_name: body.artifact.name,
                sha256,
                size_bytes: body.artifact.size,
                path: existing_path,
                signed,
                signature: None,
                boards: body.boards,
                notes_url: body.notes_url,
                added_ms: now_ms(),
                pinned: false,
            };
            if let Err(error) = self.upsert(entry) {
                warnings.push(error.to_string());
            }
        }
        warnings
    }

    /// Fetches every source's index and adds the releases in it.
    pub async fn refresh(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for source in self.sources() {
            match self.fetch_index(&source).await {
                Ok(manifests) => warnings.extend(self.add_manifests(&source, manifests)),
                Err(error) => warnings.push(format!("{}: {error}", source.name)),
            }
        }
        warnings
    }

    async fn fetch_index(&self, source: &RemoteSource) -> Result<Vec<Manifest>, ReleaseError> {
        let bytes = read_all(self.fetch.as_ref(), &source.index_url, INDEX_LIMIT)
            .await
            .map_err(|message| download_error(&source.index_url, message))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| download_error(&source.index_url, error.to_string()))
    }

    /// The file to write for a release, checked against its size and
    /// SHA-256 (when known) now, and against its signature under
    /// [`Security::On`].
    pub async fn artifact(&self, id: &str) -> Result<Artifact, ReleaseError> {
        let entry = self
            .entry(id)
            .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
        let path = entry
            .path
            .clone()
            .filter(|path| path.exists())
            .ok_or_else(|| ReleaseError::NotDownloaded(id.to_string()))?;
        let actual = sha256_file(&path).await?;
        if !entry.sha256.is_empty() && actual != entry.sha256 {
            return Err(ReleaseError::HashMismatch {
                path,
                expected: entry.sha256,
                actual,
            });
        }
        let size_bytes = tokio::fs::metadata(&path)
            .await
            .map_err(|error| ReleaseError::io(&path, error))?
            .len();
        if size_bytes != entry.size_bytes {
            return Err(ReleaseError::SizeMismatch {
                path,
                expected: entry.size_bytes,
                actual: size_bytes,
            });
        }
        if self.security == Security::On {
            match entry.origin {
                ReleaseOrigin::LocalFile => {
                    self.signature_of(&path)
                        .await
                        .judge(&entry.artifact_name, self.security)?;
                }
                ReleaseOrigin::Remote { .. } if !entry.signed => {
                    return Err(ReleaseError::Untrusted {
                        name: entry.artifact_name,
                        reason: "its manifest has no good signature from a trusted key".into(),
                    });
                }
                ReleaseOrigin::Remote { .. } => {}
            }
        }
        Ok(Artifact {
            name: entry.artifact_name,
            path,
            sha256: actual,
            size_bytes,
        })
    }

    fn cached_target(&self, entry: &ReleaseEntry) -> PathBuf {
        self.cache_dir.join(cache_name(entry))
    }
}

/// Drops the oldest unpinned local images beyond [`KEEP_LOCAL`]. Entries are
/// kept in the order they were added, so the front is the oldest.
fn prune_local(entries: &mut Vec<ReleaseEntry>) {
    let unpinned: Vec<String> = entries
        .iter()
        .filter(|entry| entry.channel == Channel::Local && !entry.pinned)
        .map(|entry| entry.id.clone())
        .collect();
    if unpinned.len() <= KEEP_LOCAL {
        return;
    }
    let drop = &unpinned[..unpinned.len() - KEEP_LOCAL];
    entries.retain(|entry| !drop.contains(&entry.id));
}

#[cfg(test)]
mod tests;
