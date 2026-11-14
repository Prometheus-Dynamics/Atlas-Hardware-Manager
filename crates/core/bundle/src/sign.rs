//! `slot.sig`: the signature member of a slot bundle, in the first-flash record's form.
//!
//! The member is exactly [`RECORD_LEN`] (108) bytes, byte-identical to the first-flash
//! record `atlas-image` reads and writes: `LMNSSIG1` | algorithm 1 (Ed25519) | 3 zero bytes |
//! key id (SHA-256 of the public key, 32 bytes) | signature (64 bytes). The record code is
//! [`atlas_image::Record`]; nothing here writes the record a second way.
//!
//! What is signed is not the bare `slot.json`. It is the message [`slot_message`] builds: the
//! tag `lemnos-slot-bundle\0`, a u32 little-endian version (1), a u32 little-endian length, and
//! the exact bytes of `slot.json`. A signature over the bare bytes does not verify.
//!
//! The signing key is a parameter and is never stored here. The keys a reader holds are
//! `atlas-release`'s [`PublicKey`]; the signing and verifying are its [`sign_bytes`] and
//! [`PublicKey::verify_bytes`].
// LOCK: Lemnos L2. Today Lemnos reads a bare 64-byte `slot.sig` (and a `slot.key` member) and
// refuses this form. The message, the record and the order below are Lemnos's I5g2 (the vector is
// in tests/bundle.rs). Nothing moves here at the lock except the vector if Lemnos changes it.

use atlas_image::{Algorithm, Record};
use atlas_release::{PublicKey, public_key_for, sign_bytes};
use sha2::{Digest, Sha256};

use crate::BundleError;

/// The ed25519 signature's length in bytes (the record's last field).
pub const SIG_LEN: usize = 64;
/// The `slot.sig` member's length: the first-flash record (`atlas_image::Record`), 108 bytes.
pub const RECORD_LEN: usize = 108;
/// The domain tag at the start of the signed message, with its NUL (19 bytes).
pub const SLOT_TAG: &[u8; 19] = b"lemnos-slot-bundle\0";
/// The signed message's version.
pub const SLOT_MESSAGE_VERSION: u32 = 1;

/// The bytes `slot.sig` signs for `slot_json`, the exact `slot.json` bytes: the tag, a u32 LE
/// version, a u32 LE length, then the manifest. This is the only construction of the message in
/// this crate; the signer and the verifier both call it.
///
/// # Errors
///
/// `Malformed` when `slot_json` is longer than a u32 can count.
pub fn slot_message(slot_json: &[u8]) -> Result<Vec<u8>, BundleError> {
    let length = u32::try_from(slot_json.len()).map_err(|_| {
        BundleError::Malformed("slot.json is too long to sign: its length is not a u32".into())
    })?;
    let mut message = Vec::with_capacity(SLOT_TAG.len() + 8 + slot_json.len());
    message.extend_from_slice(SLOT_TAG);
    message.extend_from_slice(&SLOT_MESSAGE_VERSION.to_le_bytes());
    message.extend_from_slice(&length.to_le_bytes());
    message.extend_from_slice(slot_json);
    Ok(message)
}

/// Signs `slot_json` (the exact bytes written to `slot.json`) and returns the record that is the
/// `slot.sig` member.
///
/// # Errors
///
/// As [`slot_message`].
pub fn sign_slot(slot_json: &[u8], secret: &[u8; 32]) -> Result<Record, BundleError> {
    let message = slot_message(slot_json)?;
    let signature = sign_bytes(&message, secret);
    Ok(Record {
        algorithm: Algorithm::Ed25519,
        key_id: key_id_of(&public_key_for(secret)),
        signature,
    })
}

/// What a reader found about a bundle's signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signature {
    /// No `slot.sig` member: a legal bundle.
    Unsigned,
    /// Signed by a key on the list, and the signature verifies. `signer` indexes that list.
    Verified { signer: usize },
    /// Signed by a key the list does not hold: nothing is checked, and it is not refused. The key
    /// id is lowercase hex.
    UnknownKey { key_id: String },
}

/// The key id of a public key: the SHA-256 of its 32 raw bytes.
#[must_use]
pub fn key_id_of(key: &PublicKey) -> [u8; 32] {
    Sha256::digest(raw_public(key)).into()
}

/// Checks `record_bytes` (the 108-byte `slot.sig` member) against `slot_json` with `keys`:
///
/// - a malformed record (length, magic, algorithm, padding) is refused;
/// - a record whose key id is a key on the list must verify under that key, or it is refused
///   with [`BundleError::BadSignature`];
/// - a record whose key id is on no list is [`Signature::UnknownKey`].
///
/// # Errors
///
/// `Malformed` for a malformed record; `BadSignature` for a held key that does not verify.
pub fn verify_slot(
    slot_json: &[u8],
    record_bytes: &[u8],
    keys: &[PublicKey],
) -> Result<Signature, BundleError> {
    if record_bytes.len() != RECORD_LEN {
        return Err(BundleError::Malformed(format!(
            "slot.sig is {} bytes, not the {RECORD_LEN}-byte signature record",
            record_bytes.len()
        )));
    }
    let record = Record::parse(record_bytes)
        .map_err(|error| BundleError::Malformed(format!("slot.sig: {error}")))?;
    let message = slot_message(slot_json)?;
    match keys.iter().position(|key| key_id_of(key) == record.key_id) {
        None => Ok(Signature::UnknownKey {
            key_id: hex(&record.key_id),
        }),
        Some(signer) => {
            if keys[signer].verify_bytes(&message, &record.signature) {
                Ok(Signature::Verified { signer })
            } else {
                Err(BundleError::BadSignature)
            }
        }
    }
}

/// The raw 32 bytes of a public key. `atlas-release` keeps the key private and writes it as
/// `ed25519:<64 hex>`; this reads those bytes back.
fn raw_public(key: &PublicKey) -> [u8; 32] {
    let text = key.to_string();
    let hex_text = text.strip_prefix("ed25519:").unwrap_or(&text);
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(hex_text.as_bytes().chunks(2)) {
        *byte = std::str::from_utf8(pair)
            .ok()
            .and_then(|digits| u8::from_str_radix(digits, 16).ok())
            .unwrap_or(0);
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
