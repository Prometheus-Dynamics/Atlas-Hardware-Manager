use std::fmt::Write as _;
use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::ReleaseError;

const READ_CHUNK: usize = 1024 * 1024;

/// Hashes a file without loading it into memory. Returns lowercase hex.
pub async fn sha256_file(path: &Path) -> Result<String, ReleaseError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| ReleaseError::io(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| ReleaseError::io(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// Accepts `sha256:<hex>` or bare hex in any case; returns lowercase hex.
pub fn normalize_sha256(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let value = match trimmed.split_once(':') {
        Some((algorithm, value)) if algorithm.trim().eq_ignore_ascii_case("sha256") => value.trim(),
        Some(_) => return None,
        None => trimmed,
    };
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn normalize_accepts_prefixed_and_bare_hex() {
        assert_eq!(normalize_sha256(DIGEST).as_deref(), Some(DIGEST));
        assert_eq!(
            normalize_sha256(&format!("SHA256:{}", DIGEST.to_uppercase())).as_deref(),
            Some(DIGEST)
        );
        assert_eq!(normalize_sha256("sha1:abcd"), None);
        assert_eq!(normalize_sha256("abcd"), None);
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 171, 255];
        assert_eq!(from_hex(&to_hex(&bytes)).unwrap(), bytes);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[tokio::test]
    async fn file_hash_matches_known_value() {
        let path = std::env::temp_dir().join(format!("atlas-release-hash-{}", std::process::id()));
        tokio::fs::write(&path, b"abc").await.unwrap();
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = tokio::fs::remove_file(path).await;
    }
}
