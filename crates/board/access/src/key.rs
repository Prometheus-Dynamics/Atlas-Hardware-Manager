//! A public key a person may install: one OpenSSH ed25519 public key line, checked in form before
//! anything is sent. A private key is never read: a line that looks like one is refused.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::error::AccessError;

/// The key type this crate installs. Orion's trust store holds ed25519 keys only.
const TYPE: &str = "ssh-ed25519";
/// The most a key line may be, in bytes.
const MAX_LINE: usize = 1024;
/// The longest comment kept.
const MAX_COMMENT: usize = 256;
/// The length of the key blob: a length and the type name, a length and the 32 key bytes.
const BLOB_LEN: usize = 4 + 11 + 4 + 32;

/// One ed25519 public key with its comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    bytes: [u8; 32],
    comment: String,
}

impl PublicKey {
    /// A key from its 32 bytes and a comment (empty for none).
    #[must_use]
    pub fn ed25519(bytes: [u8; 32], comment: &str) -> Self {
        Self {
            bytes,
            comment: comment.to_owned(),
        }
    }

    /// Reads one OpenSSH public key line: `ssh-ed25519 BASE64 [comment]`.
    ///
    /// # Errors
    ///
    /// [`AccessError::BadKey`] with the reason, for anything that is not exactly that form: a
    /// private key, another key type, a blob that does not decode to an ed25519 key, a control
    /// character anywhere in the line, or a comment that is too long.
    pub fn parse(line: &str) -> Result<Self, AccessError> {
        let trimmed = line.trim();
        if trimmed.starts_with("-----BEGIN") {
            return Err(AccessError::BadKey(
                "a private key is never accepted: give the public key",
            ));
        }
        if trimmed.len() > MAX_LINE {
            return Err(AccessError::BadKey("the line is too long"));
        }
        if trimmed.chars().any(char::is_control) {
            return Err(AccessError::BadKey("the line has a control character"));
        }
        let words: Vec<&str> = trimmed.split(' ').filter(|word| !word.is_empty()).collect();
        let [kind, blob, rest @ ..] = words.as_slice() else {
            return Err(AccessError::BadKey("not an OpenSSH public key line"));
        };
        if *kind != TYPE {
            return Err(AccessError::BadKey("only ed25519 keys are installed"));
        }
        let decoded = STANDARD
            .decode(blob)
            .map_err(|_| AccessError::BadKey("the key does not decode"))?;
        let bytes = key_of_blob(&decoded)?;
        let comment = rest.join(" ");
        if comment.len() > MAX_COMMENT {
            return Err(AccessError::BadKey("the comment is too long"));
        }
        Ok(Self { bytes, comment })
    }

    /// The key's 32 bytes.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// The comment, empty for none.
    #[must_use]
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// The key as one OpenSSH public key line, the form the node is given.
    #[must_use]
    pub fn line(&self) -> String {
        let mut blob = Vec::with_capacity(BLOB_LEN);
        blob.extend_from_slice(&11_u32.to_be_bytes());
        blob.extend_from_slice(TYPE.as_bytes());
        blob.extend_from_slice(&32_u32.to_be_bytes());
        blob.extend_from_slice(&self.bytes);
        let encoded = STANDARD.encode(&blob);
        if self.comment.is_empty() {
            format!("{TYPE} {encoded}")
        } else {
            format!("{TYPE} {encoded} {}", self.comment)
        }
    }
}

fn key_of_blob(blob: &[u8]) -> Result<[u8; 32], AccessError> {
    let not_ed25519 = AccessError::BadKey("the key is not an ed25519 public key");
    if blob.len() != BLOB_LEN
        || blob[0..4] != 11_u32.to_be_bytes()
        || &blob[4..15] != TYPE.as_bytes()
        || blob[15..19] != 32_u32.to_be_bytes()
    {
        return Err(not_ed25519);
    }
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&blob[19..]);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_line_round_trips_with_and_without_a_comment() {
        let key = PublicKey::ed25519([7; 32], "ops@desk");
        assert_eq!(PublicKey::parse(&key.line()).unwrap(), key);
        let bare = PublicKey::ed25519([9; 32], "");
        assert_eq!(bare.line().split(' ').count(), 2);
        assert_eq!(PublicKey::parse(&bare.line()).unwrap(), bare);
    }

    #[test]
    fn a_malformed_line_is_refused_with_its_reason() {
        let good = PublicKey::ed25519([3; 32], "x").line();
        let blob = good.split(' ').nth(1).unwrap().to_owned();
        let short = STANDARD.encode(b"short");
        let cases: Vec<(String, &str)> = vec![
            (String::new(), "not an OpenSSH public key line"),
            ("ssh-ed25519".into(), "not an OpenSSH public key line"),
            (
                "ssh-rsa AAAAB3NzaC1yc2E user@h".into(),
                "only ed25519 keys are installed",
            ),
            ("ssh-ed25519 not*base64".into(), "the key does not decode"),
            (
                format!("ssh-ed25519 {short}"),
                "the key is not an ed25519 public key",
            ),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----".into(),
                "a private key is never accepted: give the public key",
            ),
            (
                format!("ssh-ed25519 {blob}\nssh-rsa AAAA"),
                "the line has a control character",
            ),
            (
                format!("ssh-ed25519 {blob} a\tb"),
                "the line has a control character",
            ),
        ];
        for (line, reason) in cases {
            assert_eq!(
                PublicKey::parse(&line),
                Err(AccessError::BadKey(reason)),
                "line {line:?}"
            );
        }
    }

    #[test]
    fn a_blob_of_another_key_type_inside_an_ed25519_line_is_refused() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11_u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-dss\0\0\0\0");
        blob.extend_from_slice(&32_u32.to_be_bytes());
        blob.extend_from_slice(&[1; 32]);
        let line = format!("ssh-ed25519 {}", STANDARD.encode(&blob));
        assert!(PublicKey::parse(&line).is_err());
    }
}
