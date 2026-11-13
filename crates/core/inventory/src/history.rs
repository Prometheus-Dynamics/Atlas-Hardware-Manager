use serde::{Deserialize, Serialize};

/// Entries kept per device. The oldest go first.
pub const HISTORY_LIMIT: usize = 200;

/// What happened to a device, for its history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryKind {
    /// First sighting.
    Found,
    /// Seen again after being absent.
    Returned,
    /// Not seen on any link in a scan.
    Absent,
    /// The record now follows a different link.
    LinkChanged,
    /// Its identity changed, such as its version.
    Changed,
    /// Joined, left or was renamed in a robot.
    Robot,
    /// Its robot's power cycle started or ended.
    PowerCycle,
    /// The user changed its label.
    Label,
    /// Another key for this board was folded into it, or unfolded again.
    Folded,
}

/// One line of a device's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at_ms: u64,
    pub kind: HistoryKind,
    pub message: String,
}
