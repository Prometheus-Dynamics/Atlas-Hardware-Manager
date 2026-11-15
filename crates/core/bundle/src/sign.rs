//! `slot.sig`: Lemnos's signing record over `slot.json`'s exact bytes, 108 bytes:
//! `LMNSSIG1`, algorithm 1, three zero bytes, the signer's key id (SHA-256 of the public key), then the
//! Ed25519 signature over `lemnos_slot_store::signing::slot_bundle_message`. The format, the message and
//! the key id are Lemnos's, read and written by `lemnos-slot-store`; this module only applies them with
//! `atlas-release`'s keys. The signing key is a parameter and is never stored here.
//!
//! A bare 64-byte signature is not a `slot.sig`: it is refused as malformed.

use atlas_release::{PublicKey, public_key_for, sign_bytes};
use lemnos_slot_store::signing;

use crate::BundleError;

pub use lemnos_slot_store::signing::RECORD_LEN;

/// The 32 raw bytes of a public key. `atlas-release` keeps the key private and displays it as
/// `ed25519:<64 hex>`; the display is decoded here, the one place that does.
pub(crate) fn raw_public(key: &PublicKey) -> [u8; 32] {
    let text = key.to_string();
    let hex = text.strip_prefix("ed25519:").unwrap_or(&text);
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(pair).unwrap_or("");
        *slot = u8::from_str_radix(digits, 16)
            .expect("a public key displays as ed25519:<64 hex characters>");
    }
    out
}

/// The key id Lemnos gives a public key: SHA-256 of its 32 raw bytes.
pub(crate) fn key_id_of(key: &PublicKey) -> [u8; 32] {
    signing::key_id(&raw_public(key))
}

/// Signs `slot_json` (the exact bytes written to `slot.json`) and returns the 108-byte record.
pub fn sign_slot(slot_json: &[u8], secret: &[u8; 32]) -> Vec<u8> {
    let key_id = key_id_of(&public_key_for(secret));
    let signature = sign_bytes(&signing::slot_bundle_message(slot_json), secret);
    signing::record_bytes(&key_id, &signature)
}

/// The index in `keys` of the key that made `signature` (a `slot.sig` record) over `slot_json`.
///
/// # Errors
///
/// `Malformed` when `signature` is not a Lemnos record (length, magic, algorithm or padding). `BadSignature`
/// when no key in `keys` has the record's key id and verifies it.
pub fn verify_slot(
    slot_json: &[u8],
    signature: &[u8],
    keys: &[PublicKey],
) -> Result<usize, BundleError> {
    let record = signing::parse_record(signature)
        .map_err(|why| BundleError::Malformed(format!("slot.sig: {why}")))?;
    let message = signing::slot_bundle_message(slot_json);
    keys.iter()
        .position(|key| {
            key_id_of(key) == record.key_id && key.verify_bytes(&message, &record.signature)
        })
        .ok_or(BundleError::BadSignature)
}
