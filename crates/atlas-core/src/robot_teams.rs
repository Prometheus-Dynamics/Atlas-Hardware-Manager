//! The teams whose robot networks Atlas has seen devices on, for drivers
//! that look for devices behind a board's USB route
//! (`atlas_driver::robot_net::RobotTeams`): a route wider than one robot
//! network is narrowed to the known teams' networks. A device seen at
//! 10.63.90.10 once means team 6390's network is worth a look next time,
//! even after a restart (the saved inventory keeps its addresses).

use std::net::IpAddr;

use atlas_driver::attributes;
use atlas_driver::robot_net::RobotTeams;

use crate::DeviceRecord;
use crate::links::host_ip;

/// Notes the robot networks of a device's addresses.
pub(crate) fn note(ips: &[IpAddr]) {
    for ip in ips {
        if let IpAddr::V4(v4) = ip {
            RobotTeams::shared().note_address(*v4);
        }
    }
}

/// Notes the robot networks of every address the saved records list.
pub(crate) fn note_records(records: &[DeviceRecord]) {
    for record in records {
        note(&addresses(record));
    }
}

fn addresses(record: &DeviceRecord) -> Vec<IpAddr> {
    let listed = record
        .identity
        .attributes
        .get(attributes::ADDRESSES)
        .map(String::as_str)
        .unwrap_or_default()
        .split(',')
        .filter_map(|ip| ip.trim().parse().ok());
    host_ip(&record.identity.address)
        .into_iter()
        .chain(listed)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atlas_driver::{DeviceKey, DeviceMode, Identity, LinkId, LinkKind};

    use super::*;
    use crate::Presence;

    /// A camera saved at its USB address, with its robot-network address
    /// among its addresses: team 1678's network is known after a restart.
    #[test]
    fn saved_addresses_name_their_robot_networks() {
        let record = DeviceRecord {
            key: DeviceKey::new("raze", "2da7d9c5e9ff7b61"),
            identity: Identity {
                key: DeviceKey::new("raze", "2da7d9c5e9ff7b61"),
                model: "Raze Gen 1".into(),
                mode: DeviceMode::Normal,
                versions: BTreeMap::new(),
                name: Some("left".into()),
                link: LinkId("robot-net".into()),
                address: "http://172.31.167.9:5899/.well-known/pd-device".into(),
                attributes: BTreeMap::from([(
                    attributes::ADDRESSES.to_string(),
                    "10.16.78.10,172.31.167.9".to_string(),
                )]),
            },
            link_kind: LinkKind::UsbNetwork,
            capabilities: Vec::new(),
            presence: Presence::Offline,
            first_seen_ms: 0,
            last_seen_ms: 0,
            label: None,
            robot: None,
        };
        note_records(&[record]);
        assert!(RobotTeams::shared().all().contains(&1678));
    }
}
