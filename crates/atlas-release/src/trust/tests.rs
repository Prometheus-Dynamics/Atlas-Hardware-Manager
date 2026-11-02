//! The fixtures are signed by a throwaway key whose secret was deleted after
//! signing: fixtures/test.pub (8E128666038FC1F5), image.img and its
//! image.img.minisig (minisign 0.12, prehashed), and legacy.minisig (the
//! same file, signed the legacy way).

use super::*;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/trust/fixtures");

fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

fn ring(purpose: &str, expires: &str) -> Keyring {
    let public = std::fs::read_to_string(fixture("test.pub")).unwrap();
    let public = public.lines().nth(1).unwrap();
    let ring = Keyring::from_json(
        &format!(
            r#"{{"keys":[{{"name":"Test key","kind":"minisign","key_id":"8E128666038FC1F5",
            "public_key":"{public}","owner":"Test Co","contact":"https://example.com",
            "purpose":"{purpose}","created":"2026-11-02","expires":"{expires}"}}]}}"#
        ),
        "test",
    );
    assert!(ring.problems.is_empty(), "{:?}", ring.problems);
    ring
}

/// A copy of image.img (and, when given, a signature file) in a temp dir.
fn copy(contents: &[u8], signature: Option<&str>) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "atlas-trust-{}-{}",
        std::process::id(),
        now_ms() ^ contents.len() as u64 ^ signature.map_or(0, str::len) as u64
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("image.img");
    std::fs::write(&image, contents).unwrap();
    if let Some(name) = signature {
        std::fs::copy(fixture(name), signature_path(&image)).unwrap();
    }
    (dir, image)
}

#[test]
fn the_built_in_keys_all_parse() {
    let ring = Keyring::from_json(BUILTIN, "built in");
    assert!(ring.problems.is_empty(), "{:?}", ring.problems);
    let dev = ring
        .keys
        .iter()
        .find(|key| key.key_id == "1AB01E429EDCEAA6")
        .expect("the dev/example key");
    assert_eq!(dev.purpose, Purpose::Dev);
    assert_eq!(dev.fingerprint(), "minisign:1AB01E429EDCEAA6");
    assert_eq!(
        dev.who(),
        "Prometheus Dynamics dev/example key (Prometheus Dynamics, dev/example)"
    );
}

#[test]
fn a_file_signed_by_a_trusted_key_names_its_owner() {
    let status = check_file(&fixture("image.img"), &ring("production", "2099-01-01"));
    let SignatureStatus::Signed {
        key,
        comment,
        expired,
    } = &status
    else {
        panic!("{status:?}");
    };
    assert_eq!(key.owner, "Test Co");
    assert_eq!(comment, "file:image.img");
    assert!(!expired);
    assert!(status.trusted());
    assert_eq!(status.summary(), "signed by Test key (Test Co, production)");
    assert_eq!(status.warning(), None);
}

#[test]
fn a_dev_key_is_trusted_but_warned_about() {
    let status = check_file(&fixture("image.img"), &ring("dev", "2099-01-01"));
    assert!(status.trusted());
    assert!(status.warning().unwrap().contains("dev/example key"));
}

#[test]
fn an_expired_key_is_warned_about_and_not_trusted() {
    let status = check_file(&fixture("image.img"), &ring("production", "2026-01-01"));
    assert!(matches!(
        status,
        SignatureStatus::Signed { expired: true, .. }
    ));
    assert!(!status.trusted());
    assert!(status.warning().unwrap().contains("expired on 2026-01-01"));
}

#[test]
fn a_changed_file_has_a_bad_signature() {
    let (dir, image) = copy(
        b"an image, for the signature tests!\n",
        Some("image.img.minisig"),
    );
    let status = check_file(&image, &ring("production", "2099-01-01"));
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(
        status,
        SignatureStatus::Bad {
            reason: "the file doesn't match its signature".into()
        }
    );
    assert!(!status.trusted());
}

#[test]
fn unsigned_unknown_and_legacy_signatures_are_told_apart() {
    let original = std::fs::read(fixture("image.img")).unwrap();
    let (dir, image) = copy(&original, None);
    assert_eq!(
        check_file(&image, &ring("dev", "2099-01-01")),
        SignatureStatus::Unsigned
    );
    std::fs::remove_dir_all(dir).unwrap();

    let status = check_file(
        &fixture("image.img"),
        &Keyring::from_json(BUILTIN, "built in"),
    );
    assert_eq!(
        status,
        SignatureStatus::UnknownKey {
            key_id: "8E128666038FC1F5".into()
        }
    );
    assert!(
        status
            .warning()
            .unwrap()
            .contains("isn't in Atlas's trusted keys")
    );

    let (dir, image) = copy(&original, Some("legacy.minisig"));
    let status = check_file(&image, &ring("dev", "2099-01-01"));
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        matches!(&status, SignatureStatus::Bad { reason } if reason.contains("legacy")),
        "{status:?}"
    );
}

#[test]
fn a_key_whose_id_doesnt_match_its_public_key_is_left_out() {
    let ring = Keyring::from_json(
        r#"{"keys":[{"name":"Mislabelled","kind":"minisign","key_id":"0000000000000000",
        "public_key":"RWT1wY8DZoYSjg9r83y0LPK6jDHGeYxFSEuITjaqRzdsqQ0R0ZqG8HWy",
        "owner":"x","contact":"x","purpose":"dev","created":"2026-11-02","expires":null}]}"#,
        "test",
    );
    assert!(ring.keys.is_empty());
    assert!(ring.problems[0].contains("its id is 8E128666038FC1F5"));
}

#[test]
fn civil_dates_round_trip() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(20_392), (2025, 10, 31));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
}
