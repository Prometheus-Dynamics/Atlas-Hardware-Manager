//! Tests for the release catalog. No network: downloads go through a scripted
//! fake, and retries run on tokio's paused clock.

use std::collections::VecDeque;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::*;
use crate::fetch::{Body, Reply};
use crate::hash::to_hex;
use crate::manifest::public_key_for;
use crate::manifest::tests::{body, throwaway};
use crate::sign_manifest;
use crate::trust::fixture;
use crate::trust::tests::fixture_ring;

/// One scripted reply: a status, and the body's pieces (an error is a dropped
/// connection).
type Scripted = (u16, Vec<Result<Vec<u8>, String>>);

/// A fetcher that answers from a script and records each requested offset.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Scripted>>,
    offsets: Mutex<Vec<u64>>,
}

impl Script {
    fn new(replies: Vec<Scripted>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            offsets: Mutex::new(Vec::new()),
        })
    }

    fn offsets(&self) -> Vec<u64> {
        self.offsets.lock().unwrap().clone()
    }
}

struct Pieces(VecDeque<Result<Vec<u8>, String>>);

#[async_trait::async_trait]
impl Body for Pieces {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self.0.pop_front() {
            Some(Ok(piece)) => Ok(Some(piece)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }
}

#[async_trait::async_trait]
impl Fetch for Script {
    async fn get(&self, _url: &str, offset: u64) -> Result<Reply, String> {
        self.offsets.lock().unwrap().push(offset);
        let (status, pieces) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or("the script has no more replies")?;
        Ok(Reply {
            status,
            body: Box::new(Pieces(pieces.into())),
        })
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atlas-release-{label}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(public_keys: Vec<String>) -> RemoteSource {
    RemoteSource {
        name: "example".into(),
        index_url: "https://example.invalid/index.json".into(),
        public_keys,
    }
}

fn open(dir: &Path, fetch: Arc<dyn Fetch>, keyring: Keyring, security: Security) -> ReleaseCatalog {
    ReleaseCatalog::open(
        dir.join("catalog.json"),
        dir.join("cache"),
        fetch,
        keyring,
        security,
    )
    .unwrap()
}

fn no_network() -> Arc<dyn Fetch> {
    Script::new(Vec::new())
}

/// A remote entry for a release of `contents`, listed as signed or not.
fn remote_entry(id: &str, url: &str, contents: &[u8], signed: bool) -> ReleaseEntry {
    ReleaseEntry {
        id: id.into(),
        family: Family::new("example-family"),
        version: "2026.3.4".into(),
        channel: Channel::Stable,
        origin: ReleaseOrigin::Remote {
            source: "example".into(),
            url: url.into(),
        },
        artifact_name: "example.img.xz".into(),
        sha256: to_hex(&Sha256::digest(contents)),
        size_bytes: contents.len() as u64,
        path: None,
        signed,
        signature: None,
        boards: Vec::new(),
        notes_url: None,
        added_ms: 0,
        pinned: false,
    }
}

#[tokio::test]
async fn local_files_are_hashed_and_rechecked_before_use() {
    let dir = temp_dir("local");
    let image = dir.join("example.img.xz");
    std::fs::write(&image, b"abc").unwrap();
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);

    let entry = catalog
        .add_local_file(&image, Family::new("example-family"), "dev".into())
        .await
        .unwrap();
    assert!(!entry.signed);
    assert_eq!(catalog.artifact(&entry.id).await.unwrap().size_bytes, 3);

    // A tampered file is refused, even with security off: its hash is known.
    std::fs::write(&image, b"tampered").unwrap();
    assert!(matches!(
        catalog.artifact(&entry.id).await,
        Err(ReleaseError::HashMismatch { .. } | ReleaseError::SizeMismatch { .. })
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn only_first_flash_images_are_added() {
    let dir = temp_dir("kind");
    let file = dir.join("example.zip");
    std::fs::write(&file, b"abc").unwrap();
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);
    let error = catalog
        .add_local_file(&file, Family::new("example-family"), "1".into())
        .await
        .unwrap_err();
    assert!(matches!(error, ReleaseError::NotAnImage(_)));
    assert!(catalog.entries().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unsigned_manifests_are_listed_but_marked() {
    let dir = temp_dir("manifests");
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);
    let secret = throwaway(0);
    let good = sign_manifest(body("2026.3.1"), &secret).unwrap();
    let forged = sign_manifest(body("2026.9.9"), &throwaway(1)).unwrap();
    let keys = vec![public_key_for(&secret).to_string()];

    let warnings = catalog.add_manifests(&source(keys), vec![good, forged]);

    assert_eq!(warnings.len(), 1, "the forged one is listed with a warning");
    let entries = catalog.entries();
    assert_eq!(entries.len(), 2);
    let good = entries.iter().find(|e| e.version == "2026.3.1").unwrap();
    let forged = entries.iter().find(|e| e.version == "2026.9.9").unwrap();
    assert!(good.signed);
    assert!(!forged.signed);
    assert_eq!(good.channel, Channel::Stable);
    assert!(matches!(
        catalog.artifact(&good.id).await,
        Err(ReleaseError::NotDownloaded(_))
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn security_on_refuses_unsigned_manifests() {
    let dir = temp_dir("manifests-on");
    let catalog = open(&dir, no_network(), Keyring::default(), Security::On);
    let secret = throwaway(0);
    let good = sign_manifest(body("2026.3.1"), &secret).unwrap();
    let forged = sign_manifest(body("2026.9.9"), &throwaway(1)).unwrap();
    let keys = vec![public_key_for(&secret).to_string()];

    let warnings = catalog.add_manifests(&source(keys), vec![good, forged]);

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("refused"));
    let entries = catalog.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].version, "2026.3.1");
    assert!(entries[0].signed);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sources_with_bad_keys_are_refused() {
    let dir = temp_dir("sources");
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);
    let key = public_key_for(&throwaway(0)).to_string();

    assert!(
        catalog
            .set_source(source(vec!["ed25519:00".into()]))
            .is_err()
    );
    assert!(catalog.set_source(source(vec![key])).is_ok());
    assert_eq!(catalog.sources().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cache_file_names_are_sanitized() {
    assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
    assert_eq!(safe_file_name("example 1.img.xz"), "example_1.img.xz");
    assert_eq!(safe_file_name(".."), "artifact");
}

async fn add(catalog: &ReleaseCatalog, dir: &Path, name: &str, body: &str) -> ReleaseEntry {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    catalog
        .add_local_file(&path, Family::new("example-family"), name.into())
        .await
        .unwrap()
}

#[tokio::test]
async fn local_images_dont_pile_up() {
    let dir = temp_dir("pileup");
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);
    let locals = |catalog: &ReleaseCatalog| {
        catalog
            .entries()
            .into_iter()
            .filter(|entry| entry.channel == Channel::Local)
            .count()
    };

    // The same file twice is one entry.
    add(&catalog, &dir, "a.img.xz", "a").await;
    add(&catalog, &dir, "a.img.xz", "a").await;
    assert_eq!(locals(&catalog), 1);

    // A rebuilt file at the same path replaces its entry.
    let rebuilt = add(&catalog, &dir, "a.img.xz", "a2").await;
    assert_eq!(locals(&catalog), 1);
    assert!(catalog.entry(&rebuilt.id).is_some());

    // A pinned image survives pruning; only KEEP_LOCAL unpinned are kept.
    catalog.set_pinned(&rebuilt.id, true).unwrap();
    for i in 0..(KEEP_LOCAL + 3) {
        add(&catalog, &dir, &format!("b{i}.img.xz"), &format!("b{i}")).await;
    }
    assert_eq!(locals(&catalog), KEEP_LOCAL + 1);
    assert!(catalog.entry(&rebuilt.id).unwrap().pinned);
    // The oldest unpinned ones went; the newest stayed.
    let names: Vec<String> = catalog
        .entries()
        .into_iter()
        .map(|entry| entry.artifact_name)
        .collect();
    assert!(names.contains(&format!("b{}.img.xz", KEEP_LOCAL + 2)));
    assert!(!names.contains(&"b0.img.xz".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

const BODY: &[u8] = b"0123456789";
const URL: &str = "https://example.invalid/example.img.xz";

#[tokio::test(start_paused = true)]
async fn a_download_survives_a_500_and_a_dropped_connection() {
    let dir = temp_dir("flaky");
    let script = Script::new(vec![
        (500, Vec::new()),
        (
            200,
            vec![Ok(BODY[..4].to_vec()), Err("connection reset".into())],
        ),
        (206, vec![Ok(BODY[4..].to_vec())]),
    ]);
    let catalog = open(&dir, script.clone(), Keyring::default(), Security::Off);
    catalog.upsert(remote_entry("pv", URL, BODY, true)).unwrap();

    let entry = catalog.download("pv", |_| {}).await.unwrap();

    assert_eq!(std::fs::read(entry.path.unwrap()).unwrap(), BODY);
    assert_eq!(script.offsets(), [0, 0, 4]);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(start_paused = true)]
async fn a_download_resumes_from_a_partial_file_after_a_restart() {
    let dir = temp_dir("resume");
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let catalog = open(&dir, no_network(), Keyring::default(), Security::Off);
    let entry = remote_entry("pv", URL, BODY, true);
    catalog.upsert(entry.clone()).unwrap();
    // What an earlier run left behind: the first four bytes.
    let partial = cache.join(format!("{}.partial", cache_name(&entry)));
    std::fs::write(&partial, &BODY[..4]).unwrap();

    let script = Script::new(vec![(206, vec![Ok(BODY[4..].to_vec())])]);
    let catalog = open(&dir, script.clone(), Keyring::default(), Security::Off);
    let done = catalog.download("pv", |_| {}).await.unwrap();

    assert_eq!(script.offsets(), [4]);
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), BODY);
    assert!(!partial.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(start_paused = true)]
async fn a_tampered_download_is_refused_and_leaves_no_file_under_its_name() {
    let dir = temp_dir("tampered");
    let script = Script::new(vec![(200, vec![Ok(b"XXXXXXXXXX".to_vec())])]);
    let catalog = open(&dir, script, Keyring::default(), Security::Off);
    catalog.upsert(remote_entry("pv", URL, BODY, true)).unwrap();

    let error = catalog.download("pv", |_| {}).await.unwrap_err();

    assert!(
        matches!(error, ReleaseError::HashMismatch { .. }),
        "{error}"
    );
    let entry = catalog.entry("pv").unwrap();
    assert!(entry.path.is_none());
    let target = catalog.cached_target(&entry);
    assert!(!target.exists());
    assert!(!PathBuf::from(format!("{}.partial", target.display())).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(start_paused = true)]
async fn a_body_longer_than_the_release_says_is_refused() {
    let dir = temp_dir("long");
    let script = Script::new(vec![(200, vec![Ok(b"0123456789!".to_vec())])]);
    let catalog = open(&dir, script, Keyring::default(), Security::Off);
    catalog.upsert(remote_entry("pv", URL, BODY, true)).unwrap();

    let error = catalog.download("pv", |_| {}).await.unwrap_err();

    assert!(
        matches!(error, ReleaseError::SizeMismatch { .. }),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(start_paused = true)]
async fn a_client_error_is_not_retried() {
    let dir = temp_dir("missing");
    let script = Script::new(vec![(404, Vec::new())]);
    let catalog = open(&dir, script.clone(), Keyring::default(), Security::Off);
    catalog.upsert(remote_entry("pv", URL, BODY, true)).unwrap();

    let error = catalog.download("pv", |_| {}).await.unwrap_err();

    assert!(matches!(error, ReleaseError::Download { .. }), "{error}");
    assert_eq!(script.offsets(), [0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test(start_paused = true)]
async fn security_on_does_not_download_an_unsigned_release() {
    let dir = temp_dir("unsigned-download");
    let script = Script::new(Vec::new());
    let catalog = open(&dir, script.clone(), Keyring::default(), Security::On);
    catalog
        .upsert(remote_entry("pv", URL, BODY, false))
        .unwrap();

    let error = catalog.download("pv", |_| {}).await.unwrap_err();

    assert!(matches!(error, ReleaseError::Untrusted { .. }), "{error}");
    assert!(script.offsets().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_local_file_carries_what_its_signature_says() {
    let dir = temp_dir("signature");
    let catalog = open(&dir, no_network(), Keyring::builtin(), Security::Off);
    let unsigned = add(&catalog, &dir, "plain.img.xz", "plain").await;
    assert!(!unsigned.signed);
    assert_eq!(unsigned.signature, Some(SignatureStatus::Unsigned));

    // Signed, but by the fixtures' throwaway key, which Atlas doesn't trust.
    // Off: it is listed and reported.
    for name in ["image.img", "image.img.minisig"] {
        std::fs::copy(fixture(name), dir.join(name)).unwrap();
    }
    std::fs::rename(dir.join("image.img"), dir.join("image.img.xz")).unwrap();
    std::fs::rename(
        dir.join("image.img.minisig"),
        dir.join("image.img.xz.minisig"),
    )
    .unwrap();
    let entry = catalog
        .add_local_file(
            &dir.join("image.img.xz"),
            Family::new("example-family"),
            "1".into(),
        )
        .await
        .unwrap();
    assert!(!entry.signed);
    assert_eq!(
        entry.signature,
        Some(SignatureStatus::UnknownKey {
            key_id: "8E128666038FC1F5".into()
        })
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn security_on_refuses_a_tampered_image_and_an_untrusted_signature() {
    let dir = temp_dir("tamper");
    let image = dir.join("image.img.xz");
    std::fs::copy(fixture("image.img"), &image).unwrap();
    std::fs::copy(
        fixture("image.img.minisig"),
        dir.join("image.img.xz.minisig"),
    )
    .unwrap();

    // Signed by a trusted, unexpired key: added and usable.
    let trusted = open(
        &dir,
        no_network(),
        fixture_ring("production", "2099-01-01"),
        Security::On,
    );
    let entry = trusted
        .add_local_file(&image, Family::new("example-family"), "1".into())
        .await
        .unwrap();
    assert!(entry.signed);
    assert!(trusted.artifact(&entry.id).await.is_ok());

    // The same file is refused when the key is not trusted.
    let untrusted = open(&dir, no_network(), Keyring::builtin(), Security::On);
    let refused = untrusted
        .add_local_file(&image, Family::new("example-family"), "1".into())
        .await;
    assert!(matches!(refused, Err(ReleaseError::Untrusted { .. })));

    // A tampered image is refused at use: its hash no longer matches.
    std::fs::write(&image, b"an image, tampered with after signing\n").unwrap();
    let error = trusted.artifact(&entry.id).await.unwrap_err();
    assert!(
        matches!(
            error,
            ReleaseError::HashMismatch { .. } | ReleaseError::SizeMismatch { .. }
        ),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
