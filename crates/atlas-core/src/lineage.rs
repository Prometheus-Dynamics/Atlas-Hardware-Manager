//! One physical board, several identities. A Raspberry Pi based board is
//! `rpi:<boot ROM serial>` in USB boot and `raze:<full serial>` once its OS
//! runs. Drivers that know the board's own serial report it as the
//! `board_serial` attribute; when a board comes up running, the offline
//! recovery record of the same board is folded into it, so a flash does not
//! leave a device behind that "never comes back".

use atlas_driver::{DeviceKey, DeviceMode, attributes::BOARD_SERIAL};

use crate::activity::{ActivityEntry, ActivityKind, ActivityLevel};
use crate::atlas::Inner;
use crate::{DeviceRecord, Event, Presence};

/// Folds offline recovery records of the same board into `current`, a
/// running device. Keeps the user's label and robot. Returns the updated
/// record when anything was folded in.
pub(crate) fn absorb_recovery_records(
    inner: &Inner,
    current: &DeviceRecord,
) -> Option<DeviceRecord> {
    if current.identity.mode != DeviceMode::Normal {
        return None;
    }
    let board = current
        .identity
        .attributes
        .get(BOARD_SERIAL)?
        .to_ascii_lowercase();

    let (absorbed, updated) = {
        let mut state = inner.state();
        let stale: Vec<DeviceKey> = state
            .inventory
            .all()
            .into_iter()
            .filter(|record| {
                record.key != current.key
                    && record.presence == Presence::Offline
                    && record.identity.mode == DeviceMode::Recovery
                    && record
                        .identity
                        .attributes
                        .get(BOARD_SERIAL)
                        // Records saved before drivers reported a board
                        // serial: their own serial is the board serial.
                        .unwrap_or(&record.key.serial.0)
                        .eq_ignore_ascii_case(&board)
            })
            .map(|record| record.key)
            .collect();
        if stale.is_empty() {
            return None;
        }
        let absorbed: Vec<DeviceRecord> = stale
            .iter()
            .filter_map(|key| state.inventory.remove(key))
            .collect();
        let target = state.inventory.get_mut(&current.key)?;
        for old in &absorbed {
            if target.label.is_none() {
                target.label.clone_from(&old.label);
            }
            if target.robot.is_none() {
                target.robot.clone_from(&old.robot);
            }
        }
        (absorbed, target.clone())
    };

    for old in &absorbed {
        inner.events.emit(Event::DeviceForgotten {
            key: old.key.clone(),
        });
    }
    let running = updated
        .identity
        .primary_version()
        .map(|version| format!(" and runs {version}"))
        .unwrap_or_default();
    inner.record_activity(vec![ActivityEntry::about(
        &updated,
        ActivityKind::ModeChanged,
        ActivityLevel::Success,
        format!("{} left recovery{running}", updated.display_name()),
    )]);
    inner.events.emit(Event::DeviceSeen {
        record: Box::new(updated.clone()),
        new: false,
    });
    Some(updated)
}
