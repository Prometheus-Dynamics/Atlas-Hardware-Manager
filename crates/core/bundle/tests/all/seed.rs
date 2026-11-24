//! The signing seed: the key file's two forms, the owner-only check, the new-file writer, the key id
//! check before signing, and the record byte for byte against Lemnos's documented vector.
//!
//! No real key is used. Each test makes its own seed from a fixed pattern or the published vector
//! seed (0x07 repeated), in a directory under `CARGO_TARGET_TMPDIR`, which lives in the build's
//! target directory and not in `/tmp`.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_bundle::{BundleError, SEED_LEN, Seed, sign_slot_record, slot_message};
use atlas_release::{PublicKey, public_key_for};

/// The published vector: Ed25519 seed 0x07 x32 (Lemnos `docs/slots.md`, "The signed message").
const VECTOR_SEED: [u8; SEED_LEN] = [0x07; SEED_LEN];
const VECTOR_JSON: &[u8] = br#"{"id":"vector"}"#;
const VECTOR_PUBLIC: &str = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
const VECTOR_KEY_ID: &str = "fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889";
/// The 42-byte signed message for `{"id":"vector"}`.
const VECTOR_MESSAGE: &str =
    "6c656d6e6f732d736c6f742d62756e646c6500010000000f0000007b226964223a22766563746f72227d";
/// The 108-byte record: `LMNSSIG1`, algorithm 1, three zero bytes, the key id, the signature.
const VECTOR_RECORD: &str = concat!(
    "4c4d4e535349473101000000",
    "fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889",
    "56be579db020703eb207da579036a89f0321fcd8ce2bac78a9aa939a58217e265ad27a7e1fc05829b0db05d37ac4d8dba35263c7a1b2644322b45ada4a101402",
);

/// A recognisable seed for the refusal and Debug tests: no test output could contain it by chance.
const PATTERN: [u8; SEED_LEN] = [
    0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef,
    0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef, 0xde, 0xad, 0xbe, 0xef,
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("the vector is hex"))
        .collect()
}

/// A fresh directory for one test, removed when the returned value is dropped.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("seed-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("a directory under the target's temporary directory")
}

/// A file with `content`, owner-only where the platform has modes.
fn file_with(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).expect("writes the test file");
    set_mode(&path, 0o600);
    path
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("sets the mode");
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// Whether `text` holds any run of 8 bytes that `secret` holds: a refusal that quoted the file's
/// content would hold one.
fn holds_a_run_of(text: &str, secret: &[u8]) -> bool {
    secret
        .windows(8)
        .any(|run| text.as_bytes().windows(8).any(|seen| seen == run))
}

// Proof 1: the vector, byte for byte.
#[test]
fn the_vector_is_reproduced_byte_for_byte() {
    let seed = Seed::from_bytes(VECTOR_SEED);
    assert_eq!(seed.public_key(), public_key_for(&VECTOR_SEED));
    assert_eq!(
        seed.public_key().to_string(),
        format!("ed25519:{VECTOR_PUBLIC}")
    );
    assert_eq!(seed.key_id_hex(), VECTOR_KEY_ID);

    let message = slot_message(VECTOR_JSON).expect("the message");
    assert_eq!(hex(&message), VECTOR_MESSAGE, "the signed message");

    let record = sign_slot_record(&seed, VECTOR_JSON, None).expect("signs");
    assert_eq!(hex(&record), VECTOR_RECORD, "the 108-byte record");
    assert_eq!(record.len(), 108);

    // The record verifies under the vector's public key over the 42-byte message.
    let key: PublicKey = seed.public_key();
    assert!(key.verify_bytes(&unhex(VECTOR_MESSAGE), &record[44..108]));
    assert!(
        !key.verify_bytes(VECTOR_JSON, &record[44..108]),
        "the bare manifest does not verify"
    );
}

// Proof 2: the two accepted forms read; the wrong forms are refused, naming the path, never the content.
#[test]
fn seed_files_read_in_both_forms_and_wrong_forms_are_refused_without_content() {
    let dir = scratch();
    let text = hex(&PATTERN);
    let expected = public_key_for(&PATTERN);

    let good: [(&str, Vec<u8>); 5] = [
        ("raw.key", PATTERN.to_vec()),
        ("hex.key", text.clone().into_bytes()),
        ("hex-newline.key", format!("{text}\n").into_bytes()),
        ("hex-padded.key", format!("  {text}\r\n").into_bytes()),
        (
            "hex-upper.key",
            format!("{}\n", text.to_uppercase()).into_bytes(),
        ),
    ];
    for (name, content) in &good {
        let path = file_with(dir.path(), name, content);
        let seed = Seed::read(&path).unwrap_or_else(|error| panic!("{name} reads: {error}"));
        assert_eq!(seed.public_key(), expected, "{name} is the pattern's seed");
    }

    let marker = format!("{:-<64}", "marker");
    let long = vec![b'A'; 1000];
    let bad: [(&str, Vec<u8>); 7] = [
        ("31-raw.key", PATTERN[..31].to_vec()),
        ("33-raw.key", [&PATTERN[..], &[1u8][..]].concat()),
        ("63-hex.key", text.as_bytes()[..63].to_vec()),
        ("65-hex.key", format!("{text}0").into_bytes()),
        ("64-non-hex.key", marker.clone().into_bytes()),
        ("empty.key", Vec::new()),
        ("long.key", long),
    ];
    for (name, content) in &bad {
        let path = file_with(dir.path(), name, content);
        let error = Seed::read(&path).expect_err(&format!("{name} is refused"));
        let shown = error.to_string();
        assert!(matches!(error, BundleError::Seed(_)), "{name}: {shown}");
        assert!(
            shown.contains(name),
            "{name}: the refusal names the path: {shown}"
        );
        assert!(
            shown.contains("32 raw bytes") && shown.contains("64 hex"),
            "{name}: {shown}"
        );
        assert!(
            !shown.contains(&text),
            "{name}: the refusal holds no hex of the seed"
        );
        assert!(
            !holds_a_run_of(&shown, &PATTERN),
            "{name}: the refusal holds no seed bytes"
        );
        assert!(
            !shown.contains(&marker[..6]),
            "{name}: the refusal holds no file content"
        );
        assert!(
            !holds_a_run_of(&shown, content),
            "{name}: the refusal holds no run of the file"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_seed_file_readable_by_others_is_refused() {
    let dir = scratch();
    let path = file_with(dir.path(), "shared.key", &hex(&PATTERN).into_bytes());
    for mode in [0o644, 0o640, 0o604] {
        set_mode(&path, mode);
        let shown = Seed::read(&path).expect_err("refused").to_string();
        assert!(
            shown.contains(&format!("{mode:o}")),
            "names the mode: {shown}"
        );
        assert!(shown.contains("chmod 600"), "{shown}");
        assert!(!shown.contains(&hex(&PATTERN)), "{shown}");
    }
    set_mode(&path, 0o600);
    assert!(Seed::read(&path).is_ok(), "an owner-only file reads");
}

// Proof 3: write_new makes an owner-only file that reads back, and never overwrites.
#[test]
fn write_new_makes_an_owner_only_file_that_reads_back() {
    let dir = scratch();
    let path = dir.path().join("bench.key");
    let seed = Seed::from_bytes(PATTERN);
    seed.write_new(&path).expect("writes a new seed file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the file is owner-only");
    }
    let before = fs::read_to_string(&path).expect("reads the file");
    assert_eq!(
        before,
        format!("{}\n", hex(&PATTERN)),
        "64 hex characters and a newline"
    );

    let back = Seed::read(&path).expect("reads back");
    assert_eq!(back.key_id(), seed.key_id());
    assert_eq!(back.public_key(), seed.public_key());

    let error = seed
        .write_new(&path)
        .expect_err("an existing file is never overwritten");
    assert!(
        error.to_string().contains("refusing to overwrite"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("reads"),
        before,
        "the file is unchanged"
    );

    let missing = dir.path().join("no-such-directory").join("bench.key");
    let error = seed.write_new(&missing).expect_err("the parent must exist");
    assert!(error.to_string().contains("does not exist"), "{error}");
}

#[test]
fn generated_seeds_are_random_and_read_back() {
    let first = Seed::generate().expect("random bytes");
    let second = Seed::generate().expect("random bytes");
    assert_ne!(first.public_key(), second.public_key(), "two draws differ");
    let dir = scratch();
    let path = dir.path().join("generated.key");
    first.write_new(&path).expect("writes");
    assert_eq!(Seed::read(&path).expect("reads").key_id(), first.key_id());
}

// Proof 4: a stated key id is checked before anything is signed.
#[test]
fn a_stated_key_id_is_checked_before_anything_is_signed() {
    let seed = Seed::from_bytes(VECTOR_SEED);

    seed.check_key_id(VECTOR_KEY_ID)
        .expect("the right id passes");
    let record = sign_slot_record(&seed, VECTOR_JSON, Some(VECTOR_KEY_ID)).expect("right id signs");
    assert_eq!(hex(&record), VECTOR_RECORD);
    let padded = format!("  {}\n", VECTOR_KEY_ID.to_uppercase());
    assert_eq!(
        sign_slot_record(&seed, VECTOR_JSON, Some(&padded)).expect("upper case and padding pass"),
        record
    );

    let wrong = "00".repeat(32);
    match sign_slot_record(&seed, VECTOR_JSON, Some(&wrong)) {
        Err(BundleError::KeyIdMismatch { stated, derived }) => {
            assert_eq!(stated, wrong);
            assert_eq!(derived, VECTOR_KEY_ID);
        }
        other => panic!("a wrong id is refused naming both: {other:?}"),
    }

    // The seed itself stated as the id is refused, and the seed is not echoed.
    let seed_hex = hex(&VECTOR_SEED);
    let error =
        sign_slot_record(&seed, VECTOR_JSON, Some(&seed_hex)).expect_err("the seed is refused");
    assert!(matches!(error, BundleError::Seed(_)), "{error}");
    assert!(!error.to_string().contains(&seed_hex), "{error}");

    let error = sign_slot_record(&seed, VECTOR_JSON, Some("not a key id")).expect_err("not hex");
    assert!(matches!(error, BundleError::Seed(_)), "{error}");
}

// Proof 5: the Debug form holds no byte of the seed.
#[test]
fn the_debug_form_holds_no_seed_bytes() {
    let seed = Seed::from_bytes(PATTERN);
    let shown = format!("{seed:?}");
    assert_eq!(shown, "Seed(..)");
    assert!(!shown.contains(&hex(&PATTERN)));
    assert!(!holds_a_run_of(&shown, &PATTERN));
    assert!(!shown.contains("de"), "no hex pair of the pattern");
}
