//! Updates of Atlas itself, from the GitHub releases of its repository.
//!
//! A release is a tag `v<version>` with the installers as assets, each next
//! to `<installer>.manifest.json`: a signed [`Manifest`] (family
//! [`APP_FAMILY`]) naming the installer, its size and SHA-256. CI writes
//! both (.github/workflows/ci.yml, `publish`).
//!
//! The installer for this computer is the one for how Atlas was installed
//! here ([`InstallKind`]) and its architecture. Before it is used, its
//! SHA-256 must match: the manifest's, verified against the keys Atlas was
//! built with when it has any (then a release without a valid signature is
//! refused), else the manifest's or GitHub's own digest, and the result says
//! it wasn't signed.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::hash::{normalize_sha256, to_hex};
use crate::{DownloadProgress, Manifest, PublicKey, ReleaseError};

/// The manifest family of Atlas's own installers.
pub const APP_FAMILY: &str = "atlas-app";
const MANIFEST_SUFFIX: &str = ".manifest.json";
const API: &str = "https://api.github.com";

/// How Atlas was installed on this computer: which installer replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    Rpm,
    Deb,
    AppImage,
    /// The Windows setup .exe.
    Nsis,
    Msi,
    Dmg,
    /// Built from source or run from a build directory: nothing to replace.
    Source,
}

impl InstallKind {
    /// Whether `name` is this kind's installer for `arch` (Rust's
    /// `std::env::consts::ARCH`).
    pub fn matches(self, name: &str, arch: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(MANIFEST_SUFFIX) {
            return false;
        }
        let ext = match self {
            Self::Rpm => ".rpm",
            Self::Deb => ".deb",
            Self::AppImage => ".appimage",
            Self::Nsis => "-setup.exe",
            Self::Msi => ".msi",
            Self::Dmg => ".dmg",
            Self::Source => return false,
        };
        if !lower.ends_with(ext) || lower.contains("-src-tauri-") {
            return false;
        }
        // CI's own binaries are named like the app; installers carry a
        // version and, for Linux packages, an architecture.
        let arches: &[&str] = match arch {
            "x86_64" => &["x86_64", "amd64", "x64"],
            "aarch64" => &["aarch64", "arm64"],
            other => return lower.contains(other),
        };
        let other_arch = ["x86_64", "amd64", "x64", "aarch64", "arm64"]
            .iter()
            .any(|a| lower.contains(a) && !arches.contains(a));
        arches.iter().any(|a| lower.contains(a)) || !other_arch
    }
}

/// A release of Atlas newer than the running one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRelease {
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    pub published_at: Option<String>,
    /// The release's page.
    pub url: String,
    /// Its notes (Markdown), shortened.
    pub notes: String,
    /// The installer for this computer, when the release has one.
    pub installer: Option<AppInstaller>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInstaller {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub manifest_url: Option<String>,
    /// GitHub's SHA-256 of the asset, when it gives one.
    pub digest: Option<String>,
}

/// A downloaded installer whose SHA-256 matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedInstaller {
    pub path: PathBuf,
    pub sha256: String,
    /// The manifest's signature verified against a built-in key.
    pub signed: bool,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// Compares dotted versions numerically (`2026.10.0` > `2026.9.1`); a
/// pre-release (`-rc.1`) sorts before its release.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.trim().trim_start_matches('v');
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (v, None),
        };
        (
            core.split('.')
                .map(|part| part.parse().unwrap_or(0))
                .collect(),
            pre,
        )
    }
    let (ac, ap) = split(a);
    let (bc, bp) = split(b);
    let len = ac.len().max(bc.len());
    for i in 0..len {
        let order = ac.get(i).unwrap_or(&0).cmp(bc.get(i).unwrap_or(&0));
        if order.is_ne() {
            return order;
        }
    }
    match (ap, bp) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => a.cmp(b),
    }
}

/// What to look for.
pub struct AppUpdateQuery<'a> {
    /// `owner/name`.
    pub repo: &'a str,
    pub current: &'a str,
    pub prereleases: bool,
    pub kind: InstallKind,
    pub arch: &'a str,
}

/// The newest release newer than `query.current`, if any.
pub async fn latest_app_release(
    http: &reqwest::Client,
    query: &AppUpdateQuery<'_>,
) -> Result<Option<AppRelease>, ReleaseError> {
    let url = format!("{API}/repos/{}/releases?per_page=20", query.repo);
    let error = |message: String| ReleaseError::Download {
        url: url.clone(),
        message,
    };
    let releases: Vec<GhRelease> = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| error(e.to_string()))?
        .json()
        .await
        .map_err(|e| error(e.to_string()))?;
    Ok(pick(releases, query))
}

fn pick(releases: Vec<GhRelease>, query: &AppUpdateQuery<'_>) -> Option<AppRelease> {
    let newest = releases
        .into_iter()
        .filter(|r| !r.draft && (query.prereleases || !r.prerelease))
        .filter(|r| r.tag_name.starts_with('v'))
        .max_by(|a, b| version_cmp(&a.tag_name, &b.tag_name))?;
    let version = newest.tag_name.trim_start_matches('v').to_string();
    if version_cmp(&version, query.current).is_le() {
        return None;
    }
    let installer = newest
        .assets
        .iter()
        .find(|asset| query.kind.matches(&asset.name, query.arch))
        .map(|asset| {
            let manifest = format!("{}{MANIFEST_SUFFIX}", asset.name);
            AppInstaller {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                size: asset.size,
                manifest_url: newest
                    .assets
                    .iter()
                    .find(|a| a.name == manifest)
                    .map(|a| a.browser_download_url.clone()),
                digest: asset
                    .digest
                    .as_deref()
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .and_then(normalize_sha256),
            }
        });
    let mut notes = newest.body.unwrap_or_default();
    if notes.len() > 4000 {
        let cut = (0..=4000)
            .rev()
            .find(|&i| notes.is_char_boundary(i))
            .unwrap_or(0);
        notes.truncate(cut);
        notes.push('…');
    }
    Some(AppRelease {
        version,
        tag: newest.tag_name,
        prerelease: newest.prerelease,
        published_at: newest.published_at,
        url: newest.html_url,
        notes,
        installer,
    })
}

/// The SHA-256 the installer must have, and whether a key vouched for it.
async fn expected_sha256(
    http: &reqwest::Client,
    release: &AppRelease,
    installer: &AppInstaller,
    keys: &[PublicKey],
) -> Result<(String, bool), ReleaseError> {
    let unsigned = || ReleaseError::BadSignature {
        family: APP_FAMILY.into(),
        version: release.version.clone(),
    };
    let Some(manifest_url) = &installer.manifest_url else {
        if !keys.is_empty() {
            return Err(unsigned());
        }
        return installer
            .digest
            .clone()
            .map(|sha| (sha, false))
            .ok_or_else(|| {
                ReleaseError::InvalidManifest(format!(
                    "{} has no manifest or checksum to check it against",
                    installer.name
                ))
            });
    };
    let manifest: Manifest = http
        .get(manifest_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| ReleaseError::Download {
            url: manifest_url.clone(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| ReleaseError::InvalidManifest(e.to_string()))?;
    let signed = if keys.is_empty() {
        false
    } else {
        manifest.verify(keys)?;
        true
    };
    let body = &manifest.body;
    if body.family != APP_FAMILY
        || version_cmp(&body.version, &release.version).is_ne()
        || body.artifact.name != installer.name
    {
        return Err(ReleaseError::InvalidManifest(format!(
            "the manifest is for {} {} ({}), not {}",
            body.family, body.version, body.artifact.name, installer.name
        )));
    }
    let sha = normalize_sha256(&body.artifact.sha256)
        .ok_or_else(|| ReleaseError::InvalidManifest("its SHA-256 is malformed".into()))?;
    Ok((sha, signed))
}

/// Downloads the release's installer into `dir` and checks it.
pub async fn download_app_installer(
    http: &reqwest::Client,
    release: &AppRelease,
    keys: &[PublicKey],
    dir: &Path,
    progress: impl Fn(DownloadProgress) + Send,
) -> Result<VerifiedInstaller, ReleaseError> {
    let installer = release.installer.as_ref().ok_or_else(|| {
        ReleaseError::UnknownRelease(format!(
            "{} has no installer for this computer",
            release.tag
        ))
    })?;
    let (expected, signed) = expected_sha256(http, release, installer, keys).await?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| ReleaseError::io(dir, e))?;
    let name: String = installer
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let target = dir.join(&name);
    let partial = dir.join(format!("{name}.partial"));
    let error = |message: String| ReleaseError::Download {
        url: installer.url.clone(),
        message,
    };
    let response = http
        .get(&installer.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| error(e.to_string()))?;
    let total = response.content_length().or(Some(installer.size));
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| ReleaseError::io(&partial, e))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| error(e.to_string()))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| ReleaseError::io(&partial, e))?;
        downloaded += chunk.len() as u64;
        progress(DownloadProgress { downloaded, total });
    }
    file.sync_all()
        .await
        .map_err(|e| ReleaseError::io(&partial, e))?;
    drop(file);
    let actual = to_hex(&hasher.finalize());
    if actual != expected {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(ReleaseError::HashMismatch {
            path: target,
            expected,
            actual,
        });
    }
    tokio::fs::rename(&partial, &target)
        .await
        .map_err(|e| ReleaseError::io(&target, e))?;
    Ok(VerifiedInstaller {
        path: target,
        sha256: actual,
        signed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(version_cmp("2026.10.0", "2026.9.1"), Greater);
        assert_eq!(version_cmp("v2026.1.0", "2026.1.0"), Equal);
        assert_eq!(version_cmp("2026.1.0-rc.1", "2026.1.0"), Less);
        assert_eq!(version_cmp("2026.1", "2026.1.0"), Equal);
        assert_eq!(version_cmp("0.1.0", "2026.0.0"), Less);
    }

    #[test]
    fn installers_match_kind_and_arch() {
        let rpm = "Atlas.Hardware.Manager-2026.0.0-1.x86_64.rpm";
        assert!(InstallKind::Rpm.matches(rpm, "x86_64"));
        assert!(!InstallKind::Rpm.matches(rpm, "aarch64"));
        assert!(!InstallKind::Rpm.matches(&format!("{rpm}.manifest.json"), "x86_64"));
        assert!(!InstallKind::Rpm.matches(
            "Atlas.Hardware.Manager-2026.0.0-1.x86_64-src-tauri-target-x86_64-unknown-linux-gnu-release-bundle-rpm.rpm",
            "x86_64"
        ));
        assert!(InstallKind::Deb.matches("Atlas.Hardware.Manager_2026.0.0_amd64.deb", "x86_64"));
        assert!(
            InstallKind::Nsis.matches("Atlas Hardware Manager_2026.0.0_x64-setup.exe", "x86_64")
        );
        assert!(!InstallKind::Source.matches(rpm, "x86_64"));
        assert!(!InstallKind::Nsis.matches("atlas-hardware-manager.exe", "x86_64"));
    }

    fn release(tag: &str, prerelease: bool, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            published_at: None,
            html_url: format!("https://github.com/o/r/releases/tag/{tag}"),
            body: Some("notes".into()),
            assets: assets
                .iter()
                .map(|name| GhAsset {
                    name: (*name).into(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                    size: 10,
                    digest: Some(format!("sha256:{}", "ab".repeat(32))),
                })
                .collect(),
        }
    }

    #[test]
    fn picks_the_newest_newer_release_and_its_installer() {
        let query = AppUpdateQuery {
            repo: "o/r",
            current: "2026.1.0",
            prereleases: false,
            kind: InstallKind::Rpm,
            arch: "x86_64",
        };
        let releases = || {
            vec![
                release("v2026.1.0", false, &[]),
                release("v2026.3.0-rc.1", true, &[]),
                release(
                    "v2026.2.0",
                    false,
                    &[
                        "A-2026.2.0-1.x86_64.rpm",
                        "A-2026.2.0-1.x86_64.rpm.manifest.json",
                        "A_2026.2.0_amd64.deb",
                    ],
                ),
            ]
        };
        let found = pick(releases(), &query).unwrap();
        assert_eq!(found.version, "2026.2.0");
        let installer = found.installer.unwrap();
        assert_eq!(installer.name, "A-2026.2.0-1.x86_64.rpm");
        assert!(installer.manifest_url.unwrap().ends_with(".manifest.json"));
        assert_eq!(installer.digest.unwrap(), "ab".repeat(32));

        let with_pre = AppUpdateQuery {
            prereleases: true,
            ..query
        };
        assert_eq!(
            pick(releases(), &with_pre).unwrap().version,
            "2026.3.0-rc.1"
        );
        let current = AppUpdateQuery {
            current: "2026.2.0",
            ..with_pre
        };
        assert_eq!(pick(vec![release("v2026.2.0", false, &[])], &current), None);
    }
}
