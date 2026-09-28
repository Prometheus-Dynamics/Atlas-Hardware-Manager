use std::collections::BTreeMap;

use atlas_driver::{DeviceKey, Family};
use serde::{Deserialize, Serialize};

use crate::inventory::Inventory;
use crate::{Presence, ReleaseTarget, StagedRollout, UpdateRequest};

/// A named set of devices that travel together, with the versions they
/// should run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotProfile {
    pub name: String,
    pub roles: Vec<RobotRole>,
    /// Target version per family.
    #[serde(default)]
    pub targets: BTreeMap<Family, String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A slot on the robot, such as `cam-front`, filled by one device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotRole {
    pub role: String,
    pub family: Family,
    pub device: Option<DeviceKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RobotState {
    /// Every role is filled, online, and on its target version.
    Ready,
    /// Every role is online, but some run a different version.
    NeedsUpdate,
    /// A role's device is offline.
    MissingDevices,
    /// A role has no device assigned.
    Unassigned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleStatus {
    pub role: String,
    pub family: Family,
    pub device: Option<DeviceKey>,
    pub device_name: Option<String>,
    pub presence: Option<Presence>,
    pub version: Option<String>,
    pub target: Option<String>,
    /// `None` when there is no target or no known version to compare.
    pub up_to_date: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotStatus {
    pub name: String,
    pub state: RobotState,
    pub roles: Vec<RoleStatus>,
    /// Devices tagged with this robot that fill no role, such as spares.
    pub unassigned_devices: Vec<DeviceKey>,
}

impl RobotStatus {
    pub(crate) fn compute(profile: &RobotProfile, inventory: &Inventory) -> Self {
        let roles: Vec<RoleStatus> = profile
            .roles
            .iter()
            .map(|role| {
                let record = role.device.as_ref().and_then(|key| inventory.get(key));
                let version = record
                    .and_then(|record| record.identity.primary_version())
                    .map(str::to_string);
                let target = profile.targets.get(&role.family).cloned();
                let up_to_date = match (&version, &target) {
                    (Some(version), Some(target)) => Some(version == target),
                    _ => None,
                };
                RoleStatus {
                    role: role.role.clone(),
                    family: role.family.clone(),
                    device: role.device.clone(),
                    device_name: record.map(|record| record.display_name()),
                    presence: record.map(|record| record.presence),
                    version,
                    target,
                    up_to_date,
                }
            })
            .collect();

        let state = if roles.iter().any(|role| role.device.is_none()) {
            RobotState::Unassigned
        } else if roles
            .iter()
            .any(|role| role.presence != Some(Presence::Online))
        {
            RobotState::MissingDevices
        } else if roles.iter().any(|role| role.up_to_date == Some(false)) {
            RobotState::NeedsUpdate
        } else {
            RobotState::Ready
        };

        let unassigned_devices = inventory
            .all()
            .into_iter()
            .filter(|record| record.robot.as_deref() == Some(profile.name.as_str()))
            .filter(|record| {
                !profile
                    .roles
                    .iter()
                    .any(|role| role.device.as_ref() == Some(&record.key))
            })
            .map(|record| record.key)
            .collect();

        Self {
            name: profile.name.clone(),
            state,
            roles,
            unassigned_devices,
        }
    }

    /// An update request for every online role not on its target version.
    /// Returns `None` when nothing needs updating.
    pub fn update_request(&self, staged: StagedRollout) -> Option<UpdateRequest> {
        let mut request = UpdateRequest {
            staged,
            ..UpdateRequest::default()
        };
        for role in &self.roles {
            let (Some(device), Some(target), Some(false), Some(Presence::Online)) =
                (&role.device, &role.target, role.up_to_date, role.presence)
            else {
                continue;
            };
            request.devices.push(device.clone());
            request
                .releases
                .entry(role.family.clone())
                .or_insert_with(|| ReleaseTarget::version(target.clone()));
        }
        (!request.devices.is_empty()).then_some(request)
    }
}
