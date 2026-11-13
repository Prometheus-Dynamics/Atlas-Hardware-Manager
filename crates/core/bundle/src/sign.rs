//! `slot.sig`: a raw 64-byte ed25519 signature over the exact bytes of
//! `slot.json`. The keys and their owners are `atlas-release`'s
//! [`PublicKey`]; the signing key is a parameter and is never stored here.

use atlas_release::PublicKey;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::BundleError;

/// The signature's length in bytes.
pub const SIG_LEN: usize = 64;

/// Signs `slot_json`, the exact bytes written to `slot.json`.
pub fn sign_slot(slot_json: &[u8], secret: &[u8; 32]) -> [u8; SIG_LEN] {
    SigningKey::from_bytes(secret).sign(slot_json).to_bytes()
}

/// The index in `keys` of the key that made `signature` over `slot_json`.
/// Any other signature, or no key that verifies it, is `BadSignature`.
pub fn verify_slot(
    slot_json: &[u8],
    signature: &[u8],
    keys: &[PublicKey],
) -> Result<usize, BundleError> {
    let bytes: [u8; SIG_LEN] = signature
        .try_into()
        .map_err(|_| BundleError::BadSignature)?;
    let signature = Signature::from_bytes(&bytes);
    keys.iter()
        .position(|key| {
            verifying_key(key)
                .is_some_and(|verifying| verifying.verify(slot_json, &signature).is_ok())
        })
        .ok_or(BundleError::BadSignature)
}

/// The ed25519 key inside an `atlas-release` public key. Its text form is
/// `ed25519:<64 hex>`, the form the release catalog pins.
fn verifying_key(key: &PublicKey) -> Option<VerifyingKey> {
    let text = key.to_string();
    let hex = text.strip_prefix("ed25519:")?;
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    VerifyingKey::from_bytes(&bytes).ok()
}
