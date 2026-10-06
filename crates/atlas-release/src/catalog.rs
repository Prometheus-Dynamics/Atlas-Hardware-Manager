use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use atlas_driver::{Artifact, Family};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::hash::{normalize_sha256, to_hex};
use crate::{Manifest, PublicKey, ReleaseError, sha256_file};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Stable,
    Beta,
    /// A file the user added from disk. Unsigned.
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

/// One installable release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseEntry {
    pub id: String,
    pub family: Family,
    pub version: String,
    pub channel: Channel,
    pub origin: ReleaseOrigin,
    pub artifact_name: String,
    pub sha256: String,
    pub size_bytes: u64,
    /// The file on disk: the user's file for local releases, the cache for
    /// downloaded ones. `None` until a remote release is downloaded.
    pub path: Option<PathBuf>,
    /// True when a trusted key signed the manifest.
    pub signed: bool,
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

#[derive(Default, Serialize, Deserialize)]
struct CatalogFile {
    version: u32,
    entries: Vec<ReleaseEntry>,
    #[serde(default)]
    sources: Vec<RemoteSource>,
}

/// The releases Atlas knows about, saved as JSON next to a download cache.
pub struct ReleaseCatalog {
    file: PathBuf,
    cache_dir: PathBuf,
    state: Mutex<CatalogFile>,
    http: reqwest::Client,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn short(sha256: &str) -> &str {
    sha256.get(..12).unwrap_or(sha256)
}

/// Where a download that failed its checksum is kept.
fn unverified_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".unverified");
    target.with_file_name(name)
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

impl ReleaseCatalog {
    /// Opens the catalog at `file`, downloading into `cache_dir`.
    pub fn open(
        file: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
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
        let http = reqwest::Client::builder()
            .user_agent(concat!("atlas/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| ReleaseError::Download {
                url: String::new(),
                message: error.to_string(),
            })?;
        Ok(Self {
            file,
            cache_dir: cache_dir.into(),
            state: Mutex::new(state),
            http,
        })
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

    /// All releases, newest first within each family.
    pub fn entries(&self) -> Vec<ReleaseEntry> {
        let mut entries = self.state().entries.clone();
        entries.sort_by(|a, b| {
            (a.family.clone(), std::cmp::Reverse(a.added_ms))
                .cmp(&(b.family.clone(), std::cmp::Reverse(b.added_ms)))
        });
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

    /// Adds a file from disk. It is hashed now and checked again before use.
    pub async fn add_local_file(
        &self,
        path: &Path,
        family: Family,
        version: String,
    ) -> Result<ReleaseEntry, ReleaseError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| ReleaseError::io(path, error))?;
        let sha256 = sha256_file(path).await?;
        let entry = ReleaseEntry {
            id: format!("local-{}", short(&sha256)),
            family,
            version,
            channel: Channel::Local,
            origin: ReleaseOrigin::LocalFile,
            artifact_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".into()),
            sha256,
            size_bytes: metadata.len(),
            path: Some(path.to_path_buf()),
            signed: false,
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

    /// Adds the releases in manifests signed by the source's keys and
    /// returns warnings for the ones that were rejected.
    pub fn add_manifests(&self, source: &RemoteSource, manifests: Vec<Manifest>) -> Vec<String> {
        let keys: Vec<PublicKey> = source
            .public_keys
            .iter()
            .filter_map(|key| PublicKey::parse(key).ok())
            .collect();
        let mut warnings = Vec::new();
        for manifest in manifests {
            // Signatures are checked when they can be, never required: an
            // unverified release is listed as unsigned and the UI warns.
            let signed = match manifest.verify(&keys) {
                Ok(()) => true,
                Err(error) => {
                    if !keys.is_empty() {
                        warnings.push(format!(
                            "{}: {} {} listed as unsigned: {error}",
                            source.name, manifest.body.family, manifest.body.version
                        ));
                    }
                    false
                }
            };
            let body = manifest.body;
            // An empty digest means "unknown": the file is used unchecked.
            let sha256 = normalize_sha256(&body.artifact.sha256).unwrap_or_default();
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

    /// Fetches every source's index and adds the signed releases in it.
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
        let download_error = |message: String| ReleaseError::Download {
            url: source.index_url.clone(),
            message,
        };
        let response = self
            .http
            .get(&source.index_url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| download_error(error.to_string()))?;
        response
            .json::<Vec<Manifest>>()
            .await
            .map_err(|error| download_error(error.to_string()))
    }

    /// Downloads a remote release into the cache, checking its SHA-256
    /// before it is kept.
    pub async fn download(
        &self,
        id: &str,
        progress: impl Fn(DownloadProgress) + Send,
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

        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .map_err(|error| ReleaseError::io(&self.cache_dir, error))?;
        let target = self.cache_dir.join(format!(
            "{}-{}",
            short(&entry.sha256),
            safe_file_name(&entry.artifact_name)
        ));
        let partial = target.with_extension("partial");
        let download_error = |message: String| ReleaseError::Download {
            url: url.clone(),
            message,
        };

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| download_error(error.to_string()))?;
        let total = response.content_length();
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|error| ReleaseError::io(&partial, error))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| download_error(error.to_string()))?;
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| ReleaseError::io(&partial, error))?;
            downloaded += chunk.len() as u64;
            progress(DownloadProgress { downloaded, total });
        }
        file.sync_all()
            .await
            .map_err(|error| ReleaseError::io(&partial, error))?;
        drop(file);

        let actual = to_hex(&hasher.finalize());
        if !entry.sha256.is_empty() && actual != entry.sha256 {
            // Kept aside so the user can still choose to use it.
            let _ = tokio::fs::rename(&partial, unverified_path(&target)).await;
            return Err(ReleaseError::HashMismatch {
                path: target,
                expected: entry.sha256,
                actual,
            });
        }
        tokio::fs::rename(&partial, &target)
            .await
            .map_err(|error| ReleaseError::io(&target, error))?;
        entry.path = Some(target);
        self.upsert(entry.clone())?;
        Ok(entry)
    }

    /// The file to install for a release, re-checked against its SHA-256.
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
            .map(|meta| meta.len())
            .unwrap_or(entry.size_bytes);
        Ok(Artifact {
            name: entry.artifact_name,
            path,
            sha256: actual,
            size_bytes,
        })
    }

    /// The file for a release without comparing it to the expected digest:
    /// the user chose "flash anyway" after a checksum mismatch. Uses the
    /// downloaded copy that failed its check when there is no good one.
    pub async fn artifact_unchecked(&self, id: &str) -> Result<Artifact, ReleaseError> {
        let entry = self
            .entry(id)
            .ok_or_else(|| ReleaseError::UnknownRelease(id.to_string()))?;
        let candidates = [
            entry.path.clone(),
            self.cached_target(&entry)
                .map(|target| unverified_path(&target)),
        ];
        let path = candidates
            .into_iter()
            .flatten()
            .find(|path| path.exists())
            .ok_or_else(|| ReleaseError::NotDownloaded(id.to_string()))?;
        let actual = sha256_file(&path).await?;
        let size_bytes = tokio::fs::metadata(&path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(entry.size_bytes);
        Ok(Artifact {
            name: entry.artifact_name,
            path,
            sha256: actual,
            size_bytes,
        })
    }

    fn cached_target(&self, entry: &ReleaseEntry) -> Option<PathBuf> {
        matches!(entry.origin, ReleaseOrigin::Remote { .. }).then(|| {
            self.cache_dir.join(format!(
                "{}-{}",
                short(&entry.sha256),
                safe_file_name(&entry.artifact_name)
            ))
        })
    }
}

mod local;

use local::prune_local;
pub use local::{KEEP_LOCAL, local_artifact};

#[cfg(test)]
mod tests;
