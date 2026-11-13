//! The signed message, the signature record, and the two-stream file: the test
//! vector of the W1 brief (draft 2, public test key only), a throwaway key signing a
//! generated image, the layout refusals, and every reader decoding stream 1 only.
//!
//! Proof status: the Lemnos board code does not implement draft 2 yet, so these
//! tests and the vector are the proof on this side; the board side is pending.

mod common;

use atlas_image::{
    Algorithm, BoardOutcome, FlashError, Format, ImageKind, Options, Record, SignatureState,
    flash_manifest, open, probe_board, signature, signed_message,
};
use common::*;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

// The test vector (W1-actions.md, draft 2). The seed is seven bytes of 0x07 four
// times over; it is public test data, not a key anything signs with.
const VECTOR_MESSAGE: &str = "6c656d6e6f732d66697273742d666c61736800010000000a000000666c6173682d74657374040000006469736b0040000000000000f30a82fe97de9b784dd7b5735958b28dbfa95f94c0ba6d4834b441ab78b65c58";
const VECTOR_PUBLIC_KEY: &str = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
const VECTOR_SIGNATURE: &str = "bd894b37df1a05915c0c6baa7606d57c8cc2a6aa10a01451a3e4f67b8edfd517df6794fbcdeeaadfd7f46e099cea00be896e99445a8942f5e7592ec31c1b6c09";
const VECTOR_KEY_ID: &str = "fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889";
const VECTOR_RECORD: &str = "4c4d4e535349473101000000fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889bd894b37df1a05915c0c6baa7606d57c8cc2a6aa10a01451a3e4f67b8edfd517df6794fbcdeeaadfd7f46e099cea00be896e99445a8942f5e7592ec31c1b6c09";
const VECTOR_SHA: &str = "f30a82fe97de9b784dd7b5735958b28dbfa95f94c0ba6d4834b441ab78b65c58";
const VECTOR_SIZE: u64 = 16384;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_array(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A throwaway key: a fixed seed for the test only, never a secret.
fn throwaway_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42; 32])
}

/// The record stream of `record`, as the W1 signer makes it: `xz -0 --check=crc64`.
fn record_stream(record: &[u8]) -> Vec<u8> {
    xz_with(record, &["-0", "--check=crc64"])
}

#[test]
fn the_message_builder_reproduces_the_vector_and_its_signature_verifies() {
    let sha: [u8; 32] = unhex(VECTOR_SHA).try_into().expect("32 bytes");
    let message = signed_message("flash-test", ImageKind::Disk, VECTOR_SIZE, &sha);
    assert_eq!(hex(&message), VECTOR_MESSAGE, "the message bytes");
    assert_eq!(message.len(), 85);

    let public: [u8; 32] = unhex(VECTOR_PUBLIC_KEY).try_into().expect("32 bytes");
    let key = VerifyingKey::from_bytes(&public).expect("public key");
    let signature = Signature::from_slice(&unhex(VECTOR_SIGNATURE)).expect("signature");
    key.verify_strict(&message, &signature)
        .expect("the vector's signature verifies over the rebuilt message");

    // The key id is SHA-256 of the public key.
    assert_eq!(hex(&sha_array(&public)), VECTOR_KEY_ID);
}

#[test]
fn the_record_parses_and_writes_the_vector_bytes() {
    let bytes = unhex(VECTOR_RECORD);
    assert_eq!(bytes.len(), 108);
    let record = Record::parse(&bytes).expect("the vector record parses");
    assert_eq!(record.key_id_hex(), VECTOR_KEY_ID);
    assert_eq!(hex(&record.signature), VECTOR_SIGNATURE);
    assert_eq!(hex(&record.to_bytes()), VECTOR_RECORD, "the writer reproduces the bytes");
}

#[test]
fn malformed_records_are_refused_with_their_own_messages() {
    let good = unhex(VECTOR_RECORD);

    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    let error = Record::parse(&bad_magic).expect_err("magic");
    assert!(matches!(error, FlashError::Record(_)), "{error}");
    assert!(error.to_string().starts_with("malformed signature record"), "{error}");

    let mut unknown_algorithm = good.clone();
    unknown_algorithm[8] = 2;
    let error = Record::parse(&unknown_algorithm).expect_err("algorithm");
    assert!(error.to_string().contains("algorithm 2"), "{error}");

    let mut padding = good.clone();
    padding[10] = 1;
    let error = Record::parse(&padding).expect_err("padding");
    assert!(error.to_string().contains("not zero"), "{error}");

    let error = Record::parse(&good[..107]).expect_err("length");
    assert!(matches!(error, FlashError::NotFirstFlash(_)), "{error}");
    assert!(error.to_string().starts_with("not a first-flash image"), "{error}");
}

#[test]
fn the_board_outcomes_are_the_four_wire_values_and_anything_else_is_unrecognised() {
    for (wire, outcome, code) in [
        ("verified", BoardOutcome::Verified, Some(0)),
        ("not_checked_unsigned", BoardOutcome::NotCheckedUnsigned, Some(1)),
        ("not_checked_unknown_key", BoardOutcome::NotCheckedUnknownKey, Some(2)),
        ("refused", BoardOutcome::Refused, None),
    ] {
        assert_eq!(BoardOutcome::parse(wire), outcome, "{wire}");
        assert_eq!(outcome.as_wire(), wire);
        assert_eq!(outcome.code(), code, "{wire}");
    }
    assert_eq!(BoardOutcome::Verified.reason(), Some("signature verified with a key on this board's list"));
    assert_eq!(BoardOutcome::Refused.reason(), None);
    // A value outside the four is never mapped to one of them.
    let other = BoardOutcome::parse("verified_with_warning");
    assert_eq!(other, BoardOutcome::Unrecognised("verified_with_warning".into()));
    assert_eq!(other.as_wire(), "verified_with_warning");
    assert_eq!(other.code(), None);
    assert_eq!(BoardOutcome::parse(""), BoardOutcome::Unrecognised(String::new()));
    assert!(ImageKind::parse("xz").is_err(), "a compression name is never a kind");
    assert_eq!(ImageKind::parse("disk").expect("disk"), ImageKind::Disk);
}

#[test]
fn a_signed_two_stream_image_is_read_and_its_record_round_trips() {
    let dir = TestDir::new("signed");
    let raw = image_disk();
    let sha = sha_array(&raw);
    let message = signed_message("example-board", ImageKind::Disk, raw.len() as u64, &sha);
    let key = throwaway_key();
    let sig: Signature = key.sign(&message);
    let record = Record {
        algorithm: Algorithm::Ed25519,
        key_id: sha_array(key.verifying_key().as_bytes()),
        signature: sig.to_bytes(),
    };
    let mut file = xz(&raw);
    file.extend_from_slice(&record_stream(&record.to_bytes()));
    let path = dir.write("signed.img.xz", &file);

    // The file's signature state, without decoding the image.
    let state = signature(&path).expect("signature");
    match &state {
        SignatureState::Signed { key_id, record: found } => {
            assert_eq!(found, &record, "the record round-trips");
            assert_eq!(hex(key_id), hex(&record.key_id));
        }
        SignatureState::Unsigned => panic!("a signed file reads as unsigned"),
    }

    // The manifest covers stream 1 only, and the board's rebuilt message verifies.
    let made = flash_manifest(&path, dir.path(), None).expect("manifest");
    assert_eq!(made.manifest.image_size(), raw.len() as u64);
    assert_eq!(made.manifest.image_sha256(), hex(&sha));
    assert_eq!(made.manifest.kind(), ImageKind::Disk);
    let rebuilt = signed_message("example-board", made.manifest.kind(), made.manifest.image_size(), &sha);
    key.verifying_key()
        .verify_strict(&rebuilt, &sig)
        .expect("the record's signature verifies over the rebuilt message");
    let json = String::from_utf8(made.json).expect("utf8");
    assert!(json.starts_with("{\"schema\":1,\"board\":\"example-board\",\"format\":\"disk\","), "{json}");
    assert!(json.ends_with(&format!("\"signature\":\"{}\"}}", hex(&record.to_bytes()))), "{json}");
    assert!(dir.entries().iter().all(|name| !name.starts_with("atlas-image-")));
}

#[test]
fn the_two_stream_file_gives_the_single_stream_manifest_and_the_slot_reader_stops_at_stream_one() {
    let dir = TestDir::new("two-stream");
    let raw = image_disk();
    let single = dir.write("single.img.xz", &xz(&raw));
    let mut both = xz(&raw);
    both.extend_from_slice(&record_stream(&unhex(VECTOR_RECORD)));
    let double = dir.write("double.img.xz", &both);

    let one = flash_manifest(&single, dir.path(), None).expect("single");
    let two = flash_manifest(&double, dir.path(), None).expect("double");
    assert_eq!(one.manifest, two.manifest, "size and hash cover stream 1 only");
    assert_eq!(one.signature, SignatureState::Unsigned);
    assert!(matches!(two.signature, SignatureState::Signed { .. }));

    let probed = probe_board(&double, dir.path()).expect("probe");
    assert_eq!(probed.board, "example-board");
    assert_eq!(probed.format, Format::Xz);

    // The slot reader opens the two-stream file and reads the slot from stream 1.
    let image = open(&double, dir.path(), &["alpha"], &Options::default()).expect("open");
    assert_eq!(image.slot("alpha").expect("slot").image_size(), 20480);
    drop(image);
    assert!(dir.entries().iter().all(|name| !name.starts_with("atlas-image-")));
}

#[test]
fn a_sha256_check_is_accepted_and_a_bigger_dictionary_is_refused_by_name() {
    let dir = TestDir::new("checks");
    let raw = image_disk();
    let sha_file = dir.write("sha.img.xz", &xz_with(&raw, &["--check=sha256"]));
    let made = flash_manifest(&sha_file, dir.path(), None).expect("SHA-256 check accepted");
    assert_eq!(made.manifest.image_sha256(), hex(&sha256(&raw)));

    let big: Vec<u8> = (0..(17u64 << 20)).map(|i| (i % 251) as u8).collect();
    let seven = dir.write("seven.img.xz", &xz_with(&big, &["-7", "--check=crc64"]));
    let error = flash_manifest(&seven, dir.path(), None).expect_err("16 MiB dictionary");
    let text = error.to_string();
    assert!(text.contains("dictionary") && text.contains("8 MiB"), "{text}");
    assert!(text.contains("bytes"), "the size found is named: {text}");
}

#[test]
fn the_layout_refusals_are_each_by_name() {
    let dir = TestDir::new("refusals");
    let raw = image_disk();
    let record = unhex(VECTOR_RECORD);

    // A third stream.
    let mut third = xz(&raw);
    third.extend_from_slice(&record_stream(&record));
    third.extend_from_slice(&record_stream(&record));
    let path = dir.write("third.img.xz", &third);
    let error = flash_manifest(&path, dir.path(), None).expect_err("third stream");
    assert!(error.to_string().starts_with("not a first-flash image"), "{error}");

    // A junk second stream, big: not a record, refused by name.
    let mut junk = xz(&raw);
    junk.extend_from_slice(&xz(&noise(4096, 3)));
    let path = dir.write("junk.img.xz", &junk);
    let error = flash_manifest(&path, dir.path(), None).expect_err("junk stream");
    assert!(error.to_string().starts_with("not a first-flash image"), "{error}");

    // A small second stream of the wrong length.
    let mut short = xz(&raw);
    short.extend_from_slice(&record_stream(&record[..50]));
    let path = dir.write("short.img.xz", &short);
    let error = flash_manifest(&path, dir.path(), None).expect_err("short record");
    assert!(error.to_string().contains("not one 108-byte"), "{error}");

    // A record of the right length with the wrong magic.
    let mut magic = record.clone();
    magic[0] = b'X';
    let mut wrong = xz(&raw);
    wrong.extend_from_slice(&record_stream(&magic));
    let path = dir.write("magic.img.xz", &wrong);
    let error = flash_manifest(&path, dir.path(), None).expect_err("magic");
    assert!(error.to_string().starts_with("malformed signature record"), "{error}");

    // Trailing bytes after the record stream.
    let mut trailing = xz(&raw);
    trailing.extend_from_slice(&record_stream(&record));
    trailing.extend_from_slice(b"junk");
    let path = dir.write("trailing.img.xz", &trailing);
    assert!(flash_manifest(&path, dir.path(), None).is_err(), "trailing bytes");

    // Non-zero bytes between the streams.
    let mut gap = xz(&raw);
    gap.extend_from_slice(&[1, 0, 0, 0]);
    gap.extend_from_slice(&record_stream(&record));
    let path = dir.write("gap.img.xz", &gap);
    assert!(flash_manifest(&path, dir.path(), None).is_err(), "non-zero padding");

    // Zero padding in a multiple of four between the streams is the xz format's own.
    let mut padded = xz(&raw);
    padded.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
    padded.extend_from_slice(&record_stream(&record));
    let path = dir.write("padded.img.xz", &padded);
    let made = flash_manifest(&path, dir.path(), None).expect("zero padding accepted");
    assert!(matches!(made.signature, SignatureState::Signed { .. }));

    assert!(dir.entries().iter().all(|name| !name.starts_with("atlas-image-")));
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[test]
fn a_plain_image_is_unsigned_and_a_sidecar_changes_nothing() {
    let dir = TestDir::new("plain");
    let raw = image_disk();
    let path = dir.write("image.img", &raw);
    assert_eq!(signature(&path).expect("state"), SignatureState::Unsigned);
    let made = flash_manifest(&path, dir.path(), None).expect("plain");
    assert_eq!(made.manifest.image_sha256(), hex(&sha256(&raw)));
    assert!(matches!(made.signature, SignatureState::Unsigned));
}

#[test]
fn gaias_builder_command_is_accepted_and_preset_seven_is_refused_naming_the_dictionary() {
    // The builder line the coordinator gave: `xz -6 --check=crc64 -T1` (8 MiB dictionary).
    let dir = TestDir::new("gaia-preset");
    let raw = image_disk();
    let six = dir.write("six.img.xz", &xz_with(&raw, &["-6", "--check=crc64", "-T1"]));
    let made = flash_manifest(&six, dir.path(), None).expect("-6 -T1 is accepted");
    assert_eq!(made.manifest.image_sha256(), hex(&sha256(&raw)));

    let big: Vec<u8> = (0..(17u64 << 20)).map(|i| (i % 251) as u8).collect();
    let seven = dir.write("seven.img.xz", &xz_with(&big, &["-7", "--check=crc64", "-T1"]));
    let error = flash_manifest(&seven, dir.path(), None).expect_err("-7 is refused");
    let text = error.to_string();
    assert!(text.contains("dictionary is 16777216 bytes (16 MiB)"), "{text}");
    assert!(text.contains("limit of 8 MiB"), "{text}");
}
