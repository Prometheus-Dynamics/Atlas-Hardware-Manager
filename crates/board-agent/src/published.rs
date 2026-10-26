//! What board-agent publishes on its node's status lane: the writer's state
//! as the `update.*` keys, the claimed actions, the newest board event, and
//! how this boot began.

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
/// A slots board: `slots`; its default, previous and trial slot ids, the
/// free space on /data and the reserve kept, and every slot as JSON text
/// (the writer's list: id, family, version, variant, size, flags, pinned,
/// good, installed, last_boot, display_name).
pub const KEY_LAYOUT: &str = "update.layout";
pub const KEY_SLOT_DEFAULT: &str = "update.slot_default";
pub const KEY_SLOT_PREVIOUS: &str = "update.slot_previous";
pub const KEY_SLOT_TRIAL: &str = "update.slot_trial";
pub const KEY_FREE_BYTES: &str = "update.free_bytes";
pub const KEY_RESERVE_BYTES: &str = "update.reserve_bytes";
pub const KEY_SLOTS: &str = "update.slots";

/// How this boot began, from the boot record (`/run/board/boot.json`) and
/// the kernel logs kept for it (`/data/board/pstore/<boot id>/`): the boot
/// count, `power-on` or `reset`, whether the boot before shut down cleanly
/// (`true`, `false`, empty: unknown), and the kernel log this boot found of
/// the one before (`panic` with a panic or oops record, `console` with only
/// its console, empty: none). Atlas opens the log over SSH (pstore --show).
pub const KEY_BOOT_COUNT: &str = "update.boot_count";
pub const KEY_BOOT_RESET: &str = "update.boot_reset";
pub const KEY_BOOT_PREVIOUS_CLEAN: &str = "update.boot_previous_clean";
pub const KEY_BOOT_KERNEL_LOG: &str = "update.boot_kernel_log";

/// This boot's record, as published under the `update.boot_*` keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootRecord {
    pub count: u64,
    pub reset: String,
    pub previous_clean: String,
    pub kernel_log: String,
}

impl BootRecord {
    /// From `boot.json` in `run_dir` and the pstore directory for `boot_id`
    /// under `data_dir`; `None` before the boot record is written.
    pub fn read(
        run_dir: &std::path::Path,
        data_dir: &std::path::Path,
        boot_id: &str,
    ) -> Option<Self> {
        let text = std::fs::read_to_string(run_dir.join("boot.json")).ok()?;
        let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
        let kernel_log = if boot_id.is_empty() {
            ""
        } else {
            match std::fs::read_dir(data_dir.join("pstore").join(boot_id)) {
                Ok(entries) => {
                    let names: Vec<String> = entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect();
                    if names.iter().any(|name| name.starts_with("dmesg-")) {
                        "panic"
                    } else if names.is_empty() {
                        ""
                    } else {
                        "console"
                    }
                }
                Err(_) => "",
            }
        };
        Some(Self {
            count: value
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            reset: value
                .get("reset")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            previous_clean: value
                .get("previous_clean")
                .and_then(serde_json::Value::as_bool)
                .map(|clean| clean.to_string())
                .unwrap_or_default(),
            kernel_log: kernel_log.to_owned(),
        })
    }
}

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
    boot: Option<&BootRecord>,
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
    if status.layout == "slots" {
        entries.extend([
            reporter.node_status_entry(KEY_LAYOUT, text(&status.layout)),
            reporter.node_status_entry(KEY_SLOT_DEFAULT, text(&status.slot_default)),
            reporter.node_status_entry(KEY_SLOT_PREVIOUS, text(&status.slot_previous)),
            reporter.node_status_entry(KEY_SLOT_TRIAL, text(&status.slot_trial)),
            reporter.node_status_entry(KEY_FREE_BYTES, TypedConfigValue::UInt(status.free_bytes)),
            reporter.node_status_entry(
                KEY_RESERVE_BYTES,
                TypedConfigValue::UInt(status.reserve_bytes),
            ),
            reporter.node_status_entry(KEY_SLOTS, text(&status.slots.to_string())),
        ]);
    }
    if let Some(event) = last_event {
        entries.extend([
            reporter.node_status_entry(KEY_EVENT_SEQ, TypedConfigValue::UInt(event.seq)),
            reporter.node_status_entry(KEY_EVENT_KIND, text(&event.kind)),
            reporter.node_status_entry(KEY_EVENT_T, TypedConfigValue::Int(event.t)),
        ]);
    }
    if let Some(boot) = boot {
        entries.extend([
            reporter.node_status_entry(KEY_BOOT_COUNT, TypedConfigValue::UInt(boot.count)),
            reporter.node_status_entry(KEY_BOOT_RESET, text(&boot.reset)),
            reporter.node_status_entry(KEY_BOOT_PREVIOUS_CLEAN, text(&boot.previous_clean)),
            reporter.node_status_entry(KEY_BOOT_KERNEL_LOG, text(&boot.kernel_log)),
        ]);
    }
    entries
}

/// Whether the node refused a publish because the agent has no claims
/// there (Orion: "clients may publish ... only `action.*` keys and ... keys
/// of node actions they claimed").
pub(crate) fn refused_for_claims(error: &str) -> bool {
    error.contains("authorization failed") && error.contains("claimed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boot_record_and_its_kernel_log() {
        let dir = std::env::temp_dir().join(format!("board-agent-boot-{}", std::process::id()));
        let (run, data) = (dir.join("run"), dir.join("data"));
        std::fs::create_dir_all(data.join("pstore/b-2")).unwrap();
        std::fs::create_dir_all(&run).unwrap();
        assert_eq!(BootRecord::read(&run, &data, "b-2"), None);
        std::fs::write(
            run.join("boot.json"),
            r#"{"id":"b-2","count":7,"previous_clean":false,"reset":"power-on","rsts":"0x1000"}"#,
        )
        .unwrap();
        let read = || BootRecord::read(&run, &data, "b-2").unwrap();
        assert_eq!(read().kernel_log, "");
        std::fs::write(data.join("pstore/b-2/console-ramoops-0"), "x").unwrap();
        assert_eq!(read().kernel_log, "console");
        std::fs::write(data.join("pstore/b-2/dmesg-ramoops-0"), "x").unwrap();
        assert_eq!(
            read(),
            BootRecord {
                count: 7,
                reset: "power-on".into(),
                previous_clean: "false".into(),
                kernel_log: "panic".into(),
            }
        );
        std::fs::write(
            run.join("boot.json"),
            r#"{"count":1,"previous_clean":null,"reset":null}"#,
        )
        .unwrap();
        let unknown = read();
        assert_eq!(
            (unknown.reset.as_str(), unknown.previous_clean.as_str()),
            ("", "")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
