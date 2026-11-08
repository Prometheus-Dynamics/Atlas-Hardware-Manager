//! A board's SSH host key, checked against the keys its own Orion node
//! publishes.
//!
//! The keys SSH presents are read with `ssh-keyscan`, which proves nothing by
//! itself. The published fingerprints (`SHA256:<base64>`, as `ssh-keygen -l`
//! prints them) come from the board's node over an Orion session that is
//! authenticated to the node (`atlas_driver::HostKeySource`). A presented key
//! whose fingerprint matches is the board's; a different one is refused. With
//! no Orion link, or nothing published, the caller's own rules stand.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use atlas_driver::{DriverError, HostKeySource, PublishedHostKey};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use super::{SshUpdate, host_keys};

/// The most a key scan may take before the board counts as unreachable.
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);

/// What the app shows when a key matched what the board's node publishes.
pub(crate) const VERIFIED_NOTE: &str = "Verified over Orion: the board's SSH host key matches the key its Orion node publishes, so it is trusted.";

static SOURCE: RwLock<Option<Arc<dyn HostKeySource>>> = RwLock::new(None);

/// Where the keys a board publishes come from: the app's Orion, over an
/// authenticated session. Without one, no board is checked over Orion.
pub fn set_host_key_source(source: Arc<dyn HostKeySource>) {
    *SOURCE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(source);
}

fn source() -> Option<Arc<dyn HostKeySource>> {
    SOURCE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// A host key SSH presents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Presented {
    /// The SSH name: `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`.
    pub(crate) ssh_type: String,
    /// The Orion name: `ed25519`, `ecdsa`, `rsa`.
    pub(crate) kind: String,
    /// `SHA256:<base64>`.
    pub(crate) fingerprint: String,
    /// The key's base64 blob, as it is written in known_hosts.
    pub(crate) blob: String,
}

/// What the keys a board publishes say about the key SSH presents.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Nothing to compare: the caller keeps its own rules.
    NotPublished,
    /// The presented keys that match what the board publishes.
    Verified(Vec<Presented>),
    /// The key is not the board's: the reason to say.
    Refused(String),
}

/// Checks the keys SSH presents for `host` against those the board with
/// `serial` publishes over Orion.
pub(crate) async fn verdict(host: &str, serial: &str) -> Verdict {
    if serial.trim().is_empty() {
        return Verdict::NotPublished;
    }
    let Some(source) = source() else {
        return Verdict::NotPublished;
    };
    let published = source.published_host_keys(serial).await;
    if published.is_empty() {
        return Verdict::NotPublished;
    }
    match scan(host).await {
        Ok(presented) => compare(&published, &presented),
        Err(reason) => Verdict::Refused(format!(
            "Atlas couldn't read the board's SSH host key to check it against its Orion node ({reason}); nothing changed"
        )),
    }
}

impl SshUpdate {
    /// The "Trust its new SSH key" action. The key the board's Orion node
    /// publishes decides when it can: a match is pinned and reported as
    /// verified, a mismatch is refused. Otherwise the pin is dropped, as it
    /// always was, and the next connection pins what the board shows.
    pub(crate) async fn trust_new_key(&self) -> Result<Option<String>, DriverError> {
        match verdict(&self.host, self.serial()).await {
            Verdict::Refused(reason) => Err(DriverError::Incompatible(reason)),
            Verdict::Verified(keys) => self
                .pin_verified(&keys)
                .map(|()| Some(VERIFIED_NOTE.to_owned())),
            Verdict::NotPublished => self.forget_host_key().map(|()| None),
        }
    }

    /// Pins the keys Orion verified for this board in Atlas's known_hosts.
    pub(super) fn pin_verified(&self, keys: &[Presented]) -> Result<(), DriverError> {
        let config = self.access.get();
        let Some(known_hosts) = &config.known_hosts else {
            return Err(DriverError::Incompatible(
                "Atlas uses your own known_hosts for boards; remove the board's entry there".into(),
            ));
        };
        let pairs: Vec<(&str, &str)> = keys
            .iter()
            .map(|key| (key.ssh_type.as_str(), key.blob.as_str()))
            .collect();
        host_keys::pin(known_hosts, &self.alias(), &pairs).map_err(|error| {
            DriverError::Other(format!(
                "couldn't pin the board's host key in {}: {error}",
                known_hosts.display()
            ))
        })
    }

    /// The board's serial, as its Orion node publishes it ("" when unknown).
    pub(super) fn serial(&self) -> &str {
        self.usb_serial.as_deref().unwrap_or_default()
    }
}

/// The keys `host` shows, from `ssh-keyscan` (unauthenticated: compared, not
/// trusted, by the caller).
async fn scan(host: &str) -> Result<Vec<Presented>, String> {
    let output = tokio::time::timeout(
        SCAN_TIMEOUT,
        Command::new("ssh-keyscan")
            .args(["-T", "8", "-t", "ed25519,ecdsa,rsa", host])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "the board didn't answer".to_string())?
    .map_err(|error| format!("ssh-keyscan: {error}"))?;
    let keys: Vec<Presented> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_line)
        .collect();
    if keys.is_empty() {
        Err("the board showed no SSH host key".into())
    } else {
        Ok(keys)
    }
}

/// Compares presented keys with published ones. Any presented key of a
/// published kind whose fingerprint differs refuses the board; at least one
/// match is needed to verify it.
pub(crate) fn compare(published: &[PublishedHostKey], presented: &[Presented]) -> Verdict {
    let mut matched = Vec::new();
    let mut differs = Vec::new();
    for key in presented {
        for want in published.iter().filter(|want| want.kind == key.kind) {
            if want.fingerprint == key.fingerprint {
                matched.push(key.clone());
            } else {
                differs.push(key.kind.clone());
            }
        }
    }
    if !differs.is_empty() {
        return Verdict::Refused(format!(
            "the board's SSH host key ({}) isn't the one its Orion node publishes, so Atlas refuses it",
            differs.join(", ")
        ));
    }
    if matched.is_empty() {
        return Verdict::Refused(
            "the board's SSH host key matches none of the keys its Orion node publishes, so Atlas refuses it"
                .into(),
        );
    }
    Verdict::Verified(matched)
}

/// One `ssh-keyscan` line: `host ssh-type base64-blob` (comments and other
/// lines are skipped).
pub(crate) fn parse_line(line: &str) -> Option<Presented> {
    let mut parts = line.split_whitespace();
    let _host = parts.next()?;
    let ssh_type = parts.next()?;
    let blob = parts.next()?;
    let kind = kind_of(ssh_type)?;
    Some(Presented {
        ssh_type: ssh_type.to_owned(),
        kind: kind.to_owned(),
        fingerprint: fingerprint(blob)?,
        blob: blob.to_owned(),
    })
}

/// Orion's name for an SSH host key type.
fn kind_of(ssh_type: &str) -> Option<&'static str> {
    match ssh_type {
        "ssh-ed25519" => Some("ed25519"),
        "ssh-rsa" => Some("rsa"),
        other if other.starts_with("ecdsa-sha2-") => Some("ecdsa"),
        _ => None,
    }
}

/// `SHA256:<base64>` of a base64 key blob, as `ssh-keygen -l` prints it.
pub(crate) fn fingerprint(blob: &str) -> Option<String> {
    let bytes = STANDARD.decode(blob).ok()?;
    let digest = Sha256::digest(&bytes);
    Some(format!("SHA256:{}", STANDARD_NO_PAD.encode(&digest[..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway ed25519 public key (no private half kept), with the
    /// fingerprint `ssh-keygen -lf` prints for it.
    const ED25519_LINE: &str =
        "board ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMa4RE1aEJPXPyYE1vWjd4/PTP42Quon2sD51DlMsxYH";
    const ED25519_FINGERPRINT: &str = "SHA256:tY6lS67NY5z5kfAGsSmLMIEQLk9WTUCqso6gNjGeO4Y";

    fn published(kind: &str, fingerprint: &str) -> PublishedHostKey {
        PublishedHostKey {
            kind: kind.into(),
            fingerprint: fingerprint.into(),
        }
    }

    #[test]
    fn a_keyscan_line_gives_its_kind_and_fingerprint() {
        let key = parse_line(ED25519_LINE).expect("an ed25519 line");
        assert_eq!(key.ssh_type, "ssh-ed25519");
        assert_eq!(key.kind, "ed25519");
        assert_eq!(key.fingerprint, ED25519_FINGERPRINT);
        assert!(parse_line("# board:22 SSH-2.0-OpenSSH_9.2").is_none());
        assert!(parse_line("board ssh-dss AAAA").is_none());
    }

    #[test]
    fn a_matching_key_is_verified_and_a_different_one_refused() {
        let key = parse_line(ED25519_LINE).unwrap();
        let same = [published("ed25519", &key.fingerprint)];
        assert_eq!(
            compare(&same, std::slice::from_ref(&key)),
            Verdict::Verified(vec![key.clone()])
        );
        let other = [published("ed25519", "SHA256:someoneelse")];
        assert!(matches!(
            compare(&other, std::slice::from_ref(&key)),
            Verdict::Refused(_)
        ));
    }

    #[test]
    fn no_common_kind_is_refused_not_trusted() {
        let key = parse_line(ED25519_LINE).unwrap();
        let rsa_only = [published("rsa", "SHA256:x")];
        assert!(matches!(
            compare(&rsa_only, std::slice::from_ref(&key)),
            Verdict::Refused(_)
        ));
    }
}
