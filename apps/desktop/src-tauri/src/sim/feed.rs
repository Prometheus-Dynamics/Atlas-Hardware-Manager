//! The simulated release feed: what the app's own update reads where there is no network.
//!
//! It is a [`Fetch`] that answers one feed address and one installer address from memory. The
//! releases in it are signed here, with a key made up for the simulation (its secret is the
//! constant below, so it protects nothing and is trusted only in a simulated world's own key
//! list). `sim.feed` chooses what the feed holds: nothing newer, a newer release, a newer
//! release signed by a key nobody trusts, or a server that does not answer.

use std::sync::Mutex;

use async_trait::async_trait;
use atlas_release::{
    APP_FAMILY, Body, Fetch, ManifestArtifact, ManifestBody, Reply, Requirements, public_key_for,
    sign_manifest,
};
use base64::Engine;
use tokio::sync::watch;

/// The feed's address. Nothing is ever asked of it over a network.
pub const FEED_URL: &str = "https://feed.invalid/atlas/releases.json";
const INSTALLER_BASE: &str = "https://feed.invalid/atlas/";
/// The version of the simulated newer release: newer than any real one.
pub const NEWER: &str = "9999.1.0";
const SECRET: [u8; 32] = [7; 32];
const STRANGER: [u8; 32] = [9; 32];
/// The id the simulated key goes by in the trusted list.
pub const KEY_ID: &str = "0000000000005117";
const INSTALLER_BYTES: usize = 384 * 1024;
const PIECE: usize = 24 * 1024;

/// What the feed holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Holds {
    /// Only a release older than any build.
    #[default]
    NothingNewer,
    Newer,
    /// A newer release signed by a key nobody trusts.
    NewerByAStranger,
    /// The server does not answer.
    Down,
}

/// The simulated feed.
#[derive(Debug)]
pub struct SimFeed {
    holds: Mutex<Holds>,
    /// While set, an installer download stops after its first pieces and waits.
    hold: watch::Sender<bool>,
}

impl Default for SimFeed {
    fn default() -> Self {
        Self {
            holds: Mutex::new(Holds::default()),
            hold: watch::channel(false).0,
        }
    }
}

/// The installer's bytes: the same every time, and not all one value.
fn installer() -> Vec<u8> {
    (0..INSTALLER_BYTES)
        .map(|i| u8::try_from((i * 31 + i / 251) % 251).unwrap_or(0))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    use std::fmt::Write;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl SimFeed {
    pub fn set(&self, holds: Holds) {
        *self.holds.lock().unwrap_or_else(|e| e.into_inner()) = holds;
    }

    pub fn set_hold(&self, hold: bool) {
        self.hold.send_replace(hold);
    }

    /// The simulated key as an entry of a trusted-key file.
    #[must_use]
    pub fn key_file() -> String {
        let public = public_key_for(&SECRET).to_string();
        let hex = public.trim_start_matches("ed25519:");
        let key: Vec<u8> = (0..hex.len() / 2)
            .filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
            .collect();
        // A minisign public key: "Ed", the key id (little-endian), the ed25519 key.
        let id = u64::from_str_radix(KEY_ID, 16).unwrap_or(0).to_le_bytes();
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&id);
        raw.extend_from_slice(&key);
        serde_json::json!({ "keys": [{
            "name": "Simulated release key",
            "kind": "minisign",
            "key_id": KEY_ID,
            "public_key": base64::engine::general_purpose::STANDARD.encode(raw),
            "owner": "The simulated world",
            "contact": "sim@example.invalid",
            "purpose": "production",
            "created": "2026-11-01",
            "expires": null,
            "note": "Signs the simulated release feed. Its secret is public."
        }]})
        .to_string()
    }

    fn manifests(&self) -> Vec<u8> {
        let holds = *self.holds.lock().unwrap_or_else(|e| e.into_inner());
        let (version, secret) = match holds {
            Holds::NothingNewer | Holds::Down => ("0.1.0", SECRET),
            Holds::Newer => (NEWER, SECRET),
            Holds::NewerByAStranger => (NEWER, STRANGER),
        };
        let bytes = installer();
        let sha256 = sha256_hex(&bytes);
        let mut manifests = Vec::new();
        for channel in ["dev", "beta", "stable"] {
            for suffix in [".deb", ".rpm", ".AppImage"] {
                let name = format!("atlas-hardware-manager_{version}{suffix}");
                let body = ManifestBody {
                    manifest_version: 1,
                    family: APP_FAMILY.into(),
                    version: version.into(),
                    channel: channel.into(),
                    boards: Vec::new(),
                    artifact: ManifestArtifact {
                        url: format!("{INSTALLER_BASE}{name}"),
                        name,
                        size: bytes.len() as u64,
                        sha256: sha256.clone(),
                    },
                    requires: Requirements::default(),
                    methods: Vec::new(),
                    notes_url: None,
                };
                if let Ok(manifest) = sign_manifest(body, &secret) {
                    manifests.push(manifest);
                }
            }
        }
        serde_json::to_vec(&manifests).unwrap_or_default()
    }
}

struct Pieces {
    bytes: Vec<u8>,
    at: usize,
    sent: usize,
    hold: watch::Receiver<bool>,
}

#[async_trait]
impl Body for Pieces {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.at >= self.bytes.len() {
            return Ok(None);
        }
        // Held: the first pieces have arrived, the rest wait for the hold to be let go.
        if self.sent >= 4 {
            let _ = self.hold.wait_for(|held| !held).await;
        }
        let end = (self.at + PIECE).min(self.bytes.len());
        let piece = self.bytes[self.at..end].to_vec();
        self.at = end;
        self.sent += 1;
        Ok(Some(piece))
    }
}

#[async_trait]
impl Fetch for SimFeed {
    async fn get(&self, url: &str, offset: u64) -> Result<Reply, String> {
        if *self.holds.lock().unwrap_or_else(|e| e.into_inner()) == Holds::Down {
            return Err("connection refused".into());
        }
        let bytes = if url == FEED_URL {
            self.manifests()
        } else if url.starts_with(INSTALLER_BASE) {
            installer()
        } else {
            return Ok(Reply {
                status: 404,
                body: Box::new(Pieces {
                    bytes: Vec::new(),
                    at: 0,
                    sent: 0,
                    hold: self.hold.subscribe(),
                }),
            });
        };
        let at = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        Ok(Reply {
            status: if offset > 0 { 206 } else { 200 },
            body: Box::new(Pieces {
                bytes,
                at,
                sent: 0,
                hold: self.hold.subscribe(),
            }),
        })
    }
}
