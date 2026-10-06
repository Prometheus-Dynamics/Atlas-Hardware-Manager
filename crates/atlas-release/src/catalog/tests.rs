//! Tests for the release catalog.

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
    // "Flash anyway" uses the file as it is now, with its real digest.
    let anyway = catalog.artifact_unchecked(&entry.id).await.unwrap();
    assert_eq!(anyway.size_bytes, 8);
    assert_ne!(anyway.sha256, entry.sha256);

    let reopened = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
    assert_eq!(reopened.entries().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn unsigned_manifests_are_listed_but_marked() {
    let dir = temp_dir("manifests");
    let catalog = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
    let good = sign_manifest(body("2026.3.1"), &SECRET).unwrap();
    let forged = sign_manifest(body("2026.9.9"), &[1u8; 32]).unwrap();

    let warnings = catalog.add_manifests(&source(), vec![good, forged]);

    assert_eq!(warnings.len(), 1, "the forged one is listed with a warning");
    let entries = catalog.entries();
    assert_eq!(entries.len(), 2);
    let good = entries.iter().find(|e| e.version == "2026.3.1").unwrap();
    let forged = entries.iter().find(|e| e.version == "2026.9.9").unwrap();
    assert!(good.signed);
    assert!(!forged.signed);
    let entries = [good.clone()];
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

async fn add(catalog: &ReleaseCatalog, dir: &Path, name: &str, body: &str) -> ReleaseEntry {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    catalog
        .add_local_file(&path, Family::new("rpi"), name.into())
        .await
        .unwrap()
}

#[tokio::test]
async fn local_images_dont_pile_up() {
    let dir = temp_dir("pileup");
    let catalog = ReleaseCatalog::open(dir.join("catalog.json"), dir.join("cache")).unwrap();
    let locals = |catalog: &ReleaseCatalog| {
        catalog
            .entries()
            .into_iter()
            .filter(|entry| entry.channel == Channel::Local)
            .count()
    };

    // The same file twice is one entry.
    add(&catalog, &dir, "a.img", "a").await;
    add(&catalog, &dir, "a.img", "a").await;
    assert_eq!(locals(&catalog), 1);

    // A rebuilt file at the same path replaces its entry.
    let rebuilt = add(&catalog, &dir, "a.img", "a2").await;
    assert_eq!(locals(&catalog), 1);
    assert!(catalog.entry(&rebuilt.id).is_some());

    // A pinned image survives pruning; only KEEP_LOCAL unpinned are kept.
    catalog.set_pinned(&rebuilt.id, true).unwrap();
    for i in 0..(KEEP_LOCAL + 3) {
        std::thread::sleep(std::time::Duration::from_millis(2));
        add(&catalog, &dir, &format!("b{i}.img"), &format!("b{i}")).await;
    }
    assert_eq!(locals(&catalog), KEEP_LOCAL + 1);
    assert!(catalog.entry(&rebuilt.id).unwrap().pinned);
    // The oldest unpinned ones went; the newest stayed.
    let names: Vec<String> = catalog
        .entries()
        .into_iter()
        .map(|entry| entry.artifact_name)
        .collect();
    assert!(names.contains(&format!("b{}.img", KEEP_LOCAL + 2)));
    assert!(!names.contains(&"b0.img".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_one_time_file_is_hashed_but_not_listed() {
    let dir = temp_dir("onetime");
    let path = dir.join("x.img");
    std::fs::write(&path, b"abc").unwrap();
    let artifact = local_artifact(&path).await.unwrap();
    assert_eq!(artifact.size_bytes, 3);
    assert_eq!(artifact.sha256.len(), 64);
    let _ = std::fs::remove_dir_all(dir);
}
