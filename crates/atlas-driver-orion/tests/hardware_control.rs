#[allow(dead_code)]
mod support;

use std::sync::Arc;

use atlas_driver::{CapabilitySource, HardwareCommand};
use atlas_driver_orion::OrionDirectory;
use orion_control_plane::{AvailabilityState, HealthState, TypedConfigValue};
use support::{FakeOrion, NODE, device_resource, raze, text};

fn fan(fake: &FakeOrion) {
    fake.add_resource(
        device_resource(
            "fan",
            AvailabilityState::Available,
            HealthState::Healthy,
            &[
                "lemnos.board=raze",
                "lemnos.class=fan",
                "lemnos.control.duty=0..1",
            ],
        ),
        NODE,
        &[("status", text("available"))],
    );
}

#[tokio::test]
async fn commands_go_to_the_devices_resource_as_lemnos_actions() {
    let fake = FakeOrion::new();
    fan(&fake);
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let hardware = directory.capabilities_for(&device).hardware.unwrap();

    let applied = hardware
        .control(
            &device,
            "fan",
            HardwareCommand::Set {
                control: "duty".into(),
                value: 0.5,
            },
        )
        .await
        .unwrap();
    assert_eq!(applied, Some(0.5));
    hardware
        .control(
            &device,
            "fan",
            HardwareCommand::Restore {
                control: Some("duty".into()),
            },
        )
        .await
        .unwrap();
    hardware
        .control(&device, "fan", HardwareCommand::Release)
        .await
        .unwrap();

    let state = fake.0.lock().unwrap();
    let tail = state.received.len() - 3;
    assert_eq!(state.received[tail..], ["set", "restore", "release"]);
    assert_eq!(
        state.received_args[tail].get("control"),
        Some(&TypedConfigValue::String("duty".into()))
    );
    assert_eq!(
        state.received_args[tail].get("value"),
        Some(&TypedConfigValue::F64(0.5))
    );
    assert!(state.received_args[tail + 2].is_empty());
}

#[tokio::test]
async fn a_device_orion_doesnt_list_or_a_refusal_is_an_error() {
    let fake = FakeOrion::new();
    fan(&fake);
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let hardware = directory.capabilities_for(&device).hardware.unwrap();

    let missing = hardware
        .control(&device, "gps", HardwareCommand::Release)
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("gps"), "{missing}");

    fake.0.lock().unwrap().reject = true;
    let refused = hardware
        .control(&device, "fan", HardwareCommand::Release)
        .await
        .unwrap_err();
    assert!(refused.to_string().starts_with("fan: "), "{refused}");
}
