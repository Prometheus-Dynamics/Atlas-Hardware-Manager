//! Simulated devices, one in every [`DeviceState`], behind a driver the UI can list.
//!
//! The driver is the same contract a real family's driver meets: discover on a link,
//! identify each candidate, grant capabilities by state. A device can be taken offline and
//! brought back, which is how "lost and found again" is shown.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkId, LinkKind,
};

/// The family every simulated device belongs to.
pub const SIM_FAMILY: &str = "simulated";
/// The one simulated link every device sits on.
pub const SIM_LINK: &str = "sim0";

/// One simulated device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockDevice {
    pub key: DeviceKey,
    pub model: String,
    pub state: DeviceState,
    pub versions: BTreeMap<String, String>,
    pub name: Option<String>,
    /// Whether it answers on the link at all. An offline device is lost to the scan.
    pub online: bool,
}

impl MockDevice {
    /// A device in `state`, with the versions that state reports.
    #[must_use]
    pub fn new(serial: &'static str, state: DeviceState) -> Self {
        let (version_name, version) = match state {
            DeviceState::Running => ("os", "1.0.0"),
            DeviceState::Recovery => ("bootloader", "2.1.0"),
            DeviceState::BootRom => ("rom", "1"),
            DeviceState::RemovableDisk => ("firmware", "0.9.0"),
            DeviceState::RobotDevice => ("firmware", "3.2.0"),
        };
        Self {
            key: DeviceKey::from_static(SIM_FAMILY, serial),
            model: format!("simulated-{}", state_word(state)),
            state,
            versions: BTreeMap::from([(version_name.to_string(), version.to_string())]),
            name: None,
            online: true,
        }
    }
}

/// One device in every state, for the UI.
#[must_use]
pub fn one_in_every_state() -> Vec<MockDevice> {
    [
        ("SIM-RUN-1", DeviceState::Running),
        ("SIM-REC-1", DeviceState::Recovery),
        ("SIM-ROM-1", DeviceState::BootRom),
        ("SIM-DISK-1", DeviceState::RemovableDisk),
        ("SIM-ROBOT-1", DeviceState::RobotDevice),
    ]
    .into_iter()
    .map(|(serial, state)| MockDevice::new(serial, state))
    .collect()
}

fn state_word(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Running => "running",
        DeviceState::Recovery => "recovery",
        DeviceState::BootRom => "boot-rom",
        DeviceState::RemovableDisk => "disk",
        DeviceState::RobotDevice => "robot",
    }
}

/// The capability kinds a state grants. The device's state decides what is shown.
#[must_use]
pub fn grants_for(state: DeviceState) -> Vec<CapabilityKind> {
    match state {
        DeviceState::Running => vec![
            CapabilityKind::Update,
            CapabilityKind::Logs,
            CapabilityKind::Telemetry,
            CapabilityKind::Actions,
            CapabilityKind::Slots,
            CapabilityKind::Files,
            CapabilityKind::Status,
            CapabilityKind::SelfTest,
        ],
        DeviceState::Recovery => vec![
            CapabilityKind::Recover,
            CapabilityKind::Logs,
            CapabilityKind::Files,
        ],
        DeviceState::BootRom | DeviceState::RemovableDisk => vec![CapabilityKind::Flash],
        DeviceState::RobotDevice => vec![CapabilityKind::Telemetry, CapabilityKind::Status],
    }
}

/// The devices of a simulated link. Cheap to clone; clones share the same devices.
#[derive(Clone, Debug, Default)]
pub struct MockFleet {
    devices: Arc<Mutex<BTreeMap<DeviceKey, MockDevice>>>,
}

impl MockFleet {
    /// A fleet with one device in every state.
    #[must_use]
    pub fn one_in_every_state() -> Self {
        let fleet = Self::default();
        for device in one_in_every_state() {
            fleet.lock().insert(device.key.clone(), device);
        }
        fleet
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<DeviceKey, MockDevice>> {
        self.devices
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Takes a device off the link, or puts it back. Returns false for an unknown device.
    pub fn set_online(&self, key: &DeviceKey, online: bool) -> bool {
        match self.lock().get_mut(key) {
            Some(device) => {
                device.online = online;
                true
            }
            None => false,
        }
    }

    /// The device as it is now.
    #[must_use]
    pub fn device(&self, key: &DeviceKey) -> Option<MockDevice> {
        self.lock().get(key).cloned()
    }

    /// The driver for this fleet.
    #[must_use]
    pub fn driver(&self) -> MockDriver {
        MockDriver::new(self.clone())
    }
}

/// The simulated family's driver.
#[derive(Debug)]
pub struct MockDriver {
    fleet: MockFleet,
    manifest: DriverManifest,
}

impl MockDriver {
    #[must_use]
    pub fn new(fleet: MockFleet) -> Self {
        Self {
            fleet,
            manifest: DriverManifest {
                family: Family::new(SIM_FAMILY),
                name: "Simulated devices".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::Simulated],
                priority: 0,
            },
        }
    }
}

#[async_trait]
impl Driver for MockDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.kind != LinkKind::Simulated || link.id != LinkId::new(SIM_LINK) {
            return Ok(Vec::new());
        }
        Ok(self
            .fleet
            .lock()
            .values()
            .filter(|device| device.online)
            .filter_map(|device| {
                let (family, serial) = device.key.board_identity().ok()?;
                Some(Candidate {
                    link: link.id.clone(),
                    family: family.clone(),
                    address: serial.as_str().to_string(),
                })
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let key = DeviceKey::board(candidate.family.as_str(), candidate.address.as_str())
            .map_err(|error| DriverError::Incompatible(error.reason))?;
        let device = self
            .fleet
            .device(&key)
            .filter(|device| device.online)
            .ok_or_else(|| DriverError::Unreachable(format!("{key} did not answer")))?;
        Ok(Identity {
            key,
            model: device.model,
            state: device.state,
            lemnos: atlas_types::LemnosState::Absent,
            versions: device.versions,
            name: device.name,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: BTreeMap::new(),
        })
    }

    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds(grants_for(device.state))
    }
}
