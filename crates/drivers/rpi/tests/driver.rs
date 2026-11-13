//! The driver against the simulated bus: discovery and identity of boot ROMs,
//! the capability they grant, and a boot through the driver that resets
//! between the rounds. The USB ids and models here are test fixtures.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_driver::{Driver, DriverError, HealthStatus};
use atlas_rpi::{BootRom, RpiConfig, RpiDriver};
use atlas_rpiboot::{BootEvent, BootFiles, BootSettings, Chip, FileServerOutcome};
use atlas_types::{Candidate, CapabilityKind, DeviceState, Identity, Link, LinkId, LinkKind};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, UsbId};

const ROM_USB: UsbId = UsbId {
    vendor: 0x0a5c,
    product: 0x2712,
};
const LOCATION: &str = "1-2";

fn config(flasher: Option<BootFiles>) -> RpiConfig {
    RpiConfig {
        boot_roms: vec![BootRom {
            usb: ROM_USB,
            chip: Chip::Bcm2712,
            model: "sample-board".into(),
        }],
        settings: BootSettings::default(),
        flasher: flasher.map(Arc::new),
    }
}

fn flasher() -> BootFiles {
    BootFiles::in_memory([
        ("2712/bootcode5.bin", vec![9u8; 40_000]),
        ("config.txt", b"boot_ramdisk=1".to_vec()),
    ])
}

fn rom(location: &str) -> DeviceSummary {
    DeviceSummary {
        location: location.into(),
        vendor: ROM_USB.vendor,
        product: ROM_USB.product,
        serial: Some("abc123".into()),
    }
}

fn usb_boot_link() -> Link {
    Link {
        id: LinkId::new("usb-boot"),
        kind: LinkKind::UsbBoot,
        label: "USB boot".into(),
    }
}

fn request(command: i32, name: &str) -> Answer {
    let mut message = command.to_le_bytes().to_vec();
    message.extend_from_slice(name.as_bytes());
    message.resize(260, 0);
    Answer::Data(message)
}

#[tokio::test]
async fn discover_finds_the_configured_boot_roms_on_usb_boot_links_only() {
    let bus = SimBus::new();
    bus.attach(rom("1-2"));
    bus.attach(DeviceSummary {
        location: "1-3".into(),
        vendor: 0x1234,
        product: 0x5678,
        serial: None,
    });
    let driver = RpiDriver::new(Arc::new(bus), config(Some(flasher())));

    let found = driver.discover(&usb_boot_link()).await.unwrap();
    assert_eq!(
        found,
        vec![Candidate {
            link: LinkId::new("usb-boot"),
            family: atlas_types::Family::new("rpi"),
            address: "1-2".into(),
        }]
    );
    let ethernet = Link {
        id: LinkId::new("eth0"),
        kind: LinkKind::Ethernet,
        label: "Ethernet".into(),
    };
    assert!(driver.discover(&ethernet).await.unwrap().is_empty());
}

#[tokio::test]
async fn identify_names_the_board_and_grants_flash_only_in_the_boot_rom_state() {
    let bus = SimBus::new();
    bus.attach(rom("1-2"));
    let driver = RpiDriver::new(Arc::new(bus), config(Some(flasher())));
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);

    let identity = driver.identify(&candidate).await.unwrap();

    assert_eq!(identity.model, "sample-board");
    assert_eq!(identity.state, DeviceState::BootRom);
    assert_eq!(identity.key.to_string(), "rpi:abc123");
    assert_eq!(identity.address, "1-2");
    let shown = driver.capabilities(&identity).shown(DeviceState::BootRom);
    assert_eq!(shown, vec![CapabilityKind::Info, CapabilityKind::Flash]);
}

#[tokio::test]
async fn a_board_that_left_the_bus_cannot_be_identified() {
    let bus = Arc::new(SimBus::new());
    bus.attach(rom("1-2"));
    let driver = RpiDriver::new(bus.clone(), config(Some(flasher())));
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);
    bus.detach("1-2");

    let error = driver.identify(&candidate).await.err();

    assert!(matches!(error, Some(DriverError::Unreachable(_))));
}

#[tokio::test(start_paused = true)]
async fn boot_takes_the_board_through_a_reset_into_the_flasher_image() {
    let bus = Arc::new(SimBus::new());
    bus.attach(rom(LOCATION));
    bus.script(LOCATION, vec![Answer::Ok; 6]);
    bus.script(LOCATION, [Answer::Disconnect]);
    let driver = RpiDriver::new(bus.clone(), config(Some(flasher())));
    let candidate = driver.discover(&usb_boot_link()).await.unwrap().remove(0);
    let identity: Identity = driver.identify(&candidate).await.unwrap();
    let events = Mutex::new(Vec::new());
    let record = |event: BootEvent| events.lock().unwrap().push(event);
    let never = || false;

    let (outcome, ()) = tokio::join!(driver.boot(&identity, &record, &never), async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        bus.attach(rom(LOCATION));
        bus.script(
            LOCATION,
            [
                request(0, "config.txt"),
                Answer::Ok,
                request(1, "config.txt"),
                Answer::Ok,
                Answer::Ok,
                request(2, ""),
                Answer::Ok,
            ],
        );
    });

    assert_eq!(outcome, Ok(FileServerOutcome::Done));
    let events = events.into_inner().unwrap();
    assert!(events.contains(&BootEvent::SecondStage {
        chip: Chip::Bcm2712,
        bytes: 40_000
    }));
    assert_eq!(events.last(), Some(&BootEvent::FileServerDone));
}

#[tokio::test]
async fn boot_is_refused_without_flasher_files_and_health_says_so() {
    let bus = SimBus::new();
    bus.attach(rom("1-2"));
    let driver = RpiDriver::new(Arc::new(bus), config(None));
    let identity = Identity {
        key: atlas_types::DeviceKey::new("rpi", "abc123"),
        model: "sample-board".into(),
        state: DeviceState::BootRom,
        versions: Default::default(),
        name: None,
        link: LinkId::new("usb-boot"),
        address: "1-2".into(),
        attributes: Default::default(),
    };

    let refused = driver.boot(&identity, &|_| {}, &|| false).await.err();
    assert!(matches!(refused, Some(DriverError::Incompatible(_))));

    let health = driver.health().await;
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].status, HealthStatus::Warning);
}

#[tokio::test]
async fn a_model_the_config_does_not_know_is_unsupported() {
    let driver = RpiDriver::new(Arc::new(SimBus::new()), config(Some(flasher())));
    let identity = Identity {
        key: atlas_types::DeviceKey::new("rpi", "x"),
        model: "other-board".into(),
        state: DeviceState::BootRom,
        versions: Default::default(),
        name: None,
        link: LinkId::new("usb-boot"),
        address: "1-2".into(),
        attributes: Default::default(),
    };
    let refused = driver.boot(&identity, &|_| {}, &|| false).await.err();
    assert!(matches!(refused, Some(DriverError::Unsupported(_))));
}
