use std::collections::BTreeMap;
use std::sync::Arc;

use atlas_core::{
    Atlas, CoreError, InventoryStore, MemoryStore, RobotProfile, RobotRole, RobotState,
    StagedRollout,
};
use atlas_driver::{DeviceKey, Family};
use atlas_driver_mock::{MockFleet, SIM_HELIOS, SIM_MCU};

fn helios(serial: &str) -> DeviceKey {
    DeviceKey::new(SIM_HELIOS, serial)
}

fn atlas_with(fleet: &MockFleet, store: Arc<dyn InventoryStore>) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(store);
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().unwrap()
}

fn comp_bot() -> RobotProfile {
    RobotProfile {
        name: "comp".into(),
        roles: vec![
            RobotRole {
                role: "front".into(),
                family: Family::new(SIM_HELIOS),
                device: Some(helios("H-1001")),
            },
            RobotRole {
                role: "left".into(),
                family: Family::new(SIM_HELIOS),
                device: Some(helios("H-1002")),
            },
        ],
        targets: BTreeMap::from([(Family::new(SIM_HELIOS), "2026.3.1".to_string())]),
        notes: None,
    }
}

#[tokio::test]
async fn robot_goes_from_needs_update_to_ready_in_one_request() {
    let fleet = MockFleet::demo();
    let atlas = atlas_with(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;
    atlas.save_robot(comp_bot(), None).unwrap();

    let status = atlas.robot_status("comp").unwrap();
    assert_eq!(status.state, RobotState::NeedsUpdate);

    let request = status.update_request(StagedRollout::Off).unwrap();
    assert_eq!(request.devices, vec![helios("H-1001"), helios("H-1002")]);
    let job = atlas.start_update(request).unwrap();
    atlas.wait_job(job).await.unwrap();

    let status = atlas.robot_status("comp").unwrap();
    assert_eq!(status.state, RobotState::Ready);
    assert!(status.update_request(StagedRollout::Off).is_none());
}

#[tokio::test]
async fn offline_and_unassigned_roles_are_reported() {
    let fleet = MockFleet::demo();
    let atlas = atlas_with(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;
    let mut profile = comp_bot();
    atlas.save_robot(profile.clone(), None).unwrap();

    fleet.set_online(&helios("H-1002"), false);
    atlas.scan().await;
    assert_eq!(
        atlas.robot_status("comp").unwrap().state,
        RobotState::MissingDevices
    );

    profile.roles.push(RobotRole {
        role: "arm".into(),
        family: Family::new(SIM_MCU),
        device: None,
    });
    atlas.save_robot(profile, Some("comp")).unwrap();
    assert_eq!(
        atlas.robot_status("comp").unwrap().state,
        RobotState::Unassigned
    );
}

#[tokio::test]
async fn role_devices_are_tagged_and_renames_follow() {
    let fleet = MockFleet::demo();
    let atlas = atlas_with(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;
    atlas.save_robot(comp_bot(), None).unwrap();
    atlas
        .set_device_robot(&helios("H-1003"), Some("comp".into()))
        .unwrap();

    let status = atlas.robot_status("comp").unwrap();
    assert_eq!(status.unassigned_devices, vec![helios("H-1003")]);

    let mut renamed = comp_bot();
    renamed.name = "practice".into();
    atlas.save_robot(renamed, Some("comp")).unwrap();

    assert!(atlas.robot("comp").is_none());
    let tag = atlas.device(&helios("H-1003")).unwrap().robot;
    assert_eq!(tag.as_deref(), Some("practice"));

    atlas.delete_robot("practice").unwrap();
    assert_eq!(atlas.device(&helios("H-1001")).unwrap().robot, None);
}

#[tokio::test]
async fn invalid_profiles_are_rejected() {
    let fleet = MockFleet::demo();
    let atlas = atlas_with(&fleet, Arc::new(MemoryStore::default()));

    let mut blank = comp_bot();
    blank.name = "  ".into();
    let mut duplicate_roles = comp_bot();
    duplicate_roles.roles[1].role = "front".into();

    assert!(matches!(
        atlas.save_robot(blank, None),
        Err(CoreError::InvalidRobot(_))
    ));
    assert!(matches!(
        atlas.save_robot(duplicate_roles, None),
        Err(CoreError::InvalidRobot(_))
    ));
    assert!(matches!(
        atlas.set_device_robot(&helios("H-1001"), Some("nope".into())),
        Err(CoreError::UnknownRobot(_) | CoreError::UnknownDevice(_))
    ));
}

#[tokio::test]
async fn labels_and_robots_survive_a_restart() {
    let fleet = MockFleet::demo();
    let store: Arc<dyn InventoryStore> = Arc::new(MemoryStore::default());
    let atlas = atlas_with(&fleet, store.clone());
    atlas.scan().await;
    atlas.save_robot(comp_bot(), None).unwrap();
    atlas
        .set_label(&helios("H-1001"), Some(" Front camera ".into()))
        .unwrap();

    let restarted = Atlas::builder().store(store).build().unwrap();

    assert_eq!(restarted.robots(), vec![comp_bot()]);
    let front = restarted.device(&helios("H-1001")).unwrap();
    assert_eq!(front.label.as_deref(), Some("Front camera"));
    assert_eq!(front.display_name(), "Front camera");
}

#[tokio::test]
async fn only_offline_devices_can_be_forgotten() {
    let fleet = MockFleet::demo();
    let atlas = atlas_with(&fleet, Arc::new(MemoryStore::default()));
    atlas.scan().await;
    atlas.save_robot(comp_bot(), None).unwrap();

    assert!(matches!(
        atlas.forget_device(&helios("H-1001")),
        Err(CoreError::DeviceOnline(_))
    ));

    fleet.set_online(&helios("H-1001"), false);
    atlas.scan().await;
    atlas.forget_device(&helios("H-1001")).unwrap();

    assert!(atlas.device(&helios("H-1001")).is_none());
    assert_eq!(atlas.robot("comp").unwrap().roles[0].device, None);
}

#[tokio::test]
async fn health_checks_warn_when_no_drivers_are_enabled() {
    let atlas = Atlas::builder().build().unwrap();
    let checks = atlas.health_checks().await;
    assert!(checks.iter().any(|check| check.id == "core.drivers"));
}
