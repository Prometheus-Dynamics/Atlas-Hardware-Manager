use atlas_types::{CapabilitySet, DeviceKey, Identity, LinkId, LinkKind};
use serde::{Deserialize, Serialize};

use crate::history::{HISTORY_LIMIT, HistoryEntry, HistoryKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Presence {
    Online,
    Offline,
}

/// One link a device was seen through, and when it last answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSighting {
    pub link: LinkId,
    pub kind: LinkKind,
    pub address: String,
    pub last_seen_ms: u64,
}

/// Everything known about one board, online or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub key: DeviceKey,
    /// The identity from the sighting on the link the record follows.
    pub identity: Identity,
    pub capabilities: CapabilitySet,
    pub presence: Presence,
    /// Every link the device was seen through, oldest first.
    #[serde(default)]
    pub links: Vec<LinkSighting>,
    pub first_seen_ms: u64,
    /// The last time any link answered.
    pub last_seen_ms: u64,
    /// A name the user gave the device.
    #[serde(default)]
    pub label: Option<String>,
    /// The robot this device belongs to, by name.
    #[serde(default)]
    pub robot: Option<String>,
    /// Oldest first, at most [`HISTORY_LIMIT`] entries.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

impl DeviceRecord {
    /// The user's label, then the device's own name, then its key.
    pub fn display_name(&self) -> String {
        self.label
            .clone()
            .or_else(|| self.identity.name.clone())
            .unwrap_or_else(|| self.key.to_string())
    }

    /// The version shown for the device, from its identity.
    pub fn version(&self) -> Option<&str> {
        self.identity.primary_version()
    }

    /// The link the record follows.
    pub fn followed_link(&self) -> &LinkId {
        &self.identity.link
    }

    pub(crate) fn new(
        identity: Identity,
        kind: LinkKind,
        capabilities: CapabilitySet,
        now_ms: u64,
    ) -> Self {
        let mut record = Self {
            key: identity.key.clone(),
            identity,
            capabilities,
            presence: Presence::Online,
            links: Vec::new(),
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            label: None,
            robot: None,
            history: Vec::new(),
        };
        let (link, address) = (
            record.identity.link.clone(),
            record.identity.address.clone(),
        );
        record.note_link(&link, kind, &address, now_ms);
        record.push_history(now_ms, HistoryKind::Found, format!("found on {link}"));
        record
    }

    /// Records that `link` answered now. One sighting per link: the latest
    /// address and time replace the older ones.
    pub(crate) fn note_link(&mut self, link: &LinkId, kind: LinkKind, address: &str, now_ms: u64) {
        match self
            .links
            .iter_mut()
            .find(|sighting| &sighting.link == link)
        {
            Some(sighting) => {
                sighting.kind = kind;
                sighting.address = address.to_string();
                sighting.last_seen_ms = now_ms;
            }
            None => self.links.push(LinkSighting {
                link: link.clone(),
                kind,
                address: address.to_string(),
                last_seen_ms: now_ms,
            }),
        }
    }

    /// Whether a sighting on `link` should become the followed link.
    ///
    /// The record keeps its followed link while that link is live. A
    /// sighting takes over when the followed link has been silent for
    /// [`LINK_SILENT_MS`], or when the new link ranks better.
    pub(crate) fn takes_over(&self, link: &LinkId, kind: &LinkKind, now_ms: u64) -> bool {
        if self.followed_link() == link {
            return true;
        }
        let Some(followed) = self.links.iter().find(|s| &s.link == self.followed_link()) else {
            return true;
        };
        now_ms.saturating_sub(followed.last_seen_ms) > LINK_SILENT_MS
            || link_rank(kind) < link_rank(&followed.kind)
    }

    /// Takes in the links, history and details of a record folded into this
    /// one. This record's identity and capabilities stay.
    pub(crate) fn absorb(&mut self, old: DeviceRecord) {
        for sighting in old.links {
            match self
                .links
                .iter_mut()
                .find(|mine| mine.link == sighting.link)
            {
                Some(mine) if mine.last_seen_ms >= sighting.last_seen_ms => {}
                Some(mine) => *mine = sighting,
                None => self.links.push(sighting),
            }
        }
        if old.presence == Presence::Online {
            self.presence = Presence::Online;
        }
        self.first_seen_ms = self.first_seen_ms.min(old.first_seen_ms);
        self.last_seen_ms = self.last_seen_ms.max(old.last_seen_ms);
        if self.label.is_none() {
            self.label = old.label;
        }
        if self.robot.is_none() {
            self.robot = old.robot;
        }
        let mut merged = std::mem::take(&mut self.history);
        merged.extend(old.history);
        // Stable: entries of one instant keep their order.
        merged.sort_by_key(|entry| entry.at_ms);
        let excess = merged.len().saturating_sub(HISTORY_LIMIT);
        merged.drain(..excess);
        self.history = merged;
    }

    pub(crate) fn push_history(&mut self, at_ms: u64, kind: HistoryKind, message: String) {
        self.history.push(HistoryEntry {
            at_ms,
            kind,
            message,
        });
        let excess = self.history.len().saturating_sub(HISTORY_LIMIT);
        self.history.drain(..excess);
    }
}

/// A link silent for longer than this (ms) stops being preferred over a
/// worse one that answers.
pub(crate) const LINK_SILENT_MS: u64 = 30_000;

/// Preference when a device answers on several links: lower is better.
/// Direct links come first; a path through a controller is later.
pub(crate) fn link_rank(kind: &LinkKind) -> u8 {
    match kind {
        LinkKind::UsbNetwork => 0,
        LinkKind::Ethernet => 1,
        LinkKind::UsbSerial => 2,
        LinkKind::Gateway { .. } => 3,
        LinkKind::UsbBoot => 4,
        LinkKind::Simulated => 5,
    }
}
