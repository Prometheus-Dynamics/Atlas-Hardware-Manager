//! Updates of Atlas itself, from a signed release feed.
//!
//! A feed is one JSON file: a list of signed [`Manifest`]s. An app release is a manifest whose
//! family is [`APP_FAMILY`] and whose artifact is an installer (a `.deb`, an `.AppImage`, a
//! setup `.exe`, ...). [`check_app_update`] reads the feed and answers with the newest release
//! of one channel that is newer than the running version and has an installer for how Atlas is
//! installed here ([`InstallKind`]). [`download_app_update`] fetches that installer and checks
//! its size and SHA-256.
//!
//! Nothing here is ruled by the security switch. An app release is used only when its manifest
//! is signed by one of the keys given, and its installer only when it hashes to what that
//! manifest says: an unsigned app is never offered, whatever the switch says.
//!
//! Carried from `dev` (`crates/atlas-release/src/app_update.rs`), each for its own reason:
//! the family name `atlas-app` (what release tooling already writes), [`InstallKind::matches`]
//! (the installers' real file names) and [`version_cmp`] (the version scheme is unchanged).
//! Not carried: reading GitHub's release API and falling back to GitHub's own digest when no
//! key is built in. The feed is a plain signed file, and without a signature there is no offer.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{INDEX_LIMIT, fetch_file};
use crate::fetch::read_all;
use crate::hash::normalize_sha256;
use crate::{DownloadProgress, Fetch, Manifest, PublicKey, ReleaseError};

/// The manifest family of Atlas's own installers.
pub const APP_FAMILY: &str = "atlas-app";

/// How Atlas was installed on this computer: which installer replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    Deb,
    Rpm,
    AppImage,
    /// The Windows setup `.exe`.
    Nsis,
    Msi,
    Dmg,
    /// Built from source or run from a build directory: nothing to replace.
    Source,
}

impl InstallKind {
    /// Whether `name` is this kind's installer for `arch` (Rust's `std::env::consts::ARCH`).
    /// A name that states no architecture fits any.
    #[must_use]
    pub fn matches(self, name: &str, arch: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        let ext = match self {
            Self::Deb => ".deb",
            Self::Rpm => ".rpm",
            Self::AppImage => ".appimage",
            Self::Nsis => "-setup.exe",
            Self::Msi => ".msi",
            Self::Dmg => ".dmg",
            Self::Source => return false,
        };
        if !lower.ends_with(ext) {
            return false;
        }
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

/// Compares dotted versions numerically (`2026.10.0` > `2026.9.1`). A pre-release (`-rc.1`,
/// `-dev.721`) sorts before its release, and pre-releases compare part by part as semver does
/// (`dev.99` < `dev.100`). Build metadata (`+ab12cd34`) is ignored.
#[must_use]
pub fn version_cmp(a: &str, b: &str) -> Ordering {
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
    fn pre_cmp(a: &str, b: &str) -> Ordering {
        let (mut a, mut b) = (a.split('.'), b.split('.'));
        loop {
            match (a.next(), b.next()) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(x), Some(y)) => {
                    let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => x.cmp(y),
                    };
                    if order != Ordering::Equal {
                        return order;
                    }
                }
            }
        }
    }
    let (ac, ap) = split(a);
    let (bc, bp) = split(b);
    for i in 0..ac.len().max(bc.len()) {
        let order = ac
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&bc.get(i).copied().unwrap_or(0));
        if order != Ordering::Equal {
            return order;
        }
    }
    match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => pre_cmp(a, b),
    }
}

/// What to look for.
#[derive(Clone, Copy, Debug)]
pub struct AppQuery<'a> {
    /// The feed's address.
    pub feed_url: &'a str,
    /// The channel wanted, compared with the manifest's as written.
    pub channel: &'a str,
    /// The running version.
    pub current: &'a str,
    pub kind: InstallKind,
    /// `std::env::consts::ARCH`.
    pub arch: &'a str,
}

/// A release of Atlas newer than the running one, whose manifest a trusted key signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUpdate {
    pub version: String,
    pub channel: String,
    /// The installer's file name: no directory in it.
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
    pub notes_url: Option<String>,
}

/// A downloaded installer whose size and SHA-256 matched its signed manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedInstaller {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

fn feed_error(url: &str, message: String) -> ReleaseError {
    ReleaseError::Download {
        url: url.to_owned(),
        message,
    }
}

/// Reads the feed and answers with the newest release of the channel that is newer than the
/// running version and has an installer of this kind, or `None` when there is none.
///
/// The newest such release must be signed by one of `keys`: if it is not, the answer is
/// [`ReleaseError::BadSignature`], never an older release in its place (a feed that carries an
/// unsigned newest release has been changed by someone, and saying so is the point).
pub async fn check_app_update(
    fetch: &dyn Fetch,
    keys: &[PublicKey],
    query: AppQuery<'_>,
) -> Result<Option<AppUpdate>, ReleaseError> {
    let bytes = read_all(fetch, query.feed_url, INDEX_LIMIT)
        .await
        .map_err(|message| feed_error(query.feed_url, message))?;
    let manifests: Vec<Manifest> = serde_json::from_slice(&bytes)
        .map_err(|error| feed_error(query.feed_url, format!("not a release feed: {error}")))?;
    let newest = manifests
        .into_iter()
        .filter(|manifest| manifest.body.family == APP_FAMILY)
        .filter(|manifest| manifest.body.channel == query.channel)
        .filter(|manifest| query.kind.matches(&manifest.body.artifact.name, query.arch))
        .filter(|manifest| version_cmp(&manifest.body.version, query.current) == Ordering::Greater)
        .max_by(|a, b| version_cmp(&a.body.version, &b.body.version));
    let Some(manifest) = newest else {
        return Ok(None);
    };
    manifest.verify(keys)?;
    let body = manifest.body;
    let artifact = body.artifact;
    if artifact.name.contains(['/', '\\']) || artifact.name.starts_with('.') {
        return Err(ReleaseError::InvalidManifest(format!(
            "artifact.name `{}` is not a plain file name",
            artifact.name
        )));
    }
    if artifact.size == 0 {
        return Err(ReleaseError::InvalidManifest("artifact.size is 0".into()));
    }
    if !(artifact.url.starts_with("https://") || artifact.url.starts_with("http://")) {
        return Err(ReleaseError::InvalidManifest(
            "artifact.url is not an http(s) address".into(),
        ));
    }
    // `verify` has already refused a manifest without a SHA-256.
    let sha256 = normalize_sha256(&artifact.sha256).unwrap_or_default();
    Ok(Some(AppUpdate {
        version: body.version,
        channel: body.channel,
        name: artifact.name,
        url: artifact.url,
        size: artifact.size,
        sha256,
        notes_url: body.notes_url,
    }))
}

/// Downloads the installer of `update` into `dir`, resuming a partial file, and checks its size
/// and SHA-256. It does not give up while the server may still answer; the caller cancels by
/// dropping the future. A file that fails its check is deleted.
pub async fn download_app_update(
    fetch: &dyn Fetch,
    update: &AppUpdate,
    dir: &Path,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
) -> Result<VerifiedInstaller, ReleaseError> {
    if update.sha256.is_empty() {
        return Err(ReleaseError::InvalidManifest(
            "the release has no SHA-256".into(),
        ));
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| ReleaseError::io(dir, error))?;
    let path = dir.join(&update.name);
    // A whole file from an earlier run is used only if it still checks.
    if tokio::fs::metadata(&path).await.is_ok() {
        if crate::sha256_file(&path).await.ok().as_deref() == Some(update.sha256.as_str()) {
            return Ok(VerifiedInstaller {
                path,
                sha256: update.sha256.clone(),
                size: update.size,
            });
        }
        let _ = tokio::fs::remove_file(&path).await;
    }
    fetch_file(
        fetch,
        &update.url,
        &path,
        update.size,
        &update.sha256,
        progress,
    )
    .await?;
    Ok(VerifiedInstaller {
        path,
        sha256: update.sha256.clone(),
        size: update.size,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::manifest::tests::throwaway;
    use crate::{
        Body, ManifestArtifact, ManifestBody, Reply, Requirements, public_key_for, sign_manifest,
    };

    /// A feed in memory: each address answers with its bytes, from the offset asked.
    #[derive(Default)]
    struct Feed {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        offsets: Mutex<Vec<u64>>,
    }

    struct Whole(Option<Vec<u8>>);

    #[async_trait]
    impl Body for Whole {
        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.take())
        }
    }

    #[async_trait]
    impl Fetch for Feed {
        async fn get(&self, url: &str, offset: u64) -> Result<Reply, String> {
            self.offsets.lock().unwrap().push(offset);
            let Some(bytes) = self.files.lock().unwrap().get(url).cloned() else {
                return Ok(Reply {
                    status: 404,
                    body: Box::new(Whole(None)),
                });
            };
            let from = usize::try_from(offset).unwrap().min(bytes.len());
            Ok(Reply {
                status: if offset > 0 { 206 } else { 200 },
                body: Box::new(Whole(Some(bytes[from..].to_vec()))),
            })
        }
    }

    const FEED: &str = "https://example.invalid/feed.json";

    fn release(version: &str, channel: &str, name: &str, contents: &[u8]) -> ManifestBody {
        ManifestBody {
            manifest_version: 1,
            family: APP_FAMILY.into(),
            version: version.into(),
            channel: channel.into(),
            boards: Vec::new(),
            artifact: ManifestArtifact {
                name: name.into(),
                url: format!("https://example.invalid/{name}"),
                size: contents.len() as u64,
                sha256: crate::hash::to_hex(&Sha256::digest(contents)),
            },
            requires: Requirements::default(),
            methods: Vec::new(),
            notes_url: None,
        }
    }

    fn query(current: &str) -> AppQuery<'_> {
        AppQuery {
            feed_url: FEED,
            channel: "stable",
            current,
            kind: InstallKind::Deb,
            arch: "x86_64",
        }
    }

    fn feed_of(manifests: &[Manifest]) -> Feed {
        let feed = Feed::default();
        feed.files
            .lock()
            .unwrap()
            .insert(FEED.into(), serde_json::to_vec(manifests).unwrap());
        feed
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-app-update-{label}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn versions_compare_by_number_and_a_pre_release_is_before_its_release() {
        use Ordering::{Equal, Greater, Less};
        assert_eq!(version_cmp("2026.10.0", "2026.9.1"), Greater);
        assert_eq!(
            version_cmp("2026.1.0-dev.99+aa", "2026.1.0-dev.100+bb"),
            Less
        );
        assert_eq!(version_cmp("2026.1.0-dev.5", "2026.1.0"), Less);
        assert_eq!(version_cmp("v2026.1.0", "2026.1.0+abc"), Equal);
        assert_eq!(version_cmp("2026.1.0-rc.1", "2026.1.0-dev.900"), Greater);
    }

    #[test]
    fn an_installer_is_matched_by_its_kind_and_its_architecture() {
        assert!(InstallKind::Deb.matches("atlas_2026.1.0_amd64.deb", "x86_64"));
        assert!(!InstallKind::Deb.matches("atlas_2026.1.0_arm64.deb", "x86_64"));
        assert!(InstallKind::Deb.matches("atlas_2026.1.0_arm64.deb", "aarch64"));
        assert!(!InstallKind::Rpm.matches("atlas_2026.1.0_amd64.deb", "x86_64"));
        assert!(InstallKind::Dmg.matches("Atlas_2026.1.0.dmg", "aarch64"));
        assert!(InstallKind::Nsis.matches("Atlas_2026.1.0_x64-setup.exe", "x86_64"));
        assert!(!InstallKind::Source.matches("atlas_2026.1.0_amd64.deb", "x86_64"));
    }

    #[tokio::test]
    async fn the_newest_signed_release_of_the_channel_for_this_install_is_offered() {
        let secret = throwaway(1);
        let keys = [public_key_for(&secret)];
        let sign = |body| sign_manifest(body, &secret).unwrap();
        let feed = feed_of(&[
            sign(release(
                "2026.2.0",
                "stable",
                "atlas_2026.2.0_amd64.deb",
                b"two",
            )),
            sign(release(
                "2026.3.0",
                "stable",
                "atlas_2026.3.0_amd64.deb",
                b"three",
            )),
            sign(release(
                "2026.4.0",
                "beta",
                "atlas_2026.4.0_amd64.deb",
                b"beta",
            )),
            sign(release(
                "2026.5.0",
                "stable",
                "atlas_2026.5.0_arm64.deb",
                b"arm",
            )),
            sign(release(
                "2026.6.0",
                "stable",
                "atlas-2026.6.0.x86_64.rpm",
                b"rpm",
            )),
        ]);
        let found = check_app_update(&feed, &keys, query("2026.1.0"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.version, "2026.3.0");
        assert_eq!(found.name, "atlas_2026.3.0_amd64.deb");
        // Nothing newer than what runs: no offer.
        assert_eq!(
            check_app_update(&feed, &keys, query("2026.3.0"))
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn a_release_no_trusted_key_signed_is_refused_and_no_older_one_is_offered_instead() {
        let secret = throwaway(2);
        let stranger = throwaway(3);
        let keys = [public_key_for(&secret)];
        let feed = feed_of(&[
            sign_manifest(
                release("2026.2.0", "stable", "atlas_2026.2.0_amd64.deb", b"two"),
                &secret,
            )
            .unwrap(),
            sign_manifest(
                release("2026.3.0", "stable", "atlas_2026.3.0_amd64.deb", b"three"),
                &stranger,
            )
            .unwrap(),
        ]);
        let error = check_app_update(&feed, &keys, query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, ReleaseError::BadSignature { .. }),
            "{error}"
        );
        // With no key at all, nothing is ever offered.
        let error = check_app_update(&feed, &[], query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, ReleaseError::BadSignature { .. }),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_changed_manifest_and_a_name_with_a_directory_are_refused() {
        let secret = throwaway(4);
        let keys = [public_key_for(&secret)];
        let mut changed = sign_manifest(
            release("2026.2.0", "stable", "atlas_2026.2.0_amd64.deb", b"two"),
            &secret,
        )
        .unwrap();
        changed.body.artifact.url = "https://elsewhere.invalid/atlas.deb".into();
        let error = check_app_update(&feed_of(&[changed]), &keys, query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, ReleaseError::BadSignature { .. }),
            "{error}"
        );

        let escaping = sign_manifest(
            release("2026.2.0", "stable", "../atlas_2026.2.0_amd64.deb", b"two"),
            &secret,
        )
        .unwrap();
        let error = check_app_update(&feed_of(&[escaping]), &keys, query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(matches!(error, ReleaseError::InvalidManifest(_)), "{error}");
    }

    #[tokio::test]
    async fn a_feed_that_is_not_there_or_is_not_a_feed_says_so() {
        let error = check_app_update(&Feed::default(), &[], query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("answered 404"), "{error}");
        let feed = Feed::default();
        feed.files
            .lock()
            .unwrap()
            .insert(FEED.into(), b"<html>".to_vec());
        let error = check_app_update(&feed, &[], query("2026.1.0"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not a release feed"), "{error}");
    }

    #[tokio::test]
    async fn an_installer_is_downloaded_resumed_and_checked_and_a_changed_one_is_deleted() {
        let secret = throwaway(5);
        let keys = [public_key_for(&secret)];
        let contents = b"the installer's bytes, all of them".to_vec();
        let name = "atlas_2026.2.0_amd64.deb";
        let feed =
            feed_of(&[
                sign_manifest(release("2026.2.0", "stable", name, &contents), &secret).unwrap(),
            ]);
        let update = check_app_update(&feed, &keys, query("2026.1.0"))
            .await
            .unwrap()
            .unwrap();
        feed.files
            .lock()
            .unwrap()
            .insert(update.url.clone(), contents.clone());

        // A partial file from an earlier run: the download asks for the rest only.
        let dir = temp_dir("resume");
        std::fs::write(dir.join(format!("{name}.partial")), &contents[..10]).unwrap();
        let seen = Mutex::new(Vec::new());
        let installer = download_app_update(&feed, &update, &dir, &|p| {
            seen.lock().unwrap().push(p.downloaded);
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&installer.path).unwrap(), contents);
        assert_eq!(feed.offsets.lock().unwrap().last(), Some(&10));
        assert_eq!(seen.lock().unwrap().last(), Some(&(contents.len() as u64)));
        // Asked again, the checked file is used and nothing is fetched.
        let asked = feed.offsets.lock().unwrap().len();
        download_app_update(&feed, &update, &dir, &|_| {})
            .await
            .unwrap();
        assert_eq!(feed.offsets.lock().unwrap().len(), asked);

        // The server now holds other bytes of the same length: refused, and no file is left.
        let dir = temp_dir("changed");
        let mut other = contents.clone();
        other[0] ^= 1;
        feed.files.lock().unwrap().insert(update.url.clone(), other);
        let error = download_app_update(&feed, &update, &dir, &|_| {})
            .await
            .unwrap_err();
        assert!(
            matches!(error, ReleaseError::HashMismatch { .. }),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }
}
