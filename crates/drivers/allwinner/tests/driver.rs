//! The Allwinner FEL driver against [`atlas_usb::sim::SimBus`] (discover,
//! identify, the disconnect mapping) and against the stateful mock device
//! (a whole flash through the driver).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use atlas_allwinner::{FAMILY, FelDriver, Images};
use atlas_driver::{Driver, DriverError};
use atlas_fel::layout::{Layout, ThunkIsa};
use atlas_fel::protocol::VERIFY_LEN;
use atlas_fel::sim::{FelDevice, MOCK_SOC_ID};
use atlas_types::{Candidate, CapabilityKind, DeviceState, Link, LinkId, LinkKind};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, OpenOptions, UsbBus, UsbError, UsbId, UsbIo};

const IDS: [UsbId; 1] = [UsbId {
    vendor: 0x1f3a,
    product: 0xefe8,
}];

fn layout() -> Layout {
    Layout {
        thunk_isa: ThunkIsa::Armv7,
        load_address: 0x2000,
        thunk_address: 0x20000,
        swaps: Vec::new(),
        image_address: 0x4100_0000,
        image_entry: 0x4100_0000,
    }
}

fn images(expected_soc: Option<u32>) -> Images {
    Images {
        layout: layout(),
        spl: vec![0x11; 0x400],
        image: vec![0x22; 0x100],
        expected_soc,
    }
}

fn usb_link() -> Link {
    Link {
        id: LinkId::new("usb-boot"),
        kind: LinkKind::UsbBoot,
        label: "USB boot ROM".into(),
    }
}

fn device(location: &str, vendor: u16, product: u16) -> DeviceSummary {
    DeviceSummary::new(location, vendor, product)
}

fn candidate(location: &str) -> Candidate {
    Candidate {
        link: LinkId::new("usb-boot"),
        family: atlas_types::Family::new(FAMILY),
        address: location.into(),
    }
}

fn csw(status: u8) -> Answer {
    let mut block = b"AWUS".to_vec();
    block.extend_from_slice(&[0; 8]);
    block.push(status);
    Answer::Data(block)
}

/// The verify-device exchange for a device in `mode`.
fn verify_answers(mode: u16) -> Vec<Answer> {
    let mut reply = vec![0u8; VERIFY_LEN];
    reply[0..8].copy_from_slice(b"AWUSBFEX");
    reply[8..12].copy_from_slice(&MOCK_SOC_ID.to_le_bytes());
    reply[16..18].copy_from_slice(&mode.to_le_bytes());
    vec![
        Answer::Ok,
        Answer::Ok,
        csw(0),
        Answer::Ok,
        Answer::Data(reply),
        csw(0),
    ]
}

/// A bus with one stateful mock device at `1-2`, handed out once.
#[derive(Debug)]
struct OneFelDevice {
    device: Mutex<Option<FelDevice>>,
}

#[async_trait]
impl UsbBus for OneFelDevice {
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
        Ok(vec![device("1-2", 0x1f3a, 0xefe8)])
    }

    async fn open(
        &self,
        location: &str,
        _options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        let taken = self.device.lock().unwrap().take();
        match taken {
            Some(device) if location == "1-2" => Ok(Box::new(device)),
            _ => Err(UsbError::NotFound(location.into())),
        }
    }
}

#[test]
fn the_manifest_names_the_family_and_the_boot_rom_link() {
    let bus = SimBus::new();
    let driver = FelDriver::new(Arc::new(bus), IDS.to_vec(), None);
    let manifest = driver.manifest();
    assert_eq!(manifest.family.to_string(), FAMILY);
    assert!(manifest.handles(&LinkKind::UsbBoot));
    assert!(!manifest.handles(&LinkKind::Ethernet));
}

#[tokio::test]
async fn discover_finds_only_the_configured_ids_on_a_boot_rom_link() {
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    bus.attach(device("1-3", 0x0a5c, 0x2764));
    let driver = FelDriver::new(Arc::new(bus), IDS.to_vec(), None);
    let found = driver.discover(&usb_link()).await.unwrap();
    assert_eq!(found, vec![candidate("1-2")]);
    let elsewhere = Link {
        id: LinkId::new("eth0"),
        kind: LinkKind::Ethernet,
        label: "Ethernet".into(),
    };
    assert!(driver.discover(&elsewhere).await.unwrap().is_empty());
}

#[tokio::test]
async fn identify_is_read_only_and_says_boot_rom_with_the_soc_id() {
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    bus.script("1-2", verify_answers(1));
    let driver = FelDriver::new(Arc::new(bus.clone()), IDS.to_vec(), None);
    let identity = driver.identify(&candidate("1-2")).await.unwrap();
    assert_eq!(identity.state, DeviceState::BootRom);
    assert_eq!(identity.key.to_string(), format!("{FAMILY}:usb-1-2"));
    assert_eq!(identity.attributes["soc-id"], "0x07330000");
    // The only transfers were the verify exchange.
    assert_eq!(bus.records("1-2").len(), 6);
}

#[tokio::test]
async fn a_device_not_in_fel_is_incompatible_not_identified() {
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    bus.script("1-2", verify_answers(2));
    let driver = FelDriver::new(Arc::new(bus), IDS.to_vec(), None);
    let error = driver.identify(&candidate("1-2")).await.unwrap_err();
    assert!(matches!(error, DriverError::Incompatible(text) if text.contains("mode 2")));
}

#[tokio::test]
async fn a_device_that_drops_during_identify_is_unreachable() {
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    bus.script("1-2", [Answer::Ok, Answer::Disconnect]);
    let driver = FelDriver::new(Arc::new(bus.clone()), IDS.to_vec(), None);
    let error = driver.identify(&candidate("1-2")).await.unwrap_err();
    assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn flash_is_granted_in_the_boot_rom_only_with_images() {
    let bus = Arc::new(SimBus::new());
    let read_only = FelDriver::new(bus.clone(), IDS.to_vec(), None);
    let flashable = FelDriver::new(bus, IDS.to_vec(), Some(images(None)));
    let identity = |state: DeviceState| atlas_types::Identity {
        key: atlas_types::DeviceKey::from_static(FAMILY, "usb-1-2"),
        model: "x".into(),
        state,
        lemnos: atlas_types::LemnosState::Absent,
        versions: Default::default(),
        name: None,
        link: LinkId::new("usb-boot"),
        address: "1-2".into(),
        attributes: Default::default(),
    };
    let boot = identity(DeviceState::BootRom);
    assert!(flashable.capabilities(&boot).grants(CapabilityKind::Flash));
    assert!(!read_only.capabilities(&boot).grants(CapabilityKind::Flash));
    let running = identity(DeviceState::Running);
    assert!(
        !flashable
            .capabilities(&running)
            .grants(CapabilityKind::Flash)
    );
}

#[tokio::test]
async fn flash_without_images_is_refused_before_the_bus_is_touched() {
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    let driver = FelDriver::new(Arc::new(bus.clone()), IDS.to_vec(), None);
    let error = driver.flash(&candidate("1-2")).await.unwrap_err();
    assert!(matches!(error, DriverError::Incompatible(_)));
    assert!(bus.records("1-2").is_empty());
}

#[tokio::test]
async fn flash_through_the_driver_loads_the_spl_and_the_image() {
    let images = images(Some(MOCK_SOC_ID));
    let board = FelDevice::new(images.layout.clone(), images.spl.len(), images.image.len());
    let bus = Arc::new(OneFelDevice {
        device: Mutex::new(Some(board)),
    });
    let driver = FelDriver::new(bus, IDS.to_vec(), Some(images.clone()));
    let report = driver.flash(&candidate("1-2")).await.unwrap();
    assert_eq!(report.soc_id, MOCK_SOC_ID);
    assert_eq!(report.spl_pieces, 1);
    assert_eq!(report.image_bytes, images.image.len());
}

#[tokio::test]
async fn a_wrong_soc_is_incompatible_and_a_drop_mid_flash_is_unreachable() {
    // A description that expects another SoC is refused before any write.
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    bus.script("1-2", verify_answers(1));
    let driver = FelDriver::new(
        Arc::new(bus.clone()),
        IDS.to_vec(),
        Some(images(Some(0x1234))),
    );
    let error = driver.flash(&candidate("1-2")).await.unwrap_err();
    assert!(matches!(error, DriverError::Incompatible(_)), "{error:?}");
    assert_eq!(bus.records("1-2").len(), 6);

    // A drop during the SPL's thunk write is a device that did not respond.
    let bus = SimBus::new();
    bus.attach(device("1-2", 0x1f3a, 0xefe8));
    let mut steps = verify_answers(1);
    steps.extend([
        Answer::Ok,
        Answer::Ok,
        csw(0),
        Answer::Ok,
        Answer::Disconnect,
    ]);
    bus.script("1-2", steps);
    let driver = FelDriver::new(Arc::new(bus), IDS.to_vec(), Some(images(None)));
    let error = driver.flash(&candidate("1-2")).await.unwrap_err();
    assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
}
