//! Atlas reads and verifies a bundle written by Lemnos's own writer, `lemnos-ctl bundle`, at Lemnos
//! f4a295c6. The fixture and how it was made are in `tests/fixtures/lemnos-f4a295c6/README.md`. The key is
//! a TEST key (its seed is not committed).
//!
//! This is the check the hand-written writer could not give: the writer and reader of this crate agreed
//! on a 64-byte signature while Lemnos's bundles carry a 108-byte record, and no test here saw it.

use atlas_bundle::{BundleError, RECORD_LEN, inspect_bundle, verify_slot};
use atlas_release::{PublicKey, public_key_for};
use lemnos_slot_store::bundle::hex;
use lemnos_slot_store::signing;

const BUNDLE: &[u8] = include_bytes!("../fixtures/lemnos-f4a295c6/slot-a1.bundle");
const PUBKEY: &str = include_str!("../fixtures/lemnos-f4a295c6/test-key.pub");
/// The test key's key id: SHA-256 of its raw public key, as `lemnos-ctl` printed it.
const KEY_ID: &str = "600ebf633600aabc22786f2320e2fa57afe30bec95deda750c6b32883fa58c44";
/// `verity.root_hash` in the fixture's `slot.json`.
const ROOT_HASH: &str = "753405055899b1842317bbaee7dd30ee515401624de7e596ceca247868d39473";

fn test_key() -> PublicKey {
    PublicKey::parse(PUBKEY).expect("the fixture's public key parses")
}

/// The bundle's members by name, in archive order.
fn members(bundle: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = tar::Archive::new(bundle);
    let mut out = Vec::new();
    for entry in archive.entries().expect("a tar") {
        let mut entry = entry.expect("an entry");
        let name = entry.path().expect("a path").to_string_lossy().into_owned();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data).expect("the entry reads");
        out.push((name, data));
    }
    out
}

/// A tar of `members`, the same shape Lemnos writes: regular files and the `boot/` directory.
fn repack(members: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut out);
        for (name, data) in members {
            let mut header = tar::Header::new_gnu();
            if name.ends_with('/') {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                builder
                    .append_data(&mut header, name, std::io::empty())
                    .expect("a directory appends");
            } else {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                header.set_size(data.len() as u64);
                builder
                    .append_data(&mut header, name, data.as_slice())
                    .expect("a file appends");
            }
        }
        builder.finish().expect("the tar finishes");
    }
    out
}

fn member<'a>(members: &'a [(String, Vec<u8>)], name: &str) -> &'a [u8] {
    &members
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("the fixture has {name}"))
        .1
}

#[test]
fn atlas_reads_and_verifies_the_bundle_lemnos_ctl_wrote() {
    let inspected = inspect_bundle(BUNDLE, &[test_key()]).expect("Lemnos's bundle inspects");
    assert_eq!(inspected.signer, 0, "signed by the fixture's key");
    assert_eq!(inspected.board, "reference");
    assert_eq!(inspected.manifest.id, "a1");
    assert_eq!(inspected.manifest.product, "fixture-os");
    assert_eq!(inspected.manifest.version, "1.0");
    assert_eq!(inspected.manifest.verity.root_hash, ROOT_HASH);
    assert_eq!(
        inspected.manifest.boot.files.len(),
        5,
        "the five role files"
    );
}

#[test]
fn the_signature_is_lemnos_108_byte_record_over_its_message() {
    let all = members(BUNDLE);
    let json = member(&all, "slot.json");
    let sig = member(&all, "slot.sig");
    assert_eq!(sig.len(), RECORD_LEN, "a 108-byte LMNSSIG1 record");
    assert_eq!(&sig[..8], b"LMNSSIG1");
    let record = signing::parse_record(sig).expect("Lemnos reads its own record");
    assert_eq!(hex(&record.key_id), KEY_ID);
    assert_eq!(verify_slot(json, sig, &[test_key()]).expect("verifies"), 0);
}

#[test]
fn a_changed_byte_of_slot_json_is_refused_as_a_bad_signature() {
    let mut all = members(BUNDLE);
    let json = &mut all
        .iter_mut()
        .find(|(n, _)| n == "slot.json")
        .expect("slot.json")
        .1;
    let at = json
        .windows(10)
        .position(|w| w == b"fixture-os")
        .expect("the product is in slot.json");
    json[at + 8] = b'0'; // "fixture-o0": the same length, another byte
    let error = inspect_bundle(&repack(&all)[..], &[test_key()]).expect_err("refused");
    assert!(matches!(error, BundleError::BadSignature), "{error:?}");
}

#[test]
fn a_changed_root_image_byte_is_refused() {
    let mut all = members(BUNDLE);
    let image = &mut all
        .iter_mut()
        .find(|(n, _)| n == "rootfs.erofs")
        .expect("rootfs")
        .1;
    image[100] ^= 1;
    assert!(inspect_bundle(&repack(&all)[..], &[test_key()]).is_err());
}

#[test]
fn a_bare_64_byte_signature_is_not_a_slot_sig() {
    let mut all = members(BUNDLE);
    let record = member(&all, "slot.sig").to_vec();
    let bare = record[44..108].to_vec();
    all.iter_mut()
        .find(|(n, _)| n == "slot.sig")
        .expect("slot.sig")
        .1 = bare;
    let error = inspect_bundle(&repack(&all)[..], &[test_key()]).expect_err("refused");
    match error {
        BundleError::Malformed(text) => assert!(text.contains("64 bytes"), "{text}"),
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn a_key_that_did_not_sign_does_not_verify() {
    let other = public_key_for(&[9u8; 32]);
    let error = inspect_bundle(BUNDLE, &[other]).expect_err("refused");
    assert!(matches!(error, BundleError::BadSignature), "{error:?}");
}
