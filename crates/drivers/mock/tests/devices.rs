//! The simulated devices, one in every state, through the driver contract.

use atlas_driver::Driver;
use atlas_mock::MockFleet;
use atlas_mock::device::{grants_for, one_in_every_state};
use atlas_types::{CapabilityKind, DeviceKey, DeviceState, Link, LinkId, LinkKind};

fn sim_link() -> Link {
    Link {
        id: LinkId::new(atlas_mock::SIM_LINK),
        kind: LinkKind::Simulated,
        label: "Simulated link".into(),
    }
}

#[tokio::test]
async fn one_device_in_every_state_is_found_and_identified() {
    let fleet = MockFleet::one_in_every_state();
    let driver = fleet.driver();
    let candidates = driver.discover(&sim_link()).await.expect("discover");
    assert_eq!(candidates.len(), 5);
    let mut states = Vec::new();
    for candidate in &candidates {
        let identity = driver.identify(candidate).await.expect("identify");
        states.push(identity.state);
        let caps = driver.capabilities(&identity);
        for kind in grants_for(identity.state) {
            assert!(caps.grants(kind), "{:?} grants {kind:?}", identity.state);
        }
    }
    for expected in [
        DeviceState::Running,
        DeviceState::Recovery,
        DeviceState::BootRom,
        DeviceState::RemovableDisk,
        DeviceState::RobotDevice,
    ] {
        assert_eq!(
            states.iter().filter(|state| **state == expected).count(),
            1,
            "{expected:?}"
        );
    }
}

#[tokio::test]
async fn a_device_lost_from_the_link_is_found_again_when_it_returns() {
    let fleet = MockFleet::one_in_every_state();
    let driver = fleet.driver();
    let key = DeviceKey::from_static(atlas_mock::SIM_FAMILY, "SIM-RUN-1");
    assert!(fleet.set_online(&key, false));
    let seen = driver.discover(&sim_link()).await.expect("discover");
    assert_eq!(seen.len(), 4, "the lost device is not offered");

    let gone = atlas_types::Candidate {
        link: LinkId::new(atlas_mock::SIM_LINK),
        family: atlas_types::Family::new(atlas_mock::SIM_FAMILY),
        address: "SIM-RUN-1".into(),
    };
    assert!(
        driver.identify(&gone).await.is_err(),
        "it does not answer while lost"
    );

    assert!(fleet.set_online(&key, true));
    let back = driver.discover(&sim_link()).await.expect("discover");
    assert_eq!(back.len(), 5);
    assert!(driver.identify(&gone).await.is_ok());
}

#[tokio::test]
async fn the_driver_only_speaks_on_its_own_link() {
    let driver = MockFleet::one_in_every_state().driver();
    let ethernet = Link {
        id: LinkId::new("eth0"),
        kind: LinkKind::Ethernet,
        label: "Ethernet".into(),
    };
    assert!(
        driver
            .discover(&ethernet)
            .await
            .expect("discover")
            .is_empty()
    );
}

#[test]
fn every_state_has_a_device_and_its_grants_show_in_that_state() {
    let devices = one_in_every_state();
    assert_eq!(devices.len(), 5);
    let shown: Vec<CapabilityKind> = grants_for(DeviceState::Running);
    assert!(shown.contains(&CapabilityKind::Update));
    assert!(grants_for(DeviceState::Recovery).contains(&CapabilityKind::Recover));
    assert!(grants_for(DeviceState::BootRom).contains(&CapabilityKind::Flash));
}
