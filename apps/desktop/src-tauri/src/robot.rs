//! Robots: the named sets the inventory keeps, and what the app does for a robot's own devices.
//!
//! - A robot is added by its name and team number. The robot drivers look only at the robot
//!   networks of the teams the inventory's robots name, so adding one is what starts the look.
//! - A robot's own device (controller, radio) says its team. When it is found it joins the
//!   robot of that team by itself, unless a person already placed it.
//! - A robot device that serves a page of its own can have it opened in the browser.
//!
//! Nothing here writes to a robot device: the drivers behind it are read-only.

use atlas_inventory::{Inventory, Robot};
use atlas_types::{DeviceKey, DeviceState};
use serde::Deserialize;
use serde_json::Value;

use crate::call::{CallError, CallResult, args};
use crate::core::Core;

/// The attribute a robot device names its team by, and the one its own page is under (the
/// robot drivers' names).
const ATTR_TEAM: &str = "team";
const ATTR_PAGE: &str = "manage_url";

/// Puts a robot's own device in the robot of its team, when it is in none. Returns whether the
/// inventory changed.
pub fn place(inventory: &mut Inventory, key: &DeviceKey, now_ms: u64) -> bool {
    let Some(record) = inventory.get(key) else {
        return false;
    };
    if record.robot.is_some() || record.identity.state != DeviceState::RobotDevice {
        return false;
    }
    let Some(team) = record.identity.attributes.get(ATTR_TEAM).cloned() else {
        return false;
    };
    let robot = inventory
        .robots()
        .into_iter()
        .find(|robot| robot.team.as_deref().map(str::trim) == Some(team.as_str()))
        .map(|robot| robot.name.clone());
    match robot {
        Some(name) => inventory.set_device_robot(key, Some(name), now_ms),
        None => false,
    }
}

/// A robot's name and team as a person typed them, checked.
fn checked(name: &str, team: &str) -> Result<(String, String), CallError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 40 {
        return Err("A robot needs a name of 1 to 40 characters.".into());
    }
    let number = team
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|team| (1..=atlas_robot::MAX_TEAM).contains(team))
        .ok_or_else(|| {
            CallError(format!(
                "A team is a number from 1 to {}.",
                atlas_robot::MAX_TEAM
            ))
        })?;
    Ok((name.to_owned(), number.to_string()))
}

/// The `robot.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    match name {
        "robot.add" => {
            #[derive(Deserialize)]
            struct Add {
                name: String,
                team: String,
            }
            let Add { name: robot, team } = args(name, arguments)?;
            let (robot, team) = checked(&robot, &team)?;
            core.log()
                .breadcrumb(&format!("robot {robot} added, team {team}"));
            core.change_inventory(|inventory, now| {
                inventory.put_robot(Robot {
                    name: robot,
                    team: Some(team),
                    notes: None,
                });
                // Devices of that team found earlier join it now.
                let keys: Vec<DeviceKey> = inventory.all().iter().map(|r| r.key.clone()).collect();
                for key in keys {
                    place(inventory, &key, now);
                }
            });
            Ok(Value::Null)
        }
        "robot.remove" => {
            #[derive(Deserialize)]
            struct Remove {
                name: String,
            }
            let Remove { name: robot } = args(name, arguments)?;
            core.log().breadcrumb(&format!("robot {robot} removed"));
            let known = core.change_inventory(|inventory, now| {
                for key in inventory.robot_members(&robot) {
                    inventory.set_device_robot(&key, None, now);
                }
                inventory.remove_robot(&robot).is_some()
            });
            if known {
                Ok(Value::Null)
            } else {
                Err(CallError(format!("There is no robot named {robot}.")))
            }
        }
        // Opens a robot device's own page in the browser. The address is the one the device's
        // driver reported, never one the page sent.
        "robot.open_page" => {
            #[derive(Deserialize)]
            struct Open {
                id: String,
            }
            let Open { id } = args(name, arguments)?;
            let key = DeviceKey::parse(&id)
                .ok_or_else(|| CallError(format!("`{id}` is not a device key")))?;
            let page = core
                .with_inventory(|inventory| {
                    inventory
                        .get(&key)
                        .and_then(|record| record.identity.attributes.get(ATTR_PAGE).cloned())
                })
                .ok_or_else(|| CallError::from("This device serves no page of its own."))?;
            core.log().breadcrumb(&format!("opened {page}"));
            core.opener().open(&page).map_err(CallError)?;
            Ok(Value::Null)
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_types::{CapabilitySet, Identity, LemnosState, LinkId, LinkKind};

    fn device(family: &'static str, team: &str, state: DeviceState) -> Identity {
        Identity {
            key: DeviceKey::board(family, team).unwrap(),
            model: None,
            state,
            lemnos: LemnosState::Absent,
            versions: Default::default(),
            name: None,
            link: LinkId::new("net"),
            address: "10.0.0.2".into(),
            attributes: [(ATTR_TEAM.to_string(), team.to_string())].into(),
        }
    }

    #[test]
    fn a_robot_device_joins_the_robot_of_its_team_and_a_placed_one_is_left_alone() {
        let mut inventory = Inventory::new();
        for (name, team) in [("Practice", "9999"), ("Comp", "42")] {
            inventory.put_robot(Robot {
                name: name.into(),
                team: Some(team.into()),
                notes: None,
            });
        }
        let found = device("sample-controller", "9999", DeviceState::RobotDevice);
        let key = found.key.clone();
        inventory.observe(found, LinkKind::Ethernet, CapabilitySet::new(), 1);
        assert!(place(&mut inventory, &key, 2));
        assert_eq!(
            inventory.get(&key).unwrap().robot.as_deref(),
            Some("Practice")
        );
        // Moved by a person: it stays where it was put.
        inventory.set_device_robot(&key, Some("Comp".into()), 3);
        assert!(!place(&mut inventory, &key, 4));
        assert_eq!(inventory.get(&key).unwrap().robot.as_deref(), Some("Comp"));
    }

    #[test]
    fn a_board_or_a_team_with_no_robot_is_placed_nowhere() {
        let mut inventory = Inventory::new();
        inventory.put_robot(Robot {
            name: "Practice".into(),
            team: Some("9999".into()),
            notes: None,
        });
        let board = device("sample-soc", "9999", DeviceState::Running);
        let other = device("sample-controller", "7", DeviceState::RobotDevice);
        for found in [&board, &other] {
            inventory.observe(found.clone(), LinkKind::Ethernet, CapabilitySet::new(), 1);
            assert!(!place(&mut inventory, &found.key, 2));
        }
    }

    #[test]
    fn a_name_and_a_team_are_checked_before_a_robot_is_added() {
        assert_eq!(
            checked(" Practice ", " 9999 ").unwrap(),
            ("Practice".to_string(), "9999".to_string())
        );
        assert!(checked("", "1").is_err());
        assert!(checked("x", "0").is_err());
        assert!(checked("x", "team").is_err());
    }
}
