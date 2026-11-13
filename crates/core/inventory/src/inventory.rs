use std::collections::{BTreeMap, BTreeSet};

use atlas_types::{CapabilitySet, DeviceKey, Family, Identity, LinkKind};
use serde::{Deserialize, Serialize};

use crate::health::{Health, health_of};
use crate::history::HistoryKind;
use crate::record::{DeviceRecord, Presence};
use crate::robots::Robot;

/// Power cycles kept per robot. The oldest closed ones go first.
const POWER_CYCLES_KEPT: usize = 50;

/// What one sighting did to the inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A board the inventory had not seen before.
    New,
    /// A board that was absent, seen again.
    Returned,
    /// A known board whose followed link, identity or capabilities changed.
    Updated,
    /// A known board, nothing changed.
    Unchanged,
}

/// A robot's power cut: from `started_ms` until `ended_ms`, or still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerCycle {
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// Every board Atlas knows, robots, teams and robot power cycles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    devices: BTreeMap<DeviceKey, DeviceRecord>,
    robots: BTreeMap<String, Robot>,
    power_cycles: BTreeMap<String, Vec<PowerCycle>>,
}

impl Inventory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one sighting of a board on the link in `identity.link`.
    ///
    /// The board is keyed by `identity.key`, whichever state or link it is
    /// seen through. A sighting on a link that is not followed updates the
    /// record's link history only; it does not replace the followed identity
    /// unless that link ranks better or the followed link is silent.
    pub fn observe(
        &mut self,
        identity: Identity,
        link_kind: LinkKind,
        capabilities: CapabilitySet,
        now_ms: u64,
    ) -> Outcome {
        let key = identity.key.clone();
        let Some(record) = self.devices.get_mut(&key) else {
            let record = DeviceRecord::new(identity, link_kind, capabilities, now_ms);
            self.devices.insert(key, record);
            return Outcome::New;
        };

        let returned = record.presence == Presence::Offline;
        let link = identity.link.clone();
        record.note_link(&link, link_kind.clone(), &identity.address, now_ms);
        let switched = returned || record.takes_over(&link, &link_kind, now_ms);
        record.presence = Presence::Online;
        record.last_seen_ms = now_ms;

        let mut updated = false;
        if switched {
            let previous_link = record.followed_link().clone();
            let previous_version = record.version().map(str::to_string);
            let new_version = identity.primary_version().map(str::to_string);
            if previous_link != link {
                record.push_history(
                    now_ms,
                    HistoryKind::LinkChanged,
                    format!("now followed over {link}"),
                );
                updated = true;
            }
            if new_version != previous_version {
                let from = previous_version.unwrap_or_else(|| "unknown".into());
                let to = new_version.unwrap_or_else(|| "unknown".into());
                record.push_history(
                    now_ms,
                    HistoryKind::Changed,
                    format!("version {from} to {to}"),
                );
                updated = true;
            }
            if record.capabilities != capabilities {
                updated = true;
            }
            record.identity = identity;
            record.capabilities = capabilities;
        }

        if returned {
            record.push_history(now_ms, HistoryKind::Returned, format!("back on {link}"));
            Outcome::Returned
        } else if updated {
            Outcome::Updated
        } else {
            Outcome::Unchanged
        }
    }

    /// Marks each online device missing from `seen` as absent, and returns
    /// them. `seen` holds every key answered on any link in this scan.
    pub fn mark_absent(&mut self, seen: &BTreeSet<DeviceKey>, now_ms: u64) -> Vec<DeviceKey> {
        let mut gone = Vec::new();
        for (key, record) in &mut self.devices {
            if record.presence == Presence::Online && !seen.contains(key) {
                record.presence = Presence::Offline;
                record.push_history(now_ms, HistoryKind::Absent, "not seen on any link".into());
                gone.push(key.clone());
            }
        }
        gone
    }

    /// Sets or clears the user's label for a device. Returns `false` when the
    /// device is unknown.
    pub fn set_label(&mut self, key: &DeviceKey, label: Option<String>, now_ms: u64) -> bool {
        let Some(record) = self.devices.get_mut(key) else {
            return false;
        };
        let message = match &label {
            Some(text) => format!("renamed to {text}"),
            None => "label cleared".to_string(),
        };
        record.label = label;
        record.push_history(now_ms, HistoryKind::Label, message);
        true
    }

    /// Puts a device in a robot (`Some`) or takes it out (`None`). A robot
    /// that does not exist yet is created. Returns `false` when the device is
    /// unknown.
    pub fn set_device_robot(
        &mut self,
        key: &DeviceKey,
        robot: Option<String>,
        now_ms: u64,
    ) -> bool {
        let Some(record) = self.devices.get_mut(key) else {
            return false;
        };
        let message = match &robot {
            Some(name) => format!("joined robot {name}"),
            None => "left its robot".to_string(),
        };
        if let Some(name) = &robot {
            self.robots.entry(name.clone()).or_insert_with(|| Robot {
                name: name.clone(),
                ..Robot::default()
            });
        }
        record.robot = robot;
        record.push_history(now_ms, HistoryKind::Robot, message);
        true
    }

    /// Adds a robot, or replaces its team and notes.
    pub fn put_robot(&mut self, robot: Robot) {
        self.robots.insert(robot.name.clone(), robot);
    }

    /// Removes a robot. Its devices are left in no robot.
    pub fn remove_robot(&mut self, name: &str) -> Option<Robot> {
        for record in self.devices.values_mut() {
            if record.robot.as_deref() == Some(name) {
                record.robot = None;
            }
        }
        self.robots.remove(name)
    }

    pub fn robot(&self, name: &str) -> Option<&Robot> {
        self.robots.get(name)
    }

    /// Every robot, by name.
    pub fn robots(&self) -> Vec<&Robot> {
        self.robots.values().collect()
    }

    /// The devices in a robot, by key.
    pub fn robot_members(&self, name: &str) -> Vec<DeviceKey> {
        self.devices
            .values()
            .filter(|record| record.robot.as_deref() == Some(name))
            .map(|record| record.key.clone())
            .collect()
    }

    /// Each team and the robots in it, by team name. Robots without a team
    /// are not listed.
    pub fn teams(&self) -> BTreeMap<String, Vec<String>> {
        let mut teams: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for robot in self.robots.values() {
            if let Some(team) = &robot.team {
                teams
                    .entry(team.clone())
                    .or_default()
                    .push(robot.name.clone());
            }
        }
        teams
    }

    /// Starts a power cycle of a robot's power. Does nothing when one is
    /// already running. Its devices get a history line.
    pub fn begin_power_cycle(&mut self, robot: &str, now_ms: u64) {
        let cycles = self.power_cycles.entry(robot.to_string()).or_default();
        if cycles.last().is_some_and(|cycle| cycle.ended_ms.is_none()) {
            return;
        }
        cycles.push(PowerCycle {
            started_ms: now_ms,
            ended_ms: None,
        });
        let excess = cycles.len().saturating_sub(POWER_CYCLES_KEPT);
        cycles.drain(..excess);
        self.note_robot_history(robot, now_ms, "robot power cycle started");
    }

    /// Ends the running power cycle of a robot. Returns `false` when none
    /// was running.
    pub fn end_power_cycle(&mut self, robot: &str, now_ms: u64) -> bool {
        let Some(cycle) = self
            .power_cycles
            .get_mut(robot)
            .and_then(|cycles| cycles.last_mut())
            .filter(|cycle| cycle.ended_ms.is_none())
        else {
            return false;
        };
        cycle.ended_ms = Some(now_ms);
        self.note_robot_history(robot, now_ms, "robot power cycle ended");
        true
    }

    /// A robot's power cycles, oldest first.
    pub fn power_cycles(&self, robot: &str) -> &[PowerCycle] {
        self.power_cycles.get(robot).map_or(&[], Vec::as_slice)
    }

    fn note_robot_history(&mut self, robot: &str, now_ms: u64, message: &str) {
        for record in self.devices.values_mut() {
            if record.robot.as_deref() == Some(robot) {
                record.push_history(now_ms, HistoryKind::PowerCycle, message.to_string());
            }
        }
    }

    /// Removes a device the user no longer wants remembered.
    pub fn forget(&mut self, key: &DeviceKey) -> Option<DeviceRecord> {
        self.devices.remove(key)
    }

    pub fn get(&self, key: &DeviceKey) -> Option<&DeviceRecord> {
        self.devices.get(key)
    }

    /// Every record, in key order.
    pub fn all(&self) -> Vec<&DeviceRecord> {
        self.devices.values().collect()
    }

    /// Every board of one family.
    pub fn of_family(&self, family: &Family) -> Vec<&DeviceRecord> {
        self.devices
            .values()
            .filter(|record| &record.key.family == family)
            .collect()
    }

    /// The health of one device at `now_ms`, or `None` when it is unknown.
    pub fn health(&self, key: &DeviceKey, now_ms: u64) -> Option<Health> {
        let record = self.devices.get(key)?;
        let cycles = record
            .robot
            .as_deref()
            .map_or(&[][..], |robot| self.power_cycles(robot));
        Some(health_of(record, cycles, now_ms))
    }

    /// Every device whose health is [`Health::Fault`] at `now_ms`.
    pub fn faults(&self, now_ms: u64) -> Vec<DeviceKey> {
        self.devices
            .keys()
            .filter(|key| self.health(key, now_ms) == Some(Health::Fault))
            .cloned()
            .collect()
    }

    pub(crate) fn from_parts(
        devices: Vec<DeviceRecord>,
        robots: Vec<Robot>,
        power_cycles: BTreeMap<String, Vec<PowerCycle>>,
    ) -> Self {
        // Nothing is online until a scan sees it again.
        let devices = devices
            .into_iter()
            .map(|mut record| {
                record.presence = Presence::Offline;
                (record.key.clone(), record)
            })
            .collect();
        let robots = robots
            .into_iter()
            .map(|robot| (robot.name.clone(), robot))
            .collect();
        Self {
            devices,
            robots,
            power_cycles,
        }
    }

    pub(crate) fn to_parts(
        &self,
    ) -> (
        Vec<DeviceRecord>,
        Vec<Robot>,
        BTreeMap<String, Vec<PowerCycle>>,
    ) {
        (
            self.devices.values().cloned().collect(),
            self.robots.values().cloned().collect(),
            self.power_cycles.clone(),
        )
    }
}
