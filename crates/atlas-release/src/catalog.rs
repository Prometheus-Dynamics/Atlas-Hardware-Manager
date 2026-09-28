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
        };
        self.upsert(entry.clone())?;
        Ok(entry)
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
            if let Err(error) = manifest.verify(&keys) {
                warnings.push(format!("{}: skipped: {error}", source.name));
                continue;
            }
            let body = manifest.body;
            let Some(sha256) = normalize_sha256(&body.artifact.sha256) else {
                continue;
            };
            let id = format!("{}-{}-{}", body.family, body.version, short(&sha256));
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
                signed: true,
                boards: body.boards,
                notes_url: body.notes_url,
                added_ms: now_ms(),
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
        if actual != entry.sha256 {
            let _ = tokio::fs::remove_file(&partial).await;
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
        if actual != entry.sha256 {
            return Err(ReleaseError::HashMismatch {
                path,
                expected: entry.sha256,
                actual,
            });
        }
        Ok(Artifact {
            name: entry.artifact_name,
            path,
            sha256: entry.sha256,
            size_bytes: entry.size_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::public_key_for;
    use crate::manifest::tests::{SECRET, body};
    use crate::sign_manifest;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-release-{label}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn source() -> RemoteSource {
        RemoteSource {
            name: "helios".into(),
            index_url: "https://example.invalid/index.json".into(),
            public_keys: vec![public_key_for(&SECRET).to_string()],
        }
    }

    #[tokio::test]
    async fn local_files_are_hashed_and_rechecked_before_use() {
        let dir = temp_dir("local");
        let image = dir.join("image.img");
        std::fs::write(&image, b"abc").unwrap();
        let catalog = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();

        let entry = catalog
            .add_local_file(&image, Family::new("rpi"), "dev".into())
            .await
            .unwrap();
        assert!(!entry.signed);
        assert_eq!(catalog.artifact(&entry.id).await.unwrap().size_bytes, 3);

        std::fs::write(&image, b"tampered").unwrap();
        assert!(matches!(
            catalog.artifact(&entry.id).await,
            Err(ReleaseError::HashMismatch { .. })
        ));

        let reopened = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
        assert_eq!(reopened.entries().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn only_signed_manifests_are_added() {
        let dir = temp_dir("manifests");
        let catalog = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
        let good = sign_manifest(body("2026.3.1"), &SECRET).unwrap();
        let forged = sign_manifest(body("2026.9.9"), &[1u8; 32]).unwrap();

        let warnings = catalog.add_manifests(&source(), vec![good, forged]);

        assert_eq!(warnings.len(), 1);
        let entries = catalog.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].signed);
        assert_eq!(entries[0].channel, Channel::Stable);
        assert!(matches!(
            catalog.artifact(&entries[0].id).await,
            Err(ReleaseError::NotDownloaded(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sources_with_bad_keys_are_refused() {
        let dir = temp_dir("sources");
        let catalog = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
        let mut bad = source();
        bad.public_keys = vec!["ed25519:00".into()];

        assert!(catalog.set_source(bad).is_err());
        assert!(catalog.set_source(source()).is_ok());
        assert_eq!(catalog.sources().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cache_file_names_are_sanitized() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("helios 1.img.xz"), "helios_1.img.xz");
        assert_eq!(safe_file_name(".."), "artifact");
    }
}
