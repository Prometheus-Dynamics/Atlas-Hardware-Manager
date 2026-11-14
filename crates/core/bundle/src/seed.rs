//! The signing seed, its key id, and the `slot.sig` record signer.
//!
//! The key file both Atlas and Lemnos's `lemnos-ctl bundle` take is a raw Ed25519 seed: exactly 32
//! raw bytes, or 64 hex characters (with optional surrounding whitespace and one trailing newline).
//! The key id is the SHA-256 of the 32-byte raw public key derived from that seed. A key id a caller
//! states is checked against the derived one before anything is signed.
//!
//! The record is the 108-byte `LMNSSIG1` form `atlas-image` reads and writes (see [`Record`]), and
//! what is signed is [`slot_message`]: the tag `lemnos-slot-bundle\0`, a u32 LE version (1), a u32
//! LE length, then the exact bytes of `slot.json`. The vector is in `tests/seed.rs`.
//!
//! Nothing here prints seed bytes: [`Seed`]'s `Debug` is `Seed(..)`, it has no `Display`, and every
//! error names the path and the rule, never the file's content.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use atlas_image::{Algorithm, Record};
use atlas_release::{PublicKey, public_key_for, sign_bytes};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::BundleError;

/// The seed's length in bytes (the Ed25519 secret key).
pub const SEED_LEN: usize = 32;
/// The `slot.sig` record's length: the first-flash record `atlas-image` reads and writes (108 bytes).
const RECORD_LEN: usize = 108;
/// The domain tag at the start of the signed message, with its NUL (19 bytes).
const SLOT_TAG: &[u8; 19] = b"lemnos-slot-bundle\0";
/// The signed message's version.
const SLOT_MESSAGE_VERSION: u32 = 1;
/// The most bytes a seed file is read to: far more than either accepted form, so a huge file is
/// refused without being read whole.
const SEED_FILE_LIMIT: u64 = 256;
/// The prefix of a public key's display form (`atlas-release` writes `ed25519:<64 hex>`).
const PUBLIC_PREFIX: &str = "ed25519:";

/// An Ed25519 seed: the secret that signs a bundle's `slot.sig`.
///
/// It is wiped when dropped. It has no `Clone`, no `Display` and no `Serialize`; its `Debug` prints
/// `Seed(..)`.
pub struct Seed([u8; SEED_LEN]);

impl Drop for Seed {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Seed(..)")
    }
}

impl Seed {
    /// A seed from its 32 bytes. The caller owns the bytes it passes and should wipe its own copy.
    #[must_use]
    pub fn from_bytes(bytes: [u8; SEED_LEN]) -> Self {
        Self(bytes)
    }

    /// A new seed from the operating system's random source (`/dev/urandom`, the source the
    /// workspace's own tests read for their keys).
    ///
    /// # Errors
    ///
    /// `Seed` when the random source cannot be read.
    pub fn generate() -> Result<Self, BundleError> {
        let mut bytes = [0u8; SEED_LEN];
        let read = File::open("/dev/urandom").and_then(|mut random| random.read_exact(&mut bytes));
        if let Err(error) = read {
            bytes.zeroize();
            return Err(BundleError::Seed(format!(
                "reading the operating system's random bytes: {error}"
            )));
        }
        Ok(Self(bytes))
    }

    /// Reads a seed file: exactly 32 raw bytes, or 64 hex characters with optional surrounding
    /// whitespace and one trailing newline. On Unix a file readable by its group or others is
    /// refused (`chmod 600` it). A refusal names the path and the rule, never the content.
    ///
    /// # Errors
    ///
    /// `Seed` when the file cannot be read, has the wrong form, or has a mode that is not owner-only.
    pub fn read(path: &Path) -> Result<Self, BundleError> {
        let shown = path.display();
        let mut file = File::open(path).map_err(|error| {
            BundleError::Seed(format!("reading the seed file {shown}: {error}"))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file
                .metadata()
                .map_err(|error| {
                    BundleError::Seed(format!("reading the seed file {shown}: {error}"))
                })?
                .permissions()
                .mode()
                & 0o777;
            if mode & 0o077 != 0 {
                return Err(BundleError::Seed(format!(
                    "the seed file {shown} has mode {mode:o}, so it is readable by others than its owner: chmod 600 it"
                )));
            }
        }
        let mut content = Vec::new();
        let read = (&mut file)
            .take(SEED_FILE_LIMIT + 1)
            .read_to_end(&mut content)
            .map_err(|error| BundleError::Seed(format!("reading the seed file {shown}: {error}")));
        let decoded = match read {
            Ok(_) if content.len() as u64 <= SEED_FILE_LIMIT => decode_seed_file(&content),
            Ok(_) => None,
            Err(error) => {
                content.zeroize();
                return Err(error);
            }
        };
        content.zeroize();
        match decoded {
            Some(bytes) => Ok(Self(bytes)),
            None => Err(BundleError::Seed(format!(
                "the seed file {shown} must hold exactly 32 raw bytes, or 64 hex characters with optional surrounding whitespace and one trailing newline; it holds neither"
            ))),
        }
    }

    /// Writes this seed as 64 hex characters and a newline to a NEW file made with mode 0600. An
    /// existing file is never overwritten, and the parent directory must already exist.
    ///
    /// # Errors
    ///
    /// `Seed` when the file already exists, cannot be created or written, or its directory is missing.
    pub fn write_new(&self, path: &Path) -> Result<(), BundleError> {
        let shown = path.display();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|error| match error.kind() {
            ErrorKind::AlreadyExists => BundleError::Seed(format!(
                "refusing to overwrite {shown}: the seed file already exists"
            )),
            ErrorKind::NotFound => BundleError::Seed(format!(
                "creating the seed file {shown}: its directory does not exist"
            )),
            _ => BundleError::Seed(format!("creating the seed file {shown}: {error}")),
        })?;
        let mut text = encode_hex(&self.0);
        text.push('\n');
        let written = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        text.zeroize();
        if let Err(error) = written {
            drop(file);
            // The file is this call's own: a half-written seed is removed, not left behind.
            let _ = fs::remove_file(path);
            return Err(BundleError::Seed(format!(
                "writing the seed file {shown}: {error}"
            )));
        }
        Ok(())
    }

    /// The public key of this seed.
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        public_key_for(&self.0)
    }

    /// The key id: the SHA-256 of the 32 raw bytes of the public key.
    #[must_use]
    pub fn key_id(&self) -> [u8; 32] {
        Sha256::digest(raw_public(&self.public_key())).into()
    }

    /// The key id as 64 lowercase hex characters.
    #[must_use]
    pub fn key_id_hex(&self) -> String {
        encode_hex(&self.key_id())
    }

    /// Checks a stated key id (64 hex characters, either case, surrounding whitespace allowed)
    /// against this seed's key id.
    ///
    /// # Errors
    ///
    /// `Seed` when the stated id is not 64 hex characters, or is this seed itself (a key id is a
    /// digest of the public key; the seed is never echoed). `KeyIdMismatch`, naming both ids, when
    /// the stated id is another key's.
    pub fn check_key_id(&self, stated: &str) -> Result<(), BundleError> {
        let stated = stated.trim();
        if decode_hex(stated.as_bytes()).is_none() {
            return Err(BundleError::Seed(
                "the stated key id is not 64 hex characters: nothing was signed".into(),
            ));
        }
        let mut seed_hex = encode_hex(&self.0);
        let is_the_seed = stated.eq_ignore_ascii_case(&seed_hex);
        seed_hex.zeroize();
        if is_the_seed {
            return Err(BundleError::Seed(
                "the stated key id is this seed itself: a key id is the SHA-256 of the public key, never the seed; nothing was signed".into(),
            ));
        }
        let derived = self.key_id_hex();
        let stated = stated.to_ascii_lowercase();
        if stated != derived {
            return Err(BundleError::KeyIdMismatch { stated, derived });
        }
        Ok(())
    }
}

/// The bytes `slot.sig` signs for `slot_json`, the exact `slot.json` bytes: the tag, a u32 LE
/// version (1), a u32 LE length, then the manifest.
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

/// Signs `slot_json` (the exact bytes written to `slot.json`) with `seed` and returns the 108-byte
/// `slot.sig` record. When `stated_key_id` is given it is checked first ([`Seed::check_key_id`]),
/// and on a mismatch the refusal is returned and nothing is signed.
///
/// # Errors
///
/// As [`Seed::check_key_id`] when a key id is stated; `Malformed` as [`slot_message`].
pub fn sign_slot_record(
    seed: &Seed,
    slot_json: &[u8],
    stated_key_id: Option<&str>,
) -> Result<[u8; RECORD_LEN], BundleError> {
    if let Some(stated) = stated_key_id {
        seed.check_key_id(stated)?;
    }
    let message = slot_message(slot_json)?;
    let record = Record {
        algorithm: Algorithm::Ed25519,
        key_id: seed.key_id(),
        signature: sign_bytes(&message, &seed.0),
    };
    Ok(record.to_bytes())
}

/// The seed a seed file's content holds, or `None` for any other form.
fn decode_seed_file(content: &[u8]) -> Option<[u8; SEED_LEN]> {
    if content.len() == SEED_LEN {
        let mut bytes = [0u8; SEED_LEN];
        bytes.copy_from_slice(content);
        return Some(bytes);
    }
    decode_hex(content.trim_ascii())
}

/// Decodes exactly 64 hex characters (either case) into 32 bytes.
fn decode_hex(text: &[u8]) -> Option<[u8; SEED_LEN]> {
    if text.len() != 2 * SEED_LEN {
        return None;
    }
    let mut out = [0u8; SEED_LEN];
    let (pairs, _) = text.as_chunks::<2>();
    for (byte, pair) in out.iter_mut().zip(pairs) {
        *byte = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(out)
}

fn hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The 32 raw bytes of a public key. `atlas-release` keeps the key private and displays it as
/// `ed25519:<64 hex>`; a display that is not that shape is a broken invariant of that crate.
fn raw_public(key: &PublicKey) -> [u8; 32] {
    let text = key.to_string();
    let hex = text.strip_prefix(PUBLIC_PREFIX).unwrap_or(&text);
    decode_hex(hex.as_bytes()).expect("a public key displays as ed25519:<64 hex characters>")
}
