use serde::{Deserialize, Serialize};

use crate::inventory::PowerCycle;
use crate::record::{DeviceRecord, Presence};

/// How long a device may be absent, with no robot power cycle to explain it,
/// before the absence is a fault (ms).
pub const FAULT_AFTER_MS: u64 = 5 * 60 * 1000;
/// A power cycle explains an absence that began this long before it started
/// (ms): the device went quiet as the robot was cut.
pub const POWER_CYCLE_LEAD_MS: u64 = 60 * 1000;
/// A device may take this long after a power cycle ends to come back (ms).
/// Past that, an absence is a fault again.
pub const RETURN_GRACE_MS: u64 = 5 * 60 * 1000;

/// A reading of one device's health at one time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    /// Online on some link.
    Ok,
    /// Offline, but not a fault: recently gone, or gone with its robot's
    /// power cycle.
    Away,
    /// Offline for longer than [`FAULT_AFTER_MS`], and no power cycle
    /// explains it.
    Fault,
}

pub(crate) fn health_of(record: &DeviceRecord, cycles: &[PowerCycle], now_ms: u64) -> Health {
    if record.presence == Presence::Online {
        return Health::Ok;
    }
    if explained_by_power_cycle(record, cycles, now_ms) {
        return Health::Away;
    }
    if now_ms.saturating_sub(record.last_seen_ms) <= FAULT_AFTER_MS {
        Health::Away
    } else {
        Health::Fault
    }
}

/// Whether the absence began during a power cycle of the device's robot (or
/// just before it), and the device has had time to come back since.
fn explained_by_power_cycle(record: &DeviceRecord, cycles: &[PowerCycle], now_ms: u64) -> bool {
    let quiet_from = record.last_seen_ms;
    cycles.iter().any(|cycle| {
        let began_in_cycle = cycle.started_ms.saturating_sub(POWER_CYCLE_LEAD_MS) <= quiet_from;
        match cycle.ended_ms {
            None => began_in_cycle && cycle.started_ms <= now_ms,
            Some(ended) => {
                began_in_cycle
                    && quiet_from <= ended
                    && now_ms <= ended.saturating_add(RETURN_GRACE_MS)
            }
        }
    })
}
