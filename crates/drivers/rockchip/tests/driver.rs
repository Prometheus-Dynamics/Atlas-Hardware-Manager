//! The driver against the simulated boot ROM: discovery on a boot link,
//! identity, the Flash grant in the boot-ROM state, a device that left, and a
//! device that re-enumerated in loader mode at the same location.

use std::sync::Arc;

use atlas_driver::{Driver, DriverError};
use atlas_driver_rockchip::{FAMILY, RockchipDriver};
use atlas_rockchip::crc::LOADER_POLYNOMIALS;
use atlas_rockchip::flow::{ready, run_usbplug};
use atlas_rockchip::loader::{EntryKind, Loader, build_loader};
use atlas_rockchip::sim::SimRockchip;
use atlas_types::{CapabilityKind, DeviceState, Identity, Link, LinkId, LinkKind};
use atlas_usb::{OpenOptions, UsbBus, UsbId};

const ID: UsbId = UsbId {
    vendor: 0x2207,
    product: 0x350b,
};
const LOCATION: &str = "1-2";

fn boot_link() -> Link {
    Link {
        id: LinkId::new("usb-boot"),
        kind: LinkKind::UsbBoot,
        label: "USB boot".into(),
    }
}

fn driver_for(sim: &SimRockchip) -> RockchipDriver {
    RockchipDriver::new(Arc::new(sim.clone()) as Arc<dyn UsbBus>, vec![ID])
}

#[tokio::test]
async fn a_maskrom_device_is_found_named_and_granted_flash() {
    let sim = SimRockchip::maskrom(LOCATION, ID, "3588", 8);
    let driver = driver_for(&sim);

    let candidates = driver.discover(&boot_link()).await.unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].address, LOCATION);
    assert_eq!(candidates[0].family.as_str(), FAMILY);

    let identity = driver.identify(&candidates[0]).await.unwrap();
    assert_eq!(identity.state, DeviceState::BootRom);
    assert_eq!(identity.key.to_string(), format!("{FAMILY}:usb-{LOCATION}"));
    assert_eq!(identity.attributes["usb-id"], "2207:350b");

    let caps = driver.capabilities(&identity);
    assert!(caps.grants(CapabilityKind::Flash));
    assert_eq!(
        caps.shown(DeviceState::BootRom),
        vec![CapabilityKind::Info, CapabilityKind::Flash]
    );
}

#[tokio::test]
async fn flash_is_granted_only_in_the_boot_rom_state() {
    let sim = SimRockchip::maskrom(LOCATION, ID, "3588", 8);
    let driver = driver_for(&sim);
    let candidate = driver.discover(&boot_link()).await.unwrap().remove(0);
    let mut identity: Identity = driver.identify(&candidate).await.unwrap();
    identity.state = DeviceState::Running;
    assert!(
        !driver
            .capabilities(&identity)
            .shown(DeviceState::Running)
            .contains(&CapabilityKind::Flash)
    );
}

#[tokio::test]
async fn other_links_and_other_ids_are_not_offered() {
    let sim = SimRockchip::maskrom(LOCATION, ID, "3588", 8);
    let ethernet = Link {
        id: LinkId::new("eth0"),
        kind: LinkKind::Ethernet,
        label: "ethernet".into(),
    };
    assert!(
        driver_for(&sim)
            .discover(&ethernet)
            .await
            .unwrap()
            .is_empty()
    );

    let other = RockchipDriver::new(
        Arc::new(sim.clone()) as Arc<dyn UsbBus>,
        vec![UsbId {
            vendor: 0x1234,
            product: 0x5678,
        }],
    );
    assert!(other.discover(&boot_link()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_device_that_left_is_unreachable() {
    let sim = SimRockchip::maskrom(LOCATION, ID, "3588", 8);
    let driver = driver_for(&sim);
    let candidate = driver.discover(&boot_link()).await.unwrap().remove(0);
    sim.unplug();
    assert!(matches!(
        driver.identify(&candidate).await,
        Err(DriverError::Unreachable(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn the_device_is_still_identified_after_the_usbplug_re_enumerates() {
    let sim = SimRockchip::maskrom(LOCATION, ID, "3588", 8);
    let driver = driver_for(&sim);
    let candidate = driver.discover(&boot_link()).await.unwrap().remove(0);

    let ddr = vec![0x11u8; 64];
    let usbplug = vec![0x22u8; 64];
    let file = build_loader(
        b"8853",
        &[
            (EntryKind::Ddr, "ddr", &ddr, 0),
            (EntryKind::Usbplug, "usbplug", &usbplug, 0),
        ],
        LOADER_POLYNOMIALS[0],
    );
    let loader = Loader::parse(&file).unwrap();
    let mut io = run_usbplug(
        &sim,
        LOCATION,
        &loader,
        &file,
        &OpenOptions::default(),
        std::time::Duration::from_secs(10),
    )
    .await
    .unwrap();
    ready(io.as_mut()).await.unwrap();

    let identity = driver.identify(&candidate).await.unwrap();
    assert_eq!(identity.state, DeviceState::BootRom);
    assert_eq!(identity.address, LOCATION);
}
