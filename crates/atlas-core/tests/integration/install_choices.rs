//! What a device can take a file by (install methods, devices-as-data step
//! 1): a running board updates in place from an A/B image and refuses a
//! slots image with the reason; a board in USB boot takes a fresh install.

use std::sync::Arc;

use atlas_core::{Atlas, MemoryStore};
use atlas_driver::{
    Artifact, ArtifactKind, DeviceKey, InstallMethodId, InstallOptions, InstallRequest,
};
use atlas_driver_mock::{MockDevice, MockFleet, SIM_HELIOS};

async fn atlas_for(fleet: &MockFleet) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .store(Arc::new(MemoryStore::default()));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    let atlas = builder.build().unwrap();
    atlas.scan().await;
    atlas
}

fn request(kind: ArtifactKind) -> InstallRequest {
    InstallRequest {
        artifact: Artifact {
            name: "image".into(),
            path: "/nonexistent".into(),
            sha256: "0".repeat(64),
            size_bytes: 1,
        },
        kind,
        version: "2026.3.4".into(),
        options: InstallOptions::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_board_updates_from_an_ab_image_and_says_why_not_a_slots_image() {
    let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-1"));
    let atlas = atlas_for(&fleet).await;
    let key = DeviceKey::new(SIM_HELIOS, "H-1");

    let ab = atlas
        .install_choices(&key, &request(ArtifactKind::AbImage))
        .unwrap();
    assert_eq!(ab.choices[0].method, InstallMethodId::AbImage);
    assert!(ab.choices[0].plan.is_some(), "{:?}", ab.choices[0]);

    let slots = atlas
        .install_choices(&key, &request(ArtifactKind::SlotsImage))
        .unwrap();
    let update = slots
        .choices
        .iter()
        .find(|c| c.method == InstallMethodId::AbImage)
        .unwrap();
    assert!(update.plan.is_none());
    assert!(
        update
            .why_not
            .as_deref()
            .unwrap()
            .contains("needs one fresh install of a slots image")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_board_in_usb_boot_takes_a_fresh_install_that_erases() {
    let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-2").recovery());
    let atlas = atlas_for(&fleet).await;
    let key = DeviceKey::new(SIM_HELIOS, "H-2");
    let choices = atlas
        .install_choices(&key, &request(ArtifactKind::DiskImage))
        .unwrap();
    let fresh = &choices.choices[0];
    assert_eq!(fresh.method, InstallMethodId::Fresh);
    assert!(fresh.plan.as_ref().is_some_and(|p| p.erases), "{fresh:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_method_the_device_hasnt_changes_nothing() {
    let fleet = MockFleet::new().with(MockDevice::new(SIM_HELIOS, "H-3"));
    let atlas = atlas_for(&fleet).await;
    let key = DeviceKey::new(SIM_HELIOS, "H-3");
    let failure = atlas
        .install_by(
            &key,
            InstallMethodId::Slot,
            &request(ArtifactKind::SlotBundle),
            &atlas_driver::ProgressSink::discard(),
            &atlas_driver::CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(failure.changed_nothing);
}
