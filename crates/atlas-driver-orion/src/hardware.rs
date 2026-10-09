//! The board's hardware, from the `lemnos.device` resources that Lemnos's
//! Orion bridge publishes. A resource's labels describe the device (class,
//! model, its channels' units, its controls); its status lane carries its
//! state, its reason and one entry per channel.

use atlas_driver::{HardwareControl, HardwareDevice, HardwareReading, HardwareSnapshot};
use orion_control_plane::{
    AvailabilityState, HealthState, ResourceRecord, StatusEntry, TypedConfigValue, split_label,
};

use crate::status::text;

/// The resource type the bridge uses for every board device.
pub(crate) const DEVICE_TYPE: &str = "lemnos.device";

/// Status-lane keys that aren't channel readings.
const STATUS_KEY: &str = "status";
const REASON_KEY: &str = "reason";
const READ_US_KEY: &str = "read_us";
const CONTROL_PREFIX: &str = "control.";

/// The value of a `key=value` label, by its full key (`lemnos.class`).
fn label<'a>(record: &'a ResourceRecord, key: &str) -> Option<&'a str> {
    record.labels.iter().find_map(|label| {
        let (name, value) = split_label(label)?;
        (name == key).then(|| value.unwrap_or_default())
    })
}

fn label_or_empty(record: &ResourceRecord, key: &str) -> String {
    label(record, key).unwrap_or_default().to_string()
}

/// The device's id on its board: the resource id without `lemnos.<board>.`.
pub(crate) fn device_id(record: &ResourceRecord) -> String {
    let id = record.resource_id.as_str();
    label(record, "lemnos.board")
        .filter(|board| !board.is_empty())
        .and_then(|board| id.strip_prefix(&format!("lemnos.{board}.")))
        .unwrap_or(id)
        .to_string()
}

/// The status from Orion's availability and health, for a resource that
/// doesn't publish its own `status` entry.
fn status_from_availability(record: &ResourceRecord) -> &'static str {
    match (&record.availability, &record.health) {
        (
            AvailabilityState::Available | AvailabilityState::Busy,
            HealthState::Healthy | HealthState::Unknown,
        ) => "available",
        (AvailabilityState::Available | AvailabilityState::Busy, HealthState::Degraded) => {
            "degraded"
        }
        (_, HealthState::Failed) => "faulted",
        _ => "missing",
    }
}

/// A control's range label, `<min>..<max>` with the unit after a space when
/// it has one (`0..1`, `2..16 g`).
fn parse_range(text: &str) -> (Option<f64>, Option<f64>, String) {
    let (range, unit) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
    let Some((min, max)) = range.split_once("..") else {
        return (None, None, unit.trim().to_string());
    };
    (
        min.trim().parse().ok(),
        max.trim().parse().ok(),
        unit.trim().to_string(),
    )
}

fn number(value: &TypedConfigValue) -> Option<f64> {
    match value {
        TypedConfigValue::F64(value) => Some(*value),
        TypedConfigValue::Int(value) => Some(*value as f64),
        TypedConfigValue::UInt(value) => Some(*value as f64),
        _ => None,
    }
}

/// One device, from its resource record and its status-lane entries.
pub(crate) fn device(record: &ResourceRecord, entries: &[StatusEntry]) -> HardwareDevice {
    let entry = |key: &str| {
        entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.value)
    };
    let status = entry(STATUS_KEY)
        .and_then(text)
        .filter(|status| !status.is_empty())
        .unwrap_or_else(|| status_from_availability(record).to_string());
    let reason = entry(REASON_KEY)
        .and_then(text)
        .filter(|reason| !reason.is_empty());

    let mut readings: Vec<HardwareReading> = entries
        .iter()
        .filter(|entry| {
            !matches!(entry.key.as_str(), STATUS_KEY | REASON_KEY | READ_US_KEY)
                && !entry.key.starts_with(CONTROL_PREFIX)
        })
        .filter_map(|entry| {
            Some(HardwareReading {
                name: entry.key.clone(),
                value: Some(number(&entry.value)?),
                unit: label_or_empty(record, &format!("lemnos.unit.{}", entry.key)),
            })
        })
        .collect();
    readings.sort_by(|a, b| a.name.cmp(&b.name));

    let controls = record
        .labels
        .iter()
        .filter_map(|label| {
            let (key, range) = split_label(label)?;
            let name = key.strip_prefix("lemnos.control.")?;
            let (min, max, unit) = parse_range(range.unwrap_or_default());
            Some(HardwareControl {
                name: name.to_string(),
                value: entry(&format!("{CONTROL_PREFIX}{name}")).and_then(number),
                min,
                max,
                unit,
            })
        })
        .collect();

    HardwareDevice {
        id: device_id(record),
        class: label_or_empty(record, "lemnos.class"),
        model: label_or_empty(record, "lemnos.model"),
        status,
        reason,
        readings,
        controls,
    }
}

/// The snapshot of `devices` taken at `at` (Unix seconds), sorted by id; `None`
/// with no devices, which reads as unknown.
pub(crate) fn snapshot(mut devices: Vec<HardwareDevice>, at: i64) -> Option<HardwareSnapshot> {
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    (!devices.is_empty()).then_some(HardwareSnapshot { at, devices })
}

#[cfg(test)]
mod tests {
    use orion_control_plane::StatusSubject;
    use orion_core::{ProviderId, ResourceId, ResourceType};

    use super::*;

    fn record(id: &str, labels: &[&str]) -> ResourceRecord {
        labels
            .iter()
            .fold(
                ResourceRecord::builder(
                    ResourceId::new(id),
                    ResourceType::new(DEVICE_TYPE),
                    ProviderId::new("lemnos"),
                )
                .availability(AvailabilityState::Available)
                .health(HealthState::Healthy),
                |builder, label| builder.label(*label),
            )
            .build()
    }

    fn entry(id: &str, key: &str, value: TypedConfigValue) -> StatusEntry {
        StatusEntry::new(StatusSubject::Resource(ResourceId::new(id)), key, value)
    }

    #[test]
    fn labels_give_the_identity_units_and_controls() {
        let imu = record(
            "lemnos.raze.imu",
            &[
                "lemnos.board=raze",
                "lemnos.class=imu",
                "lemnos.model=bmi088",
                "lemnos.unit.accel_x=m/s²",
                "lemnos.control.range=2..16 g",
            ],
        );
        let device = device(
            &imu,
            &[
                entry("lemnos.raze.imu", "accel_x", TypedConfigValue::F64(0.12)),
                entry("lemnos.raze.imu", "read_us", TypedConfigValue::UInt(900)),
                entry(
                    "lemnos.raze.imu",
                    "control.range",
                    TypedConfigValue::F64(4.0),
                ),
            ],
        );
        assert_eq!(device.id, "imu");
        assert_eq!(device.class, "imu");
        assert_eq!(device.model, "bmi088");
        assert_eq!(device.status, "available", "from availability and health");
        assert_eq!(device.reason, None);
        assert_eq!(
            device.controls,
            vec![HardwareControl {
                name: "range".into(),
                value: Some(4.0),
                min: Some(2.0),
                max: Some(16.0),
                unit: "g".into(),
            }]
        );
        assert_eq!(
            device.readings,
            vec![HardwareReading {
                name: "accel_x".into(),
                value: Some(0.12),
                unit: "m/s²".into(),
            }]
        );
    }

    #[test]
    fn the_status_entry_wins_and_a_faulted_device_keeps_its_reason() {
        let mag = record("lemnos.raze.magnetometer", &["lemnos.board=raze"]);
        let device = device(
            &mag,
            &[
                entry(
                    "lemnos.raze.magnetometer",
                    "status",
                    TypedConfigValue::String("faulted".into()),
                ),
                entry(
                    "lemnos.raze.magnetometer",
                    "reason",
                    TypedConfigValue::String("no response on 0x1e".into()),
                ),
            ],
        );
        assert_eq!(device.status, "faulted");
        assert_eq!(device.reason.as_deref(), Some("no response on 0x1e"));
        assert!(device.readings.is_empty());
    }

    #[test]
    fn availability_and_health_stand_in_without_a_status_entry() {
        let degraded = ResourceRecord::builder(
            ResourceId::new("lemnos.raze.fan"),
            ResourceType::new(DEVICE_TYPE),
            ProviderId::new("lemnos"),
        )
        .availability(AvailabilityState::Available)
        .health(HealthState::Degraded)
        .build();
        assert_eq!(device(&degraded, &[]).status, "degraded");

        let gone = ResourceRecord::builder(
            ResourceId::new("lemnos.raze.gps"),
            ResourceType::new(DEVICE_TYPE),
            ProviderId::new("lemnos"),
        )
        .availability(AvailabilityState::Unavailable)
        .health(HealthState::Unknown)
        .build();
        assert_eq!(device(&gone, &[]).status, "missing");
    }

    #[test]
    fn a_fan_reports_its_control_and_no_control_reading() {
        let fan = record(
            "lemnos.raze.fan",
            &[
                "lemnos.board=raze",
                "lemnos.class=fan",
                "lemnos.model=pwmfan",
                "lemnos.control.duty=0..1",
            ],
        );
        let device = device(
            &fan,
            &[
                entry(
                    "lemnos.raze.fan",
                    "control.duty",
                    TypedConfigValue::F64(0.83),
                ),
                entry("lemnos.raze.fan", "rpm", TypedConfigValue::UInt(4200)),
            ],
        );
        assert_eq!(device.controls.len(), 1);
        assert_eq!(device.controls[0].name, "duty");
        assert_eq!(device.controls[0].value, Some(0.83));
        assert_eq!(
            (device.controls[0].min, device.controls[0].max),
            (Some(0.0), Some(1.0))
        );
        assert_eq!(device.controls[0].unit, "");
        assert_eq!(device.readings.len(), 1);
        assert_eq!(device.readings[0].name, "rpm");
        assert_eq!(device.readings[0].value, Some(4200.0));
        assert_eq!(device.readings[0].unit, "");
    }

    #[test]
    fn no_devices_is_no_snapshot() {
        assert_eq!(snapshot(Vec::new(), 1), None);
        let one = vec![HardwareDevice::default()];
        assert_eq!(
            snapshot(one.clone(), 7),
            Some(HardwareSnapshot {
                at: 7,
                devices: one
            })
        );
    }
}
