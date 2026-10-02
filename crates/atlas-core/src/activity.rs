//! A short history of what happened to the fleet: devices coming and going,
//! version changes, update results, and actions. Kept between sessions.

use std::collections::VecDeque;

use atlas_driver::DeviceKey;
use serde::{Deserialize, Serialize};

use crate::atlas::Inner;
use crate::time::now_ms;
use crate::{DeviceJobStatus, DeviceRecord, Event};

/// How many entries are kept; older ones drop off.
pub(crate) const ACTIVITY_LIMIT: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityKind {
    /// Seen for the first time ever.
    DeviceFound,
    /// Back after being offline.
    DeviceOnline,
    DeviceOffline,
    VersionChanged,
    /// Entered or left recovery mode.
    ModeChanged,
    UpdateResult,
    ActionRun,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivityLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub at_ms: u64,
    pub kind: ActivityKind,
    #[serde(default)]
    pub level: ActivityLevel,
    #[serde(default)]
    pub device: Option<DeviceKey>,
    /// The robot the device belonged to at the time.
    #[serde(default)]
    pub robot: Option<String>,
    /// One sentence, for example `cam-front updated to 2026.3.1`.
    pub message: String,
}

impl ActivityEntry {
    pub(crate) fn about(
        record: &DeviceRecord,
        kind: ActivityKind,
        level: ActivityLevel,
        message: String,
    ) -> Self {
        Self {
            at_ms: now_ms(),
            kind,
            level,
            device: Some(record.key.clone()),
            robot: record.robot.clone(),
            message,
        }
    }
}

/// The entries a device sighting produces, compared with what was known.
pub(crate) fn for_sighting(
    previous: Option<&DeviceRecord>,
    current: &DeviceRecord,
) -> Vec<ActivityEntry> {
    let name = current.display_name();
    let Some(previous) = previous else {
        return vec![ActivityEntry::about(
            current,
            ActivityKind::DeviceFound,
            ActivityLevel::Info,
            format!("Found {name} ({})", current.identity.model),
        )];
    };
    let mut entries = Vec::new();
    if previous.presence == crate::Presence::Offline {
        entries.push(ActivityEntry::about(
            current,
            ActivityKind::DeviceOnline,
            ActivityLevel::Info,
            format!("{name} came online"),
        ));
    }
    if previous.identity.mode != current.identity.mode {
        let (level, message) = match current.identity.mode {
            atlas_driver::DeviceMode::Recovery => (
                ActivityLevel::Warning,
                format!("{name} is in recovery mode"),
            ),
            atlas_driver::DeviceMode::Normal => {
                (ActivityLevel::Success, format!("{name} left recovery mode"))
            }
        };
        entries.push(ActivityEntry::about(
            current,
            ActivityKind::ModeChanged,
            level,
            message,
        ));
    } else if let (Some(before), Some(after)) = (
        previous.identity.primary_version(),
        current.identity.primary_version(),
    ) && before != after
    {
        entries.push(ActivityEntry::about(
            current,
            ActivityKind::VersionChanged,
            ActivityLevel::Info,
            format!("{name} now runs {after} (was {before})"),
        ));
    }
    entries
}

/// The entry for one device's update result, if it finished.
pub(crate) fn for_update(record: &DeviceRecord, status: &DeviceJobStatus) -> Option<ActivityEntry> {
    let name = record.display_name();
    let (level, message) = match status {
        DeviceJobStatus::Verified { version } => (
            ActivityLevel::Success,
            format!("{name} updated to {version}"),
        ),
        DeviceJobStatus::RolledBack { reason } => (
            ActivityLevel::Warning,
            format!("{name} rolled back its update: {reason}"),
        ),
        DeviceJobStatus::NeedsRecovery { reason } => (
            ActivityLevel::Error,
            format!("{name} needs recovery: {reason}"),
        ),
        DeviceJobStatus::Failed { error } => (
            ActivityLevel::Error,
            format!("{name} update failed: {error}"),
        ),
        DeviceJobStatus::Queued
        | DeviceJobStatus::Running
        | DeviceJobStatus::Skipped { .. }
        | DeviceJobStatus::Cancelled => return None,
    };
    Some(ActivityEntry::about(
        record,
        ActivityKind::UpdateResult,
        level,
        message,
    ))
}

impl Inner {
    /// Adds entries, newest last, and publishes each one.
    pub(crate) fn record_activity(&self, entries: Vec<ActivityEntry>) {
        if entries.is_empty() {
            return;
        }
        {
            let mut state = self.state();
            for entry in &entries {
                state.activity.push_back(entry.clone());
            }
            trim(&mut state.activity);
        }
        for entry in entries {
            self.events.emit(Event::Activity { entry });
        }
    }
}

pub(crate) fn trim(activity: &mut VecDeque<ActivityEntry>) {
    while activity.len() > ACTIVITY_LIMIT {
        activity.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_driver::{CapabilityKind, DeviceMode, Identity, LinkId, LinkKind};

    use super::*;
    use crate::Presence;

    fn record(version: &str, presence: Presence, mode: DeviceMode) -> DeviceRecord {
        let key = DeviceKey::new("helios", "a");
        DeviceRecord {
            key: key.clone(),
            identity: Identity {
                key,
                model: "cm5".into(),
                mode,
                versions: BTreeMap::from([("os".to_string(), version.to_string())]),
                name: Some("cam-front".into()),
                link: LinkId("x".into()),
                address: "x".into(),
                attributes: BTreeMap::new(),
            },
            link_kind: LinkKind::UsbNetwork,
            capabilities: vec![CapabilityKind::Info],
            presence,
            first_seen_ms: 1,
            last_seen_ms: 2,
            label: None,
            robot: Some("comp".into()),
        }
    }

    #[test]
    fn a_new_device_is_found() {
        let entries = for_sighting(None, &record("1", Presence::Online, DeviceMode::Normal));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, ActivityKind::DeviceFound);
        assert_eq!(entries[0].robot.as_deref(), Some("comp"));
    }

    #[test]
    fn coming_back_with_a_new_version_reports_both() {
        let before = record("1", Presence::Offline, DeviceMode::Normal);
        let after = record("2", Presence::Online, DeviceMode::Normal);
        let kinds: Vec<_> = for_sighting(Some(&before), &after)
            .into_iter()
            .map(|entry| entry.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![ActivityKind::DeviceOnline, ActivityKind::VersionChanged]
        );
    }

    #[test]
    fn an_unchanged_sighting_is_quiet() {
        let current = record("1", Presence::Online, DeviceMode::Normal);
        assert!(for_sighting(Some(&current), &current).is_empty());
    }

    #[test]
    fn entering_recovery_is_a_warning() {
        let before = record("1", Presence::Online, DeviceMode::Normal);
        let after = record("bootloader", Presence::Online, DeviceMode::Recovery);
        let entries = for_sighting(Some(&before), &after);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, ActivityLevel::Warning);
    }

    #[test]
    fn the_history_is_bounded() {
        let mut activity: VecDeque<ActivityEntry> = (0..ACTIVITY_LIMIT + 5)
            .map(|i| ActivityEntry {
                at_ms: i as u64,
                kind: ActivityKind::ActionRun,
                level: ActivityLevel::Info,
                device: None,
                robot: None,
                message: i.to_string(),
            })
            .collect();
        trim(&mut activity);
        assert_eq!(activity.len(), ACTIVITY_LIMIT);
        assert_eq!(activity.front().unwrap().at_ms, 5);
    }
}
