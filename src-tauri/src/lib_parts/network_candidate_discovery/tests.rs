use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_number_maps_to_roborio_ip() {
        assert_eq!(team_number_to_roborio_ip(6390), "10.63.90.2");
        assert_eq!(team_number_to_roborio_ip(254), "10.2.54.2");
    }

    #[test]
    fn usb_ip_neighbor_requires_real_reachability() {
        let neighbor = NetworkNeighbor {
            ip: "172.22.11.2".to_string(),
            mac: Some("00:80:2f:aa:bb:cc".to_string()),
            interface: Some("usb0".to_string()),
            state: Some("REACHABLE".to_string()),
            is_helios_candidate: false,
        };

        assert!(!is_live_roborio_neighbor_candidate(&neighbor));
    }

    #[test]
    fn active_neighbor_with_ni_mac_can_still_be_live() {
        let neighbor = NetworkNeighbor {
            ip: "10.63.90.2".to_string(),
            mac: Some("00:80:2f:11:22:33".to_string()),
            interface: Some("eth0".to_string()),
            state: Some("REACHABLE".to_string()),
            is_helios_candidate: false,
        };

        assert!(is_live_roborio_neighbor_candidate(&neighbor));
    }

    #[test]
    fn incomplete_neighbor_without_mac_is_not_selected() {
        let neighbors = vec![NetworkNeighbor {
            ip: "10.63.90.2".to_string(),
            mac: None,
            interface: Some("enp0s1".to_string()),
            state: Some("INCOMPLETE".to_string()),
            is_helios_candidate: false,
        }];

        assert!(find_roborio_neighbor(&neighbors).is_none());
    }
}
