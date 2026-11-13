//! `slot.sig`: a raw 64-byte ed25519 signature over the exact bytes of
//! `slot.json`. The keys and their owners are `atlas-release`'s
//! [`PublicKey`]; the signing and verifying are its [`sign_bytes`] and
//! [`PublicKey::verify_bytes`]. The signing key is a parameter and is never
//! stored here.

use atlas_release::{PublicKey, sign_bytes};

use crate::BundleError;

/// The signature's length in bytes.
pub const SIG_LEN: usize = 64;

/// Signs `slot_json`, the exact bytes written to `slot.json`.
pub fn sign_slot(slot_json: &[u8], secret: &[u8; 32]) -> [u8; SIG_LEN] {
    sign_bytes(slot_json, secret)
}

/// The index in `keys` of the key that made `signature` over `slot_json`.
/// Any other signature, or no key that verifies it, is `BadSignature`.
pub fn verify_slot(
    slot_json: &[u8],
    signature: &[u8],
    keys: &[PublicKey],
) -> Result<usize, BundleError> {
    keys.iter()
        .position(|key| key.verify_bytes(slot_json, signature))
        .ok_or(BundleError::BadSignature)
}
