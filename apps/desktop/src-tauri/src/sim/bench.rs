//! The simulated bench boards: a board in recovery and a board that was seen and is gone.
//!
//! The devices are `atlas-mock`'s. What this adds is where each one is plugged in: a USB port
//! with its own link, and for the old board an address on the network too, so the list reads as
//! it does for real boards ("USB 1-4", "10.42.1.9") and one record carries both of its links.
//! `atlas-mock`'s own driver puts every device on one link of the kind `simulated`, which says
//! nothing a person can compare with a real bench.
//!
//! The boards have a family of their own: the simulated Orion node reports the family
//! `simulated` and is found on the same kinds of link by the board driver, and a named driver
//! wins its family on a link kind it handles (`DriverRegistry::resolve`).

use std::collections::BTreeMap;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_mock::device::grants_for;
use atlas_mock::{MockDevice, MockFleet};
use atlas_types::{
    Candidate, CapabilitySet, DeviceKey, DeviceState, Family, Identity, LemnosState, Link, LinkId,
    LinkKind,
};

/// The family of the simulated bench boards.
pub const FAMILY: &str = "sim-bench";

/// One place a board is plugged in: the link, and the address it answers at there.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub serial: &'static str,
    pub link: &'static str,
    pub address: &'static str,
    /// The USB port, for a link that is one.
    pub usb_port: Option<&'static str>,
}

pub const RECOVERY_BOARD: &str = "SIM-REC-1";
pub const OLD_BOARD: &str = "SIM-OLD-1";
/// The network link the old board also answered on (the app's network link).
pub const NETWORK_LINK: &str = "lan";

/// The USB ports the bench boards sit on, as links: id, label.
pub const USB_LINKS: [(&str, &str); 2] = [("usb-1-4", "USB 1-4"), ("usb-1-6", "USB 1-6")];

const PLACES: [Place; 3] = [
    Place {
        serial: RECOVERY_BOARD,
        link: "usb-1-6",
        address: "10.42.2.1",
        usb_port: Some("1-6"),
    },
    Place {
        serial: OLD_BOARD,
        link: "usb-1-4",
        address: "10.42.1.1",
        usb_port: Some("1-4"),
    },
    Place {
        serial: OLD_BOARD,
        link: NETWORK_LINK,
        address: "10.42.1.9",
        usb_port: None,
    },
];

/// The links a board is placed on, each a place of its own.
pub fn links_of(serial: &str) -> impl Iterator<Item = LinkId> + '_ {
    PLACES
        .iter()
        .filter(move |place| place.serial == serial)
        .map(|place| LinkId::new(place.link))
}

#[must_use]
pub fn key(serial: &'static str) -> DeviceKey {
    DeviceKey::from_static(FAMILY, serial)
}

/// The two bench boards, as the demo world has them.
#[must_use]
pub fn boards() -> MockFleet {
    let board = |serial, state| MockDevice {
        key: key(serial),
        ..MockDevice::new(serial, state)
    };
    MockFleet::of([
        board(RECOVERY_BOARD, DeviceState::Recovery).named("bench-c"),
        board(OLD_BOARD, DeviceState::Running)
            .named("old-bench")
            .with_version("product", "demo-os")
            .with_version("os", "2026.10.20"),
    ])
}

/// Finds the fleet's boards where they are plugged in.
#[derive(Debug)]
pub struct BenchDriver {
    fleet: MockFleet,
    manifest: DriverManifest,
}

impl BenchDriver {
    #[must_use]
    pub fn new(fleet: MockFleet) -> Self {
        Self {
            fleet,
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Simulated bench boards".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbNetwork, LinkKind::Ethernet],
                priority: 0,
            },
        }
    }

    fn online(&self, serial: &str) -> Option<MockDevice> {
        let key = DeviceKey::board(FAMILY, serial).ok()?;
        self.fleet.device(&key).filter(|device| device.online)
    }
}

#[async_trait]
impl Driver for BenchDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(PLACES
            .iter()
            .filter(|place| {
                link.id == LinkId::new(place.link) && self.online(place.serial).is_some()
            })
            .map(|place| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: place.address.to_string(),
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let unreachable =
            || DriverError::Unreachable(format!("{} did not answer", candidate.address));
        let place = PLACES
            .iter()
            .find(|place| {
                candidate.link == LinkId::new(place.link) && candidate.address == place.address
            })
            .ok_or_else(unreachable)?;
        let device = self.online(place.serial).ok_or_else(unreachable)?;
        Ok(Identity {
            key: device.key,
            model: Some(device.model),
            state: device.state,
            lemnos: LemnosState::Absent,
            versions: device.versions,
            name: device.name,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: place
                .usb_port
                .map(|port| BTreeMap::from([("usb-port".to_string(), port.to_string())]))
                .unwrap_or_default(),
        })
    }

    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds(grants_for(device.state))
    }
}
