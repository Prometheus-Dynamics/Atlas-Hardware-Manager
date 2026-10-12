//! One physical board, several identities. A Raspberry Pi based board is
//! `rpi:<boot ROM serial>` in USB boot and `raze:<full serial>` once its OS
//! runs. Drivers that know the board's own serial report it as the
//! `board_serial` attribute; when a board comes up running, the offline
//! recovery record of the same board is folded into it, so a flash does not
//! leave a device behind that "never comes back".

use std::collections::BTreeSet;

use atlas_driver::attributes::{BOARD_SERIAL, normalize_board_serial};
use atlas_driver::{DeviceKey, DeviceMode};

use crate::activity::{ActivityEntry, ActivityKind, ActivityLevel};
use crate::atlas::Inner;
use crate::{DeviceRecord, Event, Presence};

/// The physical board a record is: its `board_serial`, else (records saved
/// before drivers reported one) its own serial by the same rule.
fn board_of(record: &DeviceRecord) -> Option<String> {
    record
        .identity
        .attributes
        .get(BOARD_SERIAL)
        .map(|serial| serial.to_ascii_lowercase())
        .or_else(|| normalize_board_serial(&record.key.serial.0))
}

/// Tidies the recovery records a flash leaves behind. Each offline recovery
/// record (boot ROM, eMMC-as-disk) of a board Atlas also knows running, or
/// sees in recovery under another key right now, is folded into that one,
/// whether or not the running board is online: it is the same camera. A
/// recovery record keyed by its USB port (`port-…`: a boot ROM that reports
/// no serial) is only ever "a board on this port", so once offline it is
/// dropped unless the user named it. Records a running job is using are left alone.
/// Returns whether anything changed.
pub(crate) fn fold_recovery_records(inner: &Inner, seen: &BTreeSet<DeviceKey>) -> bool {
    struct Fold {
        old: DeviceRecord,
        into: Option<DeviceKey>,
    }
    let (folds, updated) = {
        let mut state = inner.state();
        let in_jobs: BTreeSet<DeviceKey> = state
            .jobs
            .values()
            .flat_map(|job| job.devices.iter())
            .filter(|device| !device.status.is_finished())
            .map(|device| device.device.clone())
            .collect();
        let records = state.inventory.all();
        let mut folds = Vec::new();
        for old in records.iter().filter(|record| {
            record.presence == Presence::Offline
                && record.identity.mode == DeviceMode::Recovery
                && !in_jobs.contains(&record.key)
        }) {
            let Some(board) = board_of(old) else {
                let by_port = old.key.serial.0.starts_with("port-");
                if by_port && old.label.is_none() && old.robot.is_none() {
                    folds.push(Fold {
                        old: old.clone(),
                        into: None,
                    });
                }
                continue;
            };
            // Running first (online, then the most recently seen), then a
            // recovery record of the same board that is online now.
            let target = records
                .iter()
                .filter(|other| other.key != old.key && board_of(other).as_deref() == Some(&board))
                .filter(|other| {
                    other.identity.mode == DeviceMode::Normal || other.presence == Presence::Online
                })
                .max_by_key(|other| {
                    (
                        other.identity.mode == DeviceMode::Normal,
                        other.presence == Presence::Online,
                        other.last_seen_ms,
                    )
                });
            if let Some(target) = target {
                folds.push(Fold {
                    old: old.clone(),
                    into: Some(target.key.clone()),
                });
            }
        }
        if folds.is_empty() {
            return false;
        }
        let mut updated: Vec<DeviceRecord> = Vec::new();
        for fold in &folds {
            state.inventory.remove(&fold.old.key);
            let Some(into) = &fold.into else { continue };
            let Some(target) = state.inventory.get_mut(into) else {
                continue;
            };
            if target.label.is_none() {
                target.label.clone_from(&fold.old.label);
            }
            if target.robot.is_none() {
                target.robot.clone_from(&fold.old.robot);
            }
            updated.retain(|record| &record.key != into);
            updated.push(target.clone());
        }
        (folds, updated)
    };

    for fold in &folds {
        inner.events.emit(Event::DeviceForgotten {
            key: fold.old.key.clone(),
        });
    }
    let mut history = Vec::new();
    for record in &updated {
        // Said once, when the board shows up running after a flash.
        if record.identity.mode == DeviceMode::Normal && seen.contains(&record.key) {
            let running = record
                .identity
                .primary_version()
                .map(|version| format!(" and runs {version}"))
                .unwrap_or_default();
            history.push(ActivityEntry::about(
                record,
                ActivityKind::ModeChanged,
                ActivityLevel::Success,
                format!("{} left recovery{running}", record.display_name()),
            ));
        }
        inner.events.emit(Event::DeviceSeen {
            record: Box::new(record.clone()),
            new: false,
        });
    }
    inner.record_activity(history);
    true
}
