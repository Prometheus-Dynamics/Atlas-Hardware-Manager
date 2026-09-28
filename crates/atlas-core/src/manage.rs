//! Inventory management: names, robot profiles, and host health.

use atlas_driver::{DeviceKey, HealthCheck, HealthStatus};

use crate::inventory::Presence;
use crate::robots::RobotStatus;
use crate::{Atlas, CoreError, Event, RobotProfile};

fn clean(text: Option<String>) -> Option<String> {
    text.map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

impl Atlas {
    /// Names a device in Atlas. `None` or blank clears the name.
    pub fn set_label(&self, key: &DeviceKey, label: Option<String>) -> Result<(), CoreError> {
        self.edit_device(key, |record| record.label = clean(label))
    }

    /// Tags a device with a robot, or clears the tag with `None`.
    pub fn set_device_robot(
        &self,
        key: &DeviceKey,
        robot: Option<String>,
    ) -> Result<(), CoreError> {
        let robot = clean(robot);
        if let Some(name) = &robot
            && !self.inner.state().robots.contains_key(name)
        {
            return Err(CoreError::UnknownRobot(name.clone()));
        }
        self.edit_device(key, |record| record.robot = robot)
    }

    /// Removes an offline device from the remembered inventory.
    pub fn forget_device(&self, key: &DeviceKey) -> Result<(), CoreError> {
        {
            let mut state = self.inner.state();
            let record = state
                .inventory
                .get(key)
                .ok_or_else(|| CoreError::UnknownDevice(key.clone()))?;
            if record.presence == Presence::Online {
                return Err(CoreError::DeviceOnline(key.clone()));
            }
            state.inventory.remove(key);
            for robot in state.robots.values_mut() {
                for role in &mut robot.roles {
                    if role.device.as_ref() == Some(key) {
                        role.device = None;
                    }
                }
            }
        }
        self.inner
            .events
            .emit(Event::DeviceForgotten { key: key.clone() });
        self.inner.events.emit(Event::RobotsChanged);
        self.inner.persist();
        Ok(())
    }

    fn edit_device(
        &self,
        key: &DeviceKey,
        change: impl FnOnce(&mut crate::DeviceRecord),
    ) -> Result<(), CoreError> {
        let record = {
            let mut state = self.inner.state();
            let record = state
                .inventory
                .get_mut(key)
                .ok_or_else(|| CoreError::UnknownDevice(key.clone()))?;
            change(record);
            record.clone()
        };
        self.inner.events.emit(Event::DeviceSeen {
            record: Box::new(record),
            new: false,
        });
        self.inner.persist();
        Ok(())
    }

    pub fn robots(&self) -> Vec<RobotProfile> {
        self.inner.state().robots.values().cloned().collect()
    }

    pub fn robot(&self, name: &str) -> Option<RobotProfile> {
        self.inner.state().robots.get(name).cloned()
    }

    /// Creates or replaces a robot profile. Devices assigned to roles are
    /// tagged with the robot. `previous_name` renames an existing profile.
    pub fn save_robot(
        &self,
        profile: RobotProfile,
        previous_name: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut profile = profile;
        profile.name = profile.name.trim().to_string();
        if profile.name.is_empty() {
            return Err(CoreError::InvalidRobot("the name is empty".into()));
        }
        let mut seen_roles = std::collections::BTreeSet::new();
        for role in &mut profile.roles {
            role.role = role.role.trim().to_string();
            if role.role.is_empty() {
                return Err(CoreError::InvalidRobot("a role has no name".into()));
            }
            if !seen_roles.insert(role.role.clone()) {
                return Err(CoreError::InvalidRobot(format!(
                    "the role `{}` appears twice",
                    role.role
                )));
            }
        }

        {
            let mut state = self.inner.state();
            let renaming_from = previous_name.filter(|old| *old != profile.name);
            if state.robots.contains_key(&profile.name) && renaming_from.is_some() {
                return Err(CoreError::InvalidRobot(format!(
                    "a robot named `{}` already exists",
                    profile.name
                )));
            }
            if let Some(old) = renaming_from {
                state.robots.remove(old);
                for record in state.inventory.all() {
                    if record.robot.as_deref() == Some(old)
                        && let Some(record) = state.inventory.get_mut(&record.key)
                    {
                        record.robot = Some(profile.name.clone());
                    }
                }
            }
            for role in &profile.roles {
                if let Some(record) = role
                    .device
                    .as_ref()
                    .and_then(|key| state.inventory.get_mut(key))
                {
                    record.robot = Some(profile.name.clone());
                }
            }
            state.robots.insert(profile.name.clone(), profile);
        }
        self.inner.events.emit(Event::RobotsChanged);
        self.inner.persist();
        Ok(())
    }

    /// Deletes a robot profile and clears its tag from devices.
    pub fn delete_robot(&self, name: &str) -> Result<(), CoreError> {
        {
            let mut state = self.inner.state();
            state
                .robots
                .remove(name)
                .ok_or_else(|| CoreError::UnknownRobot(name.to_string()))?;
            for record in state.inventory.all() {
                if record.robot.as_deref() == Some(name)
                    && let Some(record) = state.inventory.get_mut(&record.key)
                {
                    record.robot = None;
                }
            }
        }
        self.inner.events.emit(Event::RobotsChanged);
        self.inner.persist();
        Ok(())
    }

    /// Whether a robot is ready: every role filled, online, and on target.
    pub fn robot_status(&self, name: &str) -> Result<RobotStatus, CoreError> {
        let state = self.inner.state();
        let profile = state
            .robots
            .get(name)
            .ok_or_else(|| CoreError::UnknownRobot(name.to_string()))?;
        Ok(RobotStatus::compute(profile, &state.inventory))
    }

    pub fn robot_statuses(&self) -> Vec<RobotStatus> {
        let state = self.inner.state();
        state
            .robots
            .values()
            .map(|profile| RobotStatus::compute(profile, &state.inventory))
            .collect()
    }

    /// Host readiness from every driver and link source, worst first.
    pub async fn health_checks(&self) -> Vec<HealthCheck> {
        let drivers = self.inner.registry.all();
        let mut checks: Vec<HealthCheck> =
            futures::future::join_all(drivers.iter().map(|driver| driver.health()))
                .await
                .into_iter()
                .flatten()
                .collect();
        checks.extend(
            futures::future::join_all(self.inner.link_sources.iter().map(|source| source.health()))
                .await
                .into_iter()
                .flatten(),
        );
        if drivers.is_empty() {
            checks.push(HealthCheck::warning(
                "core.drivers",
                "Device drivers",
                "No device drivers are enabled.",
                "Enable simulated devices in Settings, or install a build with drivers.",
            ));
        }
        checks.sort_by_key(|check| {
            std::cmp::Reverse(match check.status {
                HealthStatus::Error => 2,
                HealthStatus::Warning => 1,
                HealthStatus::Ok => 0,
            })
        });
        checks
    }
}
