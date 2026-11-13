//! The K230 driver against the simulated USB bus: what it finds, what it says
//! each device is, what it grants, and starting an image through the boot ROM
//! with a reset and a stall.

use std::sync::Arc;
use std::time::Duration;

use atlas_driver::{Driver, DriverError};
use atlas_driver_k230::{FAMILY, K230Driver};
use atlas_k230::bootrom::Stage;
use atlas_types::{CapabilityKind, DeviceState, Link, LinkId, LinkKind};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, UsbBus, UsbId};

const VENDOR: u16 = 0x29f1;
const PRODUCT: u16 = 0x0230;
/// Dev's notes record both addresses; the caller passes one.
const ADDRESS: u32 = 0x8036_0000;

fn id() -> UsbId {
    UsbId {
        vendor: VENDOR,
        product: PRODUCT,
    }
}

fn device(location: &str, serial: Option<&str>) -> DeviceSummary {
    DeviceSummary {
        location: location.into(),
        vendor: VENDOR,
        product: PRODUCT,
        serial: serial.map(str::to_string),
    }
}

fn usb_boot_link() -> Link {
    Link {
        id: LinkId::new("usb-boot:1-2"),
        kind: LinkKind::UsbBoot,
        label: "USB boot".into(),
    }
}

/// The 32-byte CPU info of a stage, NUL padded.
fn cpu_info(text: &str) -> Answer {
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize(32, 0);
    Answer::Data(bytes)
}

fn driver(bus: &SimBus) -> K230Driver {
    K230Driver::new(Arc::new(bus.clone()), vec![id()])
}

#[tokio::test]
async fn discovery_finds_only_the_caller_ids_and_names_the_port() {
    let bus = SimBus::new();
    bus.attach(device("1-2", Some("K230-A")));
    bus.attach(DeviceSummary {
        location: "1-3".into(),
        vendor: 0x1d6b,
        product: 0x0002,
        serial: None,
    });
    let found = driver(&bus).discover(&usb_boot_link()).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].address, "1-2");
    assert_eq!(found[0].family.as_str(), FAMILY);
    assert_eq!(found[0].link, LinkId::new("usb-boot:1-2"));
}

#[tokio::test]
async fn with_no_ids_nothing_is_found() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    let driver = K230Driver::new(Arc::new(bus.clone()), Vec::new());
    assert!(driver.discover(&usb_boot_link()).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_manifest_names_the_usb_boot_link_and_the_family() {
    let bus = SimBus::new();
    let manifest = driver(&bus).manifest().clone();
    assert_eq!(manifest.family.as_str(), FAMILY);
    assert!(manifest.handles(&LinkKind::UsbBoot));
    assert!(!manifest.handles(&LinkKind::Ethernet));
}

#[tokio::test]
async fn identify_says_the_stage_and_keys_the_device_by_its_serial() {
    let bus = SimBus::new();
    bus.attach(device("1-2", Some("K230-A")));
    bus.script("1-2", [cpu_info("K230")]);
    let driver = driver(&bus);
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);
    let identity = driver.identify(&candidate).await.unwrap();
    assert_eq!(identity.state, DeviceState::BootRom);
    assert_eq!(identity.key.to_string(), format!("{FAMILY}:K230-A"));
    assert_eq!(
        identity.attributes.get("stage").map(String::as_str),
        Some("boot-rom")
    );
    let shown = driver.capabilities(&identity).shown(DeviceState::BootRom);
    assert_eq!(shown, vec![CapabilityKind::Info, CapabilityKind::Flash]);
    assert!(
        !driver
            .capabilities(&identity)
            .shown(DeviceState::Running)
            .contains(&CapabilityKind::Flash)
    );
}

#[tokio::test]
async fn a_device_without_a_serial_is_keyed_by_its_port() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    bus.script("1-2", [cpu_info("Uboot Stage for K230")]);
    let driver = driver(&bus);
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);
    let identity = driver.identify(&candidate).await.unwrap();
    assert_eq!(identity.key.to_string(), format!("{FAMILY}:usb-1-2"));
    assert_eq!(
        identity.attributes.get("stage").map(String::as_str),
        Some("burner")
    );
}

#[tokio::test]
async fn a_device_that_answers_something_else_is_incompatible() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    bus.script("1-2", [cpu_info("some other chip")]);
    let driver = driver(&bus);
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);
    let error = driver.identify(&candidate).await.unwrap_err();
    assert!(matches!(error, DriverError::Incompatible(_)), "{error:?}");
    assert!(error.to_string().contains("some other chip"), "{error}");
}

#[tokio::test]
async fn an_image_is_started_and_the_device_comes_back_as_the_stage_it_answers() {
    let bus = SimBus::new();
    bus.attach(device("1-2", Some("K230-A")));
    let image = vec![3u8; 2500];
    // Stage check, then set address, three pages, start (the reset).
    bus.script(
        "1-2",
        [
            cpu_info("K230"),
            Answer::Ok,
            Answer::Ok,
            Answer::Ok,
            Answer::Ok,
            Answer::Disconnect,
        ],
    );
    let driver = driver(&bus);
    driver.start_image("1-2", &image, ADDRESS).await.unwrap();
    assert!(bus.devices().await.unwrap().is_empty());

    // The image came back: the burner answers at the same port.
    bus.attach(device("1-2", Some("K230-A")));
    bus.script("1-2", [cpu_info("Uboot Stage for K230")]);
    let stage = driver
        .await_restart("1-2", Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(stage, Stage::Burner);
}

#[tokio::test]
async fn a_stalled_image_page_fails_the_start_and_no_start_is_sent() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    bus.script("1-2", [cpu_info("K230"), Answer::Ok, Answer::Stall]);
    let error = driver(&bus)
        .start_image("1-2", &[5u8; 2000], ADDRESS)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("stalled"), "{error}");
    // The stage check, the set address, and the page that stalled. No start.
    assert_eq!(bus.records("1-2").len(), 3);
}

#[tokio::test]
async fn a_burner_is_not_started_over_again() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    bus.script("1-2", [cpu_info("Uboot Stage for K230")]);
    let error = driver(&bus)
        .start_image("1-2", &[1u8; 4], ADDRESS)
        .await
        .unwrap_err();
    assert!(matches!(error, DriverError::Incompatible(_)), "{error:?}");
    assert_eq!(bus.records("1-2").len(), 1, "only the stage was asked");
}

#[tokio::test]
async fn an_address_the_caller_gives_outside_the_range_is_refused_before_any_transfer() {
    let bus = SimBus::new();
    bus.attach(device("1-2", None));
    let error = driver(&bus)
        .start_image("1-2", &[1u8; 4], 0x8000_0000)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outside"), "{error}");
    assert!(bus.records("1-2").is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_device_that_never_comes_back_is_reported_at_the_deadline() {
    let bus = SimBus::new();
    let error = driver(&bus)
        .await_restart("1-2", Duration::from_secs(12))
        .await
        .unwrap_err();
    assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
    assert!(error.to_string().contains("timed out"), "{error}");
}
