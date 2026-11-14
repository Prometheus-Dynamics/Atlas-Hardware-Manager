//! The PC user's one client identity: the Ed25519 secret every Atlas session of this user signs
//! with. It is made once and kept in the user's Atlas config directory, mode 0600, never in a
//! repository. A board knows this PC by its public key; enrolment (`enrol`) asks the board to know
//! it.
//!
//! The file holds the 32-byte secret as 64 hex characters and a newline, the same text form as the
//! signing seed (`atlas-bundle`'s `Seed`). That type keeps its bytes to itself, so this module
//! reads and writes the form itself, under the same rules: a file readable by its group or others
//! is refused (`chmod 600`), a damaged file is refused and left as it is, and nothing is replaced.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use orion::Identity;
use sha2::{Digest, Sha256};

use crate::report::Report;

/// The file's name in the Atlas config directory.
pub const FILE_NAME: &str = "identity.key";
/// The directory under the platform's config directory that holds the file.
const DIR_NAME: &str = "atlas";
/// 64 hex characters, a newline, and room for surrounding whitespace; a larger file is refused unread.
const FILE_LIMIT: u64 = 256;

/// The identity as the file gives it.
#[derive(Debug)]
pub struct Loaded {
    /// The 32-byte Ed25519 secret. It is what every session signs with.
    pub secret: [u8; 32],
    /// Whether this call made the file.
    pub created: bool,
}

/// Where the identity file is when `--identity` is not given: `<config>/atlas/identity.key`, where
/// `<config>` is `$XDG_CONFIG_HOME` (else `~/.config`) on Linux, `~/Library/Application Support` on
/// macOS and `%APPDATA%` on Windows. `None` when the variable that names the base is not set.
#[must_use]
pub fn default_path(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
    app_data: Option<OsString>,
) -> Option<PathBuf> {
    let non_empty = |value: Option<OsString>| value.filter(|value| !value.is_empty());
    let base = if cfg!(windows) {
        PathBuf::from(non_empty(app_data)?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(non_empty(home)?).join("Library/Application Support")
    } else {
        match non_empty(xdg_config_home) {
            Some(xdg) => PathBuf::from(xdg),
            None => PathBuf::from(non_empty(home)?).join(".config"),
        }
    };
    Some(base.join(DIR_NAME).join(FILE_NAME))
}

/// Reads the identity at `path`, or makes it when the file is absent. A file that is there but
/// cannot be used is refused in words that name the path and the rule; it is never replaced.
///
/// # Errors
///
/// Words for the person when the file has a mode other than owner-only, has the wrong form, or
/// cannot be read or written. The words never quote the file's content.
pub fn load_or_create(path: &Path) -> Result<Loaded, String> {
    match read(path) {
        Ok(Some(secret)) => Ok(Loaded {
            secret,
            created: false,
        }),
        Ok(None) => create(path),
        Err(why) => Err(why),
    }
}

/// The identity's public key, as the board sees it.
#[must_use]
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    Identity::from_secret(secret).public()
}

/// A key's fingerprint in the form `next` prints for every node: `sha256:` and 32 hex digits, the
/// first 16 bytes of the SHA-256 of the key. This is `orion_identity::fingerprint` at the pinned
/// Orion rev (`crates/core/identity/src/lib.rs`); a test checks it against a simulated node's own.
#[must_use]
pub fn fingerprint(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    let mut out = String::from("sha256:");
    for byte in &digest[..16] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Reads the file: `Ok(None)` when it does not exist.
fn read(path: &Path) -> Result<Option<[u8; 32]>, String> {
    let shown = path.display();
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("reading the identity file {shown}: {error}")),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|error| format!("reading the identity file {shown}: {error}"))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "the identity file {shown} has mode {mode:o}, so it is readable by others than its owner: chmod 600 it"
            ));
        }
    }
    let mut content = Vec::new();
    (&mut file)
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut content)
        .map_err(|error| format!("reading the identity file {shown}: {error}"))?;
    let decoded = if content.len() as u64 > FILE_LIMIT {
        None
    } else {
        decode(&content)
    };
    match decoded {
        Some(secret) => Ok(Some(secret)),
        None => Err(format!(
            "the identity file {shown} must hold 64 hex characters and a newline; it holds something else, so it is refused and left as it is. Nothing was replaced."
        )),
    }
}

/// Makes the file: a new secret, written with mode 0600 into a directory made 0700 if it is missing.
/// An existing file is never overwritten: if one appears in the meantime it is read instead.
fn create(path: &Path) -> Result<Loaded, String> {
    let shown = path.display();
    let parent = path
        .parent()
        .ok_or_else(|| format!("the identity file {shown} has no directory"))?;
    make_private_dir(parent)?;
    let secret = Identity::generate().secret();
    let mut text = encode(&secret);
    text.push('\n');
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return read(path)?.map_or_else(
                || Err(format!("the identity file {shown} cannot be made")),
                |secret| {
                    Ok(Loaded {
                        secret,
                        created: false,
                    })
                },
            );
        }
        Err(error) => return Err(format!("creating the identity file {shown}: {error}")),
    };
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        drop(file);
        // The file is this call's own: a half-written secret is removed, not left behind.
        let _ = fs::remove_file(path);
        return Err(format!("writing the identity file {shown}: {error}"));
    }
    Ok(Loaded {
        secret,
        created: true,
    })
}

/// Makes `dir` and its missing parents, owner-only. A directory that exists is left as it is.
fn make_private_dir(dir: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|error| format!("the directory {} cannot be made: {error}", dir.display()))
}

/// The 64 hex characters of a file's content, with optional surrounding whitespace.
fn decode(content: &[u8]) -> Option<[u8; 32]> {
    let text = std::str::from_utf8(content).ok()?.trim();
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        let high = hex_value(bytes[2 * index])?;
        let low = hex_value(bytes[2 * index + 1])?;
        *byte = (high << 4) | low;
    }
    Some(out)
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

fn encode(secret: &[u8; 32]) -> String {
    secret.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The `identity` command: makes the identity if it is absent (saying so), and prints its
/// fingerprint and its path. A second run prints the same fingerprint.
pub fn run<W: Write>(path: &Path, json: bool, out: &mut W) -> i32 {
    let mut r = Report::new(out, json);
    match load_or_create(path) {
        Ok(loaded) => {
            let fingerprint = fingerprint(&public_key(&loaded.secret));
            let shown = path.display().to_string();
            if loaded.created {
                r.say(format!(
                    "created this PC's client identity at {shown}: it is made once and never replaced"
                ));
            }
            r.say(format!("identity {fingerprint}"));
            r.say(format!("file {shown}"));
            r.field("fingerprint", fingerprint);
            r.field("file", shown);
            r.field("created", loaded.created);
            r.finish(0)
        }
        Err(why) => {
            r.say(why);
            r.finish(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_text_is_64_hex_and_reads_back() {
        let secret: [u8; 32] = std::array::from_fn(|index| index as u8 * 7);
        let text = encode(&secret);
        assert_eq!(text.len(), 64);
        assert_eq!(decode(format!("{text}\n").as_bytes()), Some(secret));
        assert_eq!(decode(format!("  {text}  \n").as_bytes()), Some(secret));
        assert_eq!(decode(&text.as_bytes()[..63]), None);
        assert_eq!(decode(format!("{text}0").as_bytes()), None);
        assert_eq!(decode(format!("{}zz", &text[..62]).as_bytes()), None);
    }

    #[test]
    fn default_path_follows_the_config_home() {
        let path = default_path(
            Some("/cfg".into()),
            Some("/home/u".into()),
            Some("/app".into()),
        );
        if cfg!(windows) {
            assert_eq!(path, Some(PathBuf::from("/app/atlas/identity.key")));
        } else if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                Some(PathBuf::from(
                    "/home/u/Library/Application Support/atlas/identity.key"
                ))
            );
        } else {
            assert_eq!(path, Some(PathBuf::from("/cfg/atlas/identity.key")));
        }
        assert_eq!(default_path(None, None, None), None);
    }

    #[test]
    fn the_fingerprint_has_orions_form() {
        let fp = fingerprint(&[0_u8; 32]);
        assert!(fp.starts_with("sha256:"));
        assert_eq!(fp.len(), "sha256:".len() + 32);
    }
}
