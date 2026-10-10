//! What board-agent publishes on its node's status lane: the writer's state
//! as the `update.*` keys, the claimed actions, and the newest board event.

use orion_client::ActionReporter;
use orion_control_plane::{StatusEntry, TypedConfigValue, update_action};

use crate::agent::{CLAIMED_ACTIONS, CLAIMED_KEY};
use crate::writer::WriterStatus;

/// Who started the update in progress or last finished, and what a
/// rollback would go back to (both from update.json).
pub const KEY_STARTED_BY: &str = "update.started_by";
pub const KEY_VERSION_PREVIOUS: &str = "update.version_previous";
/// While the state is `trying`: empty before the trial's checks start,
/// `checking` while they run, `failed` once they failed (update.json
/// `phase`).
pub const KEY_PHASE: &str = "update.phase";
/// The newest event in the board's log (any kind): its seq, kind and time
/// (the board's clock), so an Orion-only observer sees something happened.
/// Under `update.`: Orion lets an agent publish only `action.*` and the keys
/// of the node actions it claims.
pub const KEY_EVENT_SEQ: &str = "update.event_seq";
pub const KEY_EVENT_KIND: &str = "update.event_kind";
pub const KEY_EVENT_T: &str = "update.event_t";

/// The newest numbered event in the board's log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastEvent {
    pub seq: u64,
    pub kind: String,
    pub t: i64,
}

impl LastEvent {
    /// From the log's last line (lines are written in seq order).
    pub fn read(log: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(log).ok()?;
        let line = text.lines().rev().find(|line| !line.trim().is_empty())?;
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        Some(Self {
            seq: value.get("seq")?.as_u64()?,
            kind: value.get("kind")?.as_str()?.to_owned(),
            t: value
                .get("t")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
        })
    }
}

/// The `update.*` entries for `status` under
/// `node/<id>` (TTL 0: the node's maximum).
pub fn update_status_entries(
    reporter: &ActionReporter,
    status: &WriterStatus,
    boot_id: &str,
    last_event: Option<&LastEvent>,
) -> Vec<StatusEntry> {
    let text = |value: &str| TypedConfigValue::String(value.to_owned());
    let state = if status.state.is_empty() {
        update_action::STATE_IDLE
    } else {
        status.state.as_str()
    };
    let mut entries = vec![
        reporter.node_status_entry(update_action::KEY_STATE, text(state)),
        reporter.node_status_entry(update_action::KEY_SLOT_ACTIVE, text(&status.slot_active)),
        reporter.node_status_entry(update_action::KEY_SLOT_STAGED, text(&status.slot_staged)),
        reporter.node_status_entry(
            update_action::KEY_VERSION_ACTIVE,
            text(&status.version_active),
        ),
        reporter.node_status_entry(
            update_action::KEY_VERSION_STAGED,
            text(&status.version_staged),
        ),
        reporter.node_status_entry(
            update_action::KEY_PROGRESS,
            TypedConfigValue::UInt(status.progress.min(1000)),
        ),
        reporter.node_status_entry(update_action::KEY_ERROR, text(&status.error)),
        reporter.node_status_entry(update_action::KEY_BOOT_ID, text(boot_id)),
        reporter.node_status_entry(KEY_STARTED_BY, text(&status.started_by)),
        reporter.node_status_entry(KEY_VERSION_PREVIOUS, text(&status.version_previous)),
        reporter.node_status_entry(KEY_PHASE, text(&status.phase)),
        reporter.node_status_entry(CLAIMED_KEY, text(&CLAIMED_ACTIONS.join(","))),
    ];
    if let Some(event) = last_event {
        entries.extend([
            reporter.node_status_entry(KEY_EVENT_SEQ, TypedConfigValue::UInt(event.seq)),
            reporter.node_status_entry(KEY_EVENT_KIND, text(&event.kind)),
            reporter.node_status_entry(KEY_EVENT_T, TypedConfigValue::Int(event.t)),
        ]);
    }
    entries
}
