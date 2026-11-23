//! What the UI is sent.
//!
//! State reaches the UI as [`Update`]s: a topic name and the whole current value of that topic.
//! The core keeps the latest value of every topic, so a window that opens (or a subscriber that
//! fell behind) reads them all in one `snapshot` call and then follows the updates. Nothing is
//! replayed.
//!
//! The values here are facts: a device's state, its links, when it was seen. The words and
//! colours a person sees for them are the UI's (`apps/desktop/src/lib`), from the mockups.

use atlas_inventory::{
    DeviceRecord, Health, HistoryKind, Inventory, LinkSighting, Presence, Robot,
};
use atlas_types::{CapabilityKind, CapabilitySet, DeviceKey, Identity, Link, Offers};
use serde::Serialize;
use serde_json::Value;

/// The topics the shell publishes. Pages add their own (see each page's module).
pub mod topic {
    pub const INVENTORY: &str = "inventory";
    pub const SCAN: &str = "scan";
    pub const JOBS: &str = "jobs";
    pub const SETTINGS: &str = "settings";
}

/// One change of state: the topic and its whole new value.
#[derive(Clone, Debug, Serialize)]
pub struct Update {
    pub topic: String,
    pub data: Value,
}

/// One device record, as the UI lists it.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceView {
    /// The record's key as text. It is what a route and a call name the device by.
    pub id: String,
    pub key: DeviceKey,
    /// The user's label, then the device's own name, then its key.
    pub name: String,
    pub identity: Identity,
    pub presence: Presence,
    pub health: Health,
    /// Every link the device was seen through, oldest first.
    pub links: Vec<LinkSighting>,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub label: Option<String>,
    pub robot: Option<String>,
    /// The capability kinds that apply in the device's state now, in display order.
    pub shown: Vec<CapabilityKind>,
    /// Whether the device's offers were read. When not, a kind that is not shown may still be
    /// offered, and the UI says so instead of hiding it.
    pub offers_reported: bool,
    /// What the catalog calls the device's model, when its model is a board the catalog knows.
    pub model_name: Option<String>,
    /// `controller` for a robot's controller that does not run Orion yet (the robot drivers'
    /// SystemCore): the one robot device Atlas offers something for.
    pub role: Option<&'static str>,
    /// The version of the board's default slot, when a slot list has been read for it. The
    /// device list has none of its own: the board page's slot work fills it (F4).
    pub default_version: Option<String>,
}

impl DeviceView {
    #[must_use]
    pub fn of(record: &DeviceRecord, health: Health) -> Self {
        Self {
            id: record.key.to_string(),
            key: record.key.clone(),
            name: record.display_name(),
            identity: record.identity.clone(),
            presence: record.presence,
            health,
            links: record.links.clone(),
            first_seen_ms: record.first_seen_ms,
            last_seen_ms: record.last_seen_ms,
            label: record.label.clone(),
            robot: record.robot.clone(),
            shown: shown(&record.capabilities, record),
            offers_reported: record.capabilities.offers() == Offers::Reported,
            model_name: record.identity.model.as_deref().and_then(|model| {
                atlas_catalog::embedded()
                    .boards
                    .iter()
                    .find(|board| board.model == model)
                    .map(|board| board.display_name.clone())
            }),
            role: record
                .key
                .family()
                .is_some_and(|family| family.as_str() == atlas_robot::SYSTEMCORE_FAMILY)
                .then_some("controller"),
            default_version: None,
        }
    }
}

fn shown(capabilities: &CapabilitySet, record: &DeviceRecord) -> Vec<CapabilityKind> {
    capabilities.shown(record.identity.state)
}

/// One line of the recent events: something that happened to a device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActivityLine {
    pub at_ms: u64,
    /// `info`, `warn`, or `err` for something asked that did not happen.
    pub level: &'static str,
    /// What happened, so a page can pick its own lines (such as the results of what was asked).
    pub kind: HistoryKind,
    /// The device's key as text.
    pub device: String,
    pub name: String,
    pub message: String,
}

/// How many recent events the UI is sent.
pub const ACTIVITY_LIMIT: usize = 60;

/// The inventory topic: every record, the robots, and the recent events (newest first).
#[derive(Clone, Debug, Serialize)]
pub struct InventoryView {
    pub devices: Vec<DeviceView>,
    pub robots: Vec<Robot>,
    pub activity: Vec<ActivityLine>,
}

impl InventoryView {
    #[must_use]
    pub fn of(inventory: &Inventory, now_ms: u64) -> Self {
        let records = inventory.all();
        let devices = records
            .iter()
            .map(|record| {
                let health = inventory.health(&record.key, now_ms).unwrap_or(Health::Ok);
                DeviceView::of(record, health)
            })
            .collect();
        let mut activity: Vec<ActivityLine> = records
            .iter()
            .flat_map(|record| {
                let name = record.display_name();
                let device = record.key.to_string();
                record.history.iter().map(move |entry| ActivityLine {
                    at_ms: entry.at_ms,
                    level: match entry.kind {
                        HistoryKind::Absent => "warn",
                        HistoryKind::ActionFailed => "err",
                        _ => "info",
                    },
                    kind: entry.kind,
                    device: device.clone(),
                    name: name.clone(),
                    message: entry.message.clone(),
                })
            })
            .collect();
        // Newest first; lines of one moment keep the record order, so the list never reorders
        // between two snapshots of the same history.
        activity.sort_by(|a, b| b.at_ms.cmp(&a.at_ms).then_with(|| a.device.cmp(&b.device)));
        activity.truncate(ACTIVITY_LIMIT);
        Self {
            devices,
            robots: inventory.robots().into_iter().cloned().collect(),
            activity,
        }
    }
}

/// One discovery that is failing now, and when it is tried again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScanProblem {
    pub family: String,
    pub link: String,
    pub error: String,
    pub timed_out: bool,
    pub attempt: u32,
    pub retry_in_ms: u64,
}

/// The scan topic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScanView {
    /// The manual off: no new calls start while it is set.
    pub paused: bool,
    /// The links the scan looks at now. A record's link sightings name these by id.
    pub links: Vec<Link>,
    /// Whether every driver has answered once on every link since the app opened. Until then a
    /// record from the last run is shown as it was last known.
    pub first_round_done: bool,
    pub problems: Vec<ScanProblem>,
}
