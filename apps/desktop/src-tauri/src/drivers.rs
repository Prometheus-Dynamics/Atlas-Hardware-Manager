//! The real drivers and the computer's own links: what the app looks at outside the simulated
//! mode.
//!
//! What is wired, and how each driver is given its links:
//!
//! - Removable disks ([`atlas_driver_disk::DiskDriver`], reading the kernel's list) on one
//!   `LocalDisk` link.
//! - The robot's own devices ([`atlas_robot::drivers`]: controller, radio, SystemCore) on one
//!   `Ethernet` link. They look only at the robot networks of the teams the inventory's robots
//!   name ([`TeamList`]), so with no robot nothing is probed.
//! - Orion nodes ([`atlas_board::BoardDriver`]) on the same `Ethernet` link, at the addresses in
//!   `ATLAS_ORION_ADDRESSES` (comma separated, `orion+tcp://host:port`). The driver takes its
//!   addresses from its caller until Orion's own discovery lands (Orion D12), so there is nothing
//!   to find a running board by yet; an address that is not an Orion address is left out and said
//!   in the log.
//!
//! What is not wired, and why: the boot ROM drivers (USB boot, maskrom, burn, FEL). Each is
//! built from the USB ids it serves, and the embedded catalog lists a board's USB ids without
//! saying which loader family serves each one. That mapping is data the catalog has to carry;
//! writing vendor numbers here would put chip knowledge in the app. They join when it does.
//!
//! This mode has never been run by the item that wrote it (it would look at the bench's own
//! disks and network): it is compiled, and [`TeamList`] and the address list are unit-tested.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use atlas_board::BoardDriver;
use atlas_driver_disk::DiskDriver;
use atlas_inventory::Inventory;
use atlas_robot::{HttpReach, Teams};
use atlas_session::Seed;
use atlas_types::{Link, LinkId, LinkKind};

use crate::core::Parts;

/// The one network link of the real mode.
pub const NETWORK_LINK: &str = "network";
/// The one removable-disk link of the real mode.
pub const DISK_LINK: &str = "disks";

/// The team numbers the robot drivers look for. The core keeps it equal to the teams of the
/// inventory's robots; the drivers read it on every round.
#[derive(Clone, Debug, Default)]
pub struct TeamList {
    teams: Arc<Mutex<BTreeSet<u16>>>,
}

impl TeamList {
    /// Takes the teams of the inventory's robots. A team that is not a number is not a robot
    /// network to look at, and is left out.
    pub fn follow(&self, inventory: &Inventory) {
        let teams = inventory
            .robots()
            .iter()
            .filter_map(|robot| robot.team.as_deref()?.trim().parse::<u16>().ok())
            .filter(|team| (1..=atlas_robot::MAX_TEAM).contains(team))
            .collect();
        *self.teams.lock().unwrap_or_else(|error| error.into_inner()) = teams;
    }
}

impl Teams for TeamList {
    fn teams(&self) -> BTreeSet<u16> {
        self.teams
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

/// The seeds in a comma-separated list of Orion addresses, and the entries that are not one.
#[must_use]
pub fn seeds_from(list: &str, link: &LinkId) -> (Vec<Seed>, Vec<String>) {
    let mut seeds = Vec::new();
    let mut refused = Vec::new();
    for entry in list
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        match Seed::new(link.clone(), entry) {
            Ok(seed) => seeds.push(seed),
            Err(reason) => refused.push(format!("{entry}: {reason}")),
        }
    }
    (seeds, refused)
}

/// The app's parts for the real mode, and what could not be used (for the log).
#[must_use]
pub fn real(teams: &TeamList) -> (Parts, Vec<String>) {
    let mut parts = Parts::default();
    let network = LinkId::new(NETWORK_LINK);
    parts.links.push(Link {
        id: network.clone(),
        kind: LinkKind::Ethernet,
        label: "Network".into(),
    });
    parts.links.push(Link {
        id: LinkId::new(DISK_LINK),
        kind: LinkKind::LocalDisk,
        label: "card reader".into(),
    });
    parts.registry.register(Arc::new(DiskDriver::sysfs()));
    for driver in atlas_robot::drivers(Arc::new(HttpReach::new()), Arc::new(teams.clone())) {
        parts.registry.register(driver);
    }
    let list = std::env::var("ATLAS_ORION_ADDRESSES").unwrap_or_default();
    let (seeds, refused) = seeds_from(&list, &network);
    parts.registry.register(Arc::new(BoardDriver::new(seeds)));
    (parts, refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_inventory::Robot;

    #[test]
    fn the_team_list_is_the_numbered_teams_of_the_inventorys_robots() {
        let mut inventory = Inventory::new();
        for (name, team) in [
            ("a", Some("9999")),
            ("b", Some("no number")),
            ("c", None),
            ("d", Some("0")),
        ] {
            inventory.put_robot(Robot {
                name: name.into(),
                team: team.map(str::to_string),
                notes: None,
            });
        }
        let teams = TeamList::default();
        assert!(teams.teams().is_empty());
        teams.follow(&inventory);
        assert_eq!(teams.teams(), BTreeSet::from([9999]));
        inventory.remove_robot("a");
        teams.follow(&inventory);
        assert!(teams.teams().is_empty());
    }

    #[test]
    fn an_address_list_gives_its_orion_addresses_and_names_what_is_not_one() {
        let link = LinkId::new(NETWORK_LINK);
        let (seeds, refused) = seeds_from(" orion+tcp://10.0.0.2:9200, ,nonsense ", &link);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].link, link);
        assert_eq!(refused.len(), 1);
        assert!(refused[0].starts_with("nonsense: "), "{refused:?}");
        assert_eq!(seeds_from("", &link), (Vec::new(), Vec::new()));
    }
}
