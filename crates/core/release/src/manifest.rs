use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::hash::{from_hex, normalize_sha256, to_hex};
use crate::{IMAGE_SUFFIX, ReleaseError};

const KEY_PREFIX: &str = "ed25519:";

/// A pinned ed25519 public key, written `ed25519:<64 hex chars>`.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn parse(text: &str) -> Result<Self, ReleaseError> {
        let hex = text.trim().strip_prefix(KEY_PREFIX).unwrap_or(text.trim());
        let bytes: [u8; 32] = from_hex(hex)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| ReleaseError::InvalidKey("expected 32 bytes of hex".into()))?;
        VerifyingKey::from_bytes(&bytes)
            .map(Self)
            .map_err(|error| ReleaseError::InvalidKey(error.to_string()))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{KEY_PREFIX}{}", to_hex(self.0.as_bytes()))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestArtifact {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256:<hex>` or bare hex. Empty means unknown: only allowed in an
    /// unsigned manifest, and the file is then not hash-checked.
    pub sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_source_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_updater_version: Option<String>,
}

/// Everything in a manifest that the signature covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestBody {
    pub manifest_version: u32,
    pub family: String,
    pub version: String,
    pub channel: String,
    #[serde(default)]
    pub boards: Vec<String>,
    pub artifact: ManifestArtifact,
    #[serde(default)]
    pub requires: Requirements,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_url: Option<String>,
}

impl ManifestBody {
    /// The exact bytes that are signed: compact JSON in field order.
    fn signed_bytes(&self) -> Result<Vec<u8>, ReleaseError> {
        serde_json::to_vec(self).map_err(|error| ReleaseError::InvalidManifest(error.to_string()))
    }

    /// The only image kind is a first-flash `.img.xz` with a size and an
    /// http(s) address.
    pub(crate) fn check_image(&self) -> Result<(), ReleaseError> {
        let artifact = &self.artifact;
        if !artifact.name.ends_with(IMAGE_SUFFIX) || artifact.name.contains(['/', '\\']) {
            return Err(ReleaseError::NotAnImage(artifact.name.clone()));
        }
        if artifact.size == 0 {
            return Err(ReleaseError::InvalidManifest("artifact.size is 0".into()));
        }
        if !(artifact.url.starts_with("https://") || artifact.url.starts_with("http://")) {
            return Err(ReleaseError::InvalidManifest(
                "artifact.url is not an http(s) address".into(),
            ));
        }
        Ok(())
    }
}

/// One signed release, as published next to its artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(flatten)]
    pub body: ManifestBody,
    /// `ed25519:<128 hex chars>` over [`ManifestBody`]'s compact JSON.
    pub signature: String,
}

impl Manifest {
    /// Checks the shape of the manifest and that one of `keys` signed it.
    pub fn verify(&self, keys: &[PublicKey]) -> Result<(), ReleaseError> {
        if self.body.manifest_version != 1 {
            return Err(ReleaseError::InvalidManifest(format!(
                "manifest_version {} is not supported",
                self.body.manifest_version
            )));
        }
        if normalize_sha256(&self.body.artifact.sha256).is_none() {
            return Err(ReleaseError::InvalidManifest(
                "artifact.sha256 is missing or not a SHA-256 digest".into(),
            ));
        }
        let bad_signature = || ReleaseError::BadSignature {
            family: self.body.family.clone(),
            version: self.body.version.clone(),
        };
        let signature_bytes: [u8; 64] = self
            .signature
            .trim()
            .strip_prefix(KEY_PREFIX)
            .and_then(from_hex)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(bad_signature)?;
        let signature = Signature::from_bytes(&signature_bytes);
        let message = self.body.signed_bytes()?;
        keys.iter()
            .any(|key| key.0.verify(&message, &signature).is_ok())
            .then_some(())
            .ok_or_else(bad_signature)
    }
}

/// Signs a manifest body. Used by release tooling and tests; Atlas itself
/// only verifies.
pub fn sign_manifest(body: ManifestBody, secret_key: &[u8; 32]) -> Result<Manifest, ReleaseError> {
    let key = SigningKey::from_bytes(secret_key);
    let signature = key.sign(&body.signed_bytes()?);
    Ok(Manifest {
        body,
        signature: format!("{KEY_PREFIX}{}", to_hex(&signature.to_bytes())),
    })
}

/// The public key for a secret key, for tooling and tests.
pub fn public_key_for(secret_key: &[u8; 32]) -> PublicKey {
    PublicKey(SigningKey::from_bytes(secret_key).verifying_key())
}

/// A raw ed25519 signature over `message`: 64 bytes, no encoding around them.
/// Used for files whose format is only their signature (a bundle's `slot.sig`).
/// The signing key is a parameter and is never stored here.
pub fn sign_bytes(message: &[u8], secret_key: &[u8; 32]) -> [u8; 64] {
    SigningKey::from_bytes(secret_key).sign(message).to_bytes()
}

impl PublicKey {
    /// Whether `signature` is a good raw ed25519 signature of `message` by this
    /// key. A signature that is not 64 bytes is never good.
    pub fn verify_bytes(&self, message: &[u8], signature: &[u8]) -> bool {
        let Ok(bytes) = <[u8; 64]>::try_from(signature) else {
            return false;
        };
        self.0
            .verify(message, &Signature::from_bytes(&bytes))
            .is_ok()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn raw_signatures_verify_with_the_matching_key_only() {
        let secret = throwaway(40);
        let other = throwaway(41);
        let key = public_key_for(&secret);
        let message = b"slot.json bytes";
        let signature = sign_bytes(message, &secret);

        assert!(key.verify_bytes(message, &signature));
        assert!(!public_key_for(&other).verify_bytes(message, &signature));
    }

    #[test]
    fn one_changed_byte_or_a_short_signature_fails() {
        let secret = throwaway(42);
        let key = public_key_for(&secret);
        let mut message = b"slot.json bytes".to_vec();
        let signature = sign_bytes(&message, &secret);

        message[0] ^= 1;
        assert!(!key.verify_bytes(&message, &signature));
        message[0] ^= 1;
        assert!(key.verify_bytes(&message, &signature));
        assert!(!key.verify_bytes(&message, &signature[..63]));
        assert!(!key.verify_bytes(&message, &[]));
    }

    /// A throwaway secret made when a test runs: never stored in the repo.
    pub(crate) fn throwaway(salt: u8) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(std::process::id().to_le_bytes());
        hasher.update(crate::now_ms().to_le_bytes());
        hasher.update([salt]);
        hasher.finalize().into()
    }

    pub(crate) fn body(version: &str) -> ManifestBody {
        ManifestBody {
            manifest_version: 1,
            family: "example-family".into(),
            version: version.into(),
            channel: "stable".into(),
            boards: vec!["example-board".into()],
            artifact: ManifestArtifact {
                name: format!("example-{version}.img.xz"),
                url: format!("https://example.invalid/example-{version}.img.xz"),
                size: 3,
                sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            },
            requires: Requirements::default(),
            methods: vec!["image-write".into()],
            notes_url: None,
        }
    }

    #[test]
    fn signed_manifests_verify_with_the_matching_key_only() {
        let secret = throwaway(0);
        let manifest = sign_manifest(body("2026.3.1"), &secret).unwrap();
        let trusted = public_key_for(&secret);
        let other = public_key_for(&throwaway(1));

        assert!(manifest.verify(std::slice::from_ref(&trusted)).is_ok());
        assert!(manifest.verify(std::slice::from_ref(&other)).is_err());
        assert!(manifest.verify(&[other, trusted]).is_ok());
    }

    #[test]
    fn tampering_breaks_the_signature() {
        let secret = throwaway(0);
        let mut manifest = sign_manifest(body("2026.3.1"), &secret).unwrap();
        manifest.body.artifact.url = "https://evil.invalid/image.img.xz".into();

        let error = manifest.verify(&[public_key_for(&secret)]).unwrap_err();
        assert!(matches!(error, ReleaseError::BadSignature { .. }));
    }

    #[test]
    fn manifests_round_trip_through_json() {
        let secret = throwaway(0);
        let manifest = sign_manifest(body("2026.3.1"), &secret).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, manifest);
        assert!(parsed.verify(&[public_key_for(&secret)]).is_ok());
    }

    #[test]
    fn public_keys_parse_their_display_form() {
        let key = public_key_for(&throwaway(0));
        assert_eq!(PublicKey::parse(&key.to_string()).unwrap(), key);
        assert!(PublicKey::parse("ed25519:1234").is_err());
    }

    #[test]
    fn only_first_flash_images_are_accepted() {
        assert!(body("1").check_image().is_ok());
        let mut other = body("1");
        other.artifact.name = "example-1.zip".into();
        assert!(matches!(
            other.check_image(),
            Err(ReleaseError::NotAnImage(_))
        ));
        let mut empty = body("1");
        empty.artifact.size = 0;
        assert!(empty.check_image().is_err());
        let mut plain = body("1");
        plain.artifact.url = "file:///etc/passwd".into();
        assert!(plain.check_image().is_err());
    }
}
