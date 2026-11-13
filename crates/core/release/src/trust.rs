//! Who signed an image file: a minisign signature `<file>.minisig` next to
//! it, checked against the keys Atlas trusts. Each trusted key carries its
//! owner (name, company, contact, purpose, dates), so Atlas can say who
//! signed a file, not only that someone did.
//!
//! The keys are data: `keys/trusted-keys.json` is built in, and a user's own
//! file of the same shape is read on top of it (the caller passes its path).
//! A new key is a new entry, not a code change.
//!
//! [`Security`] is the one switch. Off (the default): a bad, unknown or
//! missing signature is reported and the file may still be used. On: any
//! signature that is not good, trusted and unexpired is a refusal.

use std::path::{Path, PathBuf};

use base64::Engine;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{ReleaseError, now_ms};

const BUILTIN: &str = include_str!("../keys/trusted-keys.json");
const READ_CHUNK: usize = 1024 * 1024;

/// Whether signatures decide what may be used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    /// Unsigned and unknown-key files are allowed; bad signatures are reported.
    #[default]
    Off,
    /// Only files and manifests with a good signature from a trusted,
    /// unexpired key are used. Everything else is refused.
    On,
}

/// What a key is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    /// Dev and example builds; its private key isn't guarded.
    Dev,
    Production,
}

impl Purpose {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dev => "dev/example",
            Self::Production => "production",
        }
    }
}

/// A key Atlas trusts and who owns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKey {
    pub name: String,
    /// Only `minisign` so far.
    pub kind: String,
    /// minisign's key id: 16 hex digits, as `minisign -G` prints it.
    pub key_id: String,
    /// The second line of the minisign .pub file.
    pub public_key: String,
    /// A person or a company.
    pub owner: String,
    /// An address or URL to reach the owner.
    pub contact: String,
    pub purpose: Purpose,
    /// YYYY-MM-DD.
    pub created: String,
    /// YYYY-MM-DD; signatures by an expired key are refused when security is on.
    pub expires: Option<String>,
    #[serde(default)]
    pub note: String,
    /// Where the entry came from: built in, or the user's file.
    #[serde(default, skip_deserializing)]
    pub source: String,
}

impl TrustedKey {
    /// `minisign:<key id>`.
    pub fn fingerprint(&self) -> String {
        format!("{}:{}", self.kind, self.key_id.to_ascii_uppercase())
    }

    /// `Example key (Example Co, dev/example)`.
    pub fn who(&self) -> String {
        format!("{} ({}, {})", self.name, self.owner, self.purpose.label())
    }

    fn expired(&self, today: &str) -> bool {
        self.expires
            .as_deref()
            .is_some_and(|expires| expires < today)
    }

    fn verifying_key(&self) -> Result<([u8; 8], VerifyingKey), String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(self.public_key.trim())
            .map_err(|error| format!("public key: {error}"))?;
        if bytes.len() != 42 || &bytes[..2] != b"Ed" {
            return Err("public key: not a minisign Ed25519 key".into());
        }
        let id: [u8; 8] = bytes[2..10].try_into().expect("8 bytes");
        let key: [u8; 32] = bytes[10..42].try_into().expect("32 bytes");
        let key = VerifyingKey::from_bytes(&key).map_err(|error| format!("public key: {error}"))?;
        if key_id_hex(&id) != self.key_id.to_ascii_uppercase() {
            return Err(format!(
                "public key: its id is {}, the entry says {}",
                key_id_hex(&id),
                self.key_id
            ));
        }
        Ok((id, key))
    }
}

/// minisign prints the id as the little-endian u64 in hex.
fn key_id_hex(id: &[u8; 8]) -> String {
    format!("{:016X}", u64::from_le_bytes(*id))
}

#[derive(Deserialize)]
struct KeyFile {
    keys: Vec<TrustedKey>,
}

/// The keys Atlas trusts.
#[derive(Clone, Debug, Default)]
pub struct Keyring {
    pub keys: Vec<TrustedKey>,
    /// Entries that couldn't be used, and why (a bad user file, a key that
    /// doesn't parse).
    pub problems: Vec<String>,
}

impl Keyring {
    /// The keys built into Atlas.
    pub fn builtin() -> Self {
        Self::from_json(BUILTIN, "built in")
    }

    /// The built-in keys plus the user's key file, when there is one.
    pub fn load(user_file: Option<&Path>) -> Self {
        let mut ring = Self::builtin();
        if let Some(path) = user_file
            && let Ok(text) = std::fs::read_to_string(path)
        {
            let user = Self::from_json(&text, &path.display().to_string());
            ring.keys.extend(user.keys);
            ring.problems.extend(user.problems);
        }
        ring
    }

    pub fn from_json(text: &str, source: &str) -> Self {
        let mut ring = Self::default();
        match serde_json::from_str::<KeyFile>(text) {
            Ok(file) => {
                for mut key in file.keys {
                    key.source = source.to_string();
                    match key.kind.as_str() {
                        "minisign" => match key.verifying_key() {
                            Ok(_) => ring.keys.push(key),
                            Err(error) => ring
                                .problems
                                .push(format!("{source}: {}: {error}", key.name)),
                        },
                        other => ring
                            .problems
                            .push(format!("{source}: {}: unknown kind {other}", key.name)),
                    }
                }
            }
            Err(error) => ring.problems.push(format!("{source}: {error}")),
        }
        ring
    }

    fn by_id(&self, id: &[u8; 8]) -> Option<&TrustedKey> {
        let hex = key_id_hex(id);
        self.keys
            .iter()
            .find(|key| key.key_id.eq_ignore_ascii_case(&hex))
    }
}

/// What the signature next to a file says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum SignatureStatus {
    /// A trusted key signed it and the file matches.
    Signed {
        key: Box<TrustedKey>,
        /// minisign's trusted comment (Gaia's names the file and time).
        comment: String,
        /// The key's expiry date has passed.
        expired: bool,
    },
    /// No `<file>.minisig`.
    Unsigned,
    /// Signed by a key Atlas doesn't trust.
    UnknownKey { key_id: String },
    /// A signature that doesn't match the file, or can't be read.
    Bad { reason: String },
}

impl SignatureStatus {
    /// True for a good signature by a trusted, unexpired key.
    pub fn trusted(&self) -> bool {
        matches!(self, Self::Signed { expired: false, .. })
    }

    /// One line for people: who signed it, or why it can't be told.
    pub fn summary(&self) -> String {
        match self {
            Self::Signed { key, .. } => format!("signed by {}", key.who()),
            Self::Unsigned => "unsigned".into(),
            Self::UnknownKey { key_id } => format!("signed by unknown key minisign:{key_id}"),
            Self::Bad { reason } => format!("bad signature: {reason}"),
        }
    }

    /// What to warn about, if anything.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Signed {
                key, expired: true, ..
            } => Some(format!(
                "signed by {}, which expired on {}",
                key.who(),
                key.expires.as_deref().unwrap_or("?")
            )),
            Self::Signed { key, .. } if key.purpose == Purpose::Dev => Some(format!(
                "signed with a dev/example key ({}): fine for testing, not a production release",
                key.name
            )),
            Self::Signed { .. } => None,
            Self::Unsigned => Some("unsigned: where this file came from can't be verified".into()),
            Self::UnknownKey { key_id } => Some(format!(
                "signed by key minisign:{key_id}, which isn't in Atlas's trusted keys"
            )),
            Self::Bad { reason } => Some(format!(
                "its signature doesn't check out ({reason}): the file may have changed after it was signed"
            )),
        }
    }

    /// Under [`Security::On`], refuses anything but a trusted, unexpired
    /// signature. Under [`Security::Off`] nothing is refused here.
    pub(crate) fn judge(&self, name: &str, security: Security) -> Result<(), ReleaseError> {
        if security == Security::Off || self.trusted() {
            return Ok(());
        }
        Err(ReleaseError::Untrusted {
            name: name.to_string(),
            reason: self
                .warning()
                .unwrap_or_else(|| "its signature is not trusted".into()),
        })
    }
}

/// The signature file Atlas looks for next to `path`.
pub fn signature_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".minisig");
    PathBuf::from(name)
}

/// Checks `<path>.minisig` against the keyring. Reads the whole file (to
/// hash it); call it off the async runtime.
pub fn check_file(path: &Path, ring: &Keyring) -> SignatureStatus {
    let text = match std::fs::read_to_string(signature_path(path)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SignatureStatus::Unsigned;
        }
        Err(error) => {
            return SignatureStatus::Bad {
                reason: format!("{}: {error}", signature_path(path).display()),
            };
        }
    };
    let signature = match MinisignSignature::parse(&text) {
        Ok(signature) => signature,
        Err(reason) => return SignatureStatus::Bad { reason },
    };
    let Some(key) = ring.by_id(&signature.key_id) else {
        return SignatureStatus::UnknownKey {
            key_id: key_id_hex(&signature.key_id),
        };
    };
    let digest = match blake2b_file(path) {
        Ok(digest) => digest,
        Err(error) => {
            return SignatureStatus::Bad {
                reason: format!("{}: {error}", path.display()),
            };
        }
    };
    match signature.verify(key, &digest) {
        Ok(()) => SignatureStatus::Signed {
            key: Box::new(key.clone()),
            comment: signature.trusted_comment,
            expired: key.expired(&today()),
        },
        Err(reason) => SignatureStatus::Bad { reason },
    }
}

fn blake2b_file(path: &Path) -> std::io::Result<[u8; 64]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Blake2b512::new();
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// A minisign signature file: the signature over the file's BLAKE2b-512
/// ("ED", minisign 0.10 on), and the global signature over that signature
/// and the trusted comment.
struct MinisignSignature {
    key_id: [u8; 8],
    signature: [u8; 64],
    trusted_comment: String,
    global: [u8; 64],
}

impl MinisignSignature {
    fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let (Some(_untrusted), Some(signature), Some(trusted), Some(global)) =
            (lines.next(), lines.next(), lines.next(), lines.next())
        else {
            return Err("the signature file is cut short".into());
        };
        let decode = |line: &str| {
            base64::engine::general_purpose::STANDARD
                .decode(line.trim())
                .map_err(|error| format!("the signature file: {error}"))
        };
        let signature = decode(signature)?;
        if signature.len() != 74 {
            return Err("the signature file: not a minisign signature".into());
        }
        match &signature[..2] {
            b"ED" => {}
            b"Ed" => {
                return Err(
                    "a legacy minisign signature (not prehashed): sign with minisign 0.10 or later"
                        .into(),
                );
            }
            _ => return Err("the signature file: not a minisign Ed25519 signature".into()),
        }
        let trusted_comment = trusted
            .strip_prefix("trusted comment: ")
            .ok_or("the signature file has no trusted comment")?
            .to_string();
        let global = decode(global)?;
        Ok(Self {
            key_id: signature[2..10].try_into().expect("8 bytes"),
            signature: signature[10..74].try_into().expect("64 bytes"),
            trusted_comment,
            global: global
                .as_slice()
                .try_into()
                .map_err(|_| "the signature file: its global signature isn't 64 bytes")?,
        })
    }

    fn verify(&self, key: &TrustedKey, digest: &[u8; 64]) -> Result<(), String> {
        let (_, public) = key.verifying_key()?;
        public
            .verify(digest, &Signature::from_bytes(&self.signature))
            .map_err(|_| "the file doesn't match its signature".to_string())?;
        let mut global = self.signature.to_vec();
        global.extend_from_slice(self.trusted_comment.as_bytes());
        public
            .verify(&global, &Signature::from_bytes(&self.global))
            .map_err(|_| "the trusted comment doesn't match its signature".to_string())
    }
}

/// Today as YYYY-MM-DD (UTC).
fn today() -> String {
    let days = i64::try_from(now_ms() / 86_400_000).unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's civil_from_days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
pub(crate) fn fixture(name: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(name)
}

#[cfg(test)]
pub(crate) mod tests;
