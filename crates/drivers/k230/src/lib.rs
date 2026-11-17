//! The K230 boot ROM driver. It finds the boards in a K230's USB download
//! mode, says which stage each answers (the boot ROM, or a burner already
//! running), and grants `Flash` in the `BootRom` state and nothing else.
//!
//! The driver starts a RAM image through the boot ROM, the same request for
//! the burner and for the flasher image ([`K230Driver::start_image`]). What
//! the image does after that is not this driver's: flashing the board is an
//! ordinary update over the USB link once the flasher image runs. The write
//! path in `atlas-k230` is not called from here.
//!
//! The USB ids and the load address come from the caller as data. Nothing
//! here names a board. This crate depends on no other driver. It is named
//! `atlas-driver-k230` because `atlas-k230` is the flash crate in
//! `crates/flash/k230`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_k230::bootrom::{self, Stage};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkKind,
};
use std::str::FromStr;

use atlas_usb::UsbIdMatch;
use atlas_usb::{
    DeviceSummary, Expect, OpenOptions, Opened, PortPath, UsbBus, UsbError, UsbId, observe_stop,
    open_among, open_expected,
};

/// The device family: the vendor's chip family, as `devices/families` names it.
pub const FAMILY: &str = "canaan-k230";

pub struct K230Driver {
    manifest: DriverManifest,
    bus: Arc<dyn UsbBus>,
    ids: Vec<UsbId>,
    options: OpenOptions,
}

impl K230Driver {
    /// A driver on `bus` that matches the caller's USB ids. Discovery finds
    /// nothing when `ids` is empty.
    pub fn new(bus: Arc<dyn UsbBus>, ids: Vec<UsbId>) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Kendryte K230 USB boot".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            bus,
            ids,
            options: OpenOptions::default(),
        }
    }

    /// Devices on the bus that match one of the ids.
    async fn matching(&self) -> Result<Vec<DeviceSummary>, DriverError> {
        let devices = self.bus.devices().await.map_err(unreachable)?;
        Ok(devices
            .into_iter()
            .filter(|device| self.ids.iter().any(|id| id.matches(device)))
            .collect())
    }

    /// Loads `image` at `address` into the boot ROM at `location` and starts it.
    ///
    /// The device must be in the boot ROM. This returns once the start request
    /// is through, or the reset that follows it. It does not wait for the
    /// image to come back: which stage answers next depends on the image. The
    /// caller waits with [`await_restart`](Self::await_restart) and asks the
    /// stage again. `address` is the caller's (dev's notes record 0x80360000 and
    /// 0x80300000); it is never picked here.
    ///
    /// The device is selected by its port path and must hold `expected` there:
    /// another identity on that port is refused before anything is claimed.
    pub async fn start_image(
        &self,
        location: &str,
        expected: UsbId,
        image: &[u8],
        address: u32,
    ) -> Result<(), DriverError> {
        bootrom::check_address(address).map_err(incompatible)?;
        let port = port_of(location)?;
        self.check_id(expected)?;
        let Opened { mut io, .. } = open_expected(
            self.bus.as_ref(),
            &port,
            Expect::Exactly(expected),
            &self.options,
            Duration::ZERO,
        )
        .await
        .into_result()
        .map_err(DriverError::Flash)?;
        let stage = match bootrom::stage(&mut *io).await {
            Ok(stage) => stage,
            Err(error) => {
                return Err(self
                    .observed(
                        &port,
                        Expect::Exactly(expected),
                        "asking the boot ROM its stage",
                        error,
                        false,
                    )
                    .await);
            }
        };
        match stage {
            Stage::BootRom => {}
            Stage::Burner => {
                return Err(DriverError::Incompatible(
                    "the device already runs a burner; start the image from a fresh boot ROM"
                        .into(),
                ));
            }
            Stage::Unknown(text) => {
                return Err(DriverError::Incompatible(format!(
                    "the device answered {text:?}, not a K230 in download mode"
                )));
            }
        }
        match bootrom::load_and_start(&mut *io, image, address).await {
            Ok(()) => Ok(()),
            Err(error) => Err(self
                .observed(
                    &port,
                    Expect::Exactly(expected),
                    "loading the image into the boot ROM",
                    error,
                    true,
                )
                .await),
        }
    }

    /// Waits for the device at `location` to come back after a start, for at
    /// most `timeout`. Returns the stage it answers. The caller decides
    /// whether that is the image it started. Any identity may come back: the
    /// image decides which stage answers, so only the port selects the device.
    pub async fn await_restart(
        &self,
        location: &str,
        timeout: Duration,
    ) -> Result<Stage, DriverError> {
        let port = port_of(location)?;
        let Opened { mut io, .. } = open_expected(
            self.bus.as_ref(),
            &port,
            Expect::AnyIdentity,
            &self.options,
            timeout,
        )
        .await
        .into_result()
        .map_err(DriverError::Flash)?;
        match bootrom::stage(&mut *io).await {
            Ok(stage) => Ok(stage),
            Err(error) => Err(self
                .observed(
                    &port,
                    Expect::AnyIdentity,
                    "asking the stage its name",
                    error,
                    false,
                )
                .await),
        }
    }

    fn check_id(&self, expected: UsbId) -> Result<(), DriverError> {
        if self.ids.contains(&expected) {
            Ok(())
        } else {
            Err(DriverError::Incompatible(format!(
                "{expected} is not an id this K230 driver handles"
            )))
        }
    }

    /// A transfer error as a typed stop: what the bus shows on the port decides.
    async fn observed(
        &self,
        port: &PortPath,
        expect: Expect,
        step: &str,
        error: UsbError,
        may_have_applied: bool,
    ) -> DriverError {
        DriverError::Flash(
            observe_stop(
                self.bus.as_ref(),
                port,
                expect,
                step,
                self.options.timeouts.bulk,
                may_have_applied,
                error,
            )
            .await,
        )
    }
}

impl std::fmt::Debug for K230Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("K230Driver")
            .field("manifest", &self.manifest)
            .field("ids", &self.ids)
            .finish_non_exhaustive()
    }
}

/// The port path a location names, refused when it is not one.
fn port_of(location: &str) -> Result<PortPath, DriverError> {
    PortPath::from_str(location).map_err(|error| DriverError::Incompatible(error.to_string()))
}

/// A transfer error as the driver reports it. An access problem carries its
/// fix; a device that answered unexpectedly is incompatible; the rest say the
/// device did not respond.
fn unreachable(error: UsbError) -> DriverError {
    match error {
        UsbError::Access { message, fix } => DriverError::Unreachable(format!("{message} {fix}")),
        UsbError::Protocol(message) => DriverError::Incompatible(message),
        other => DriverError::Unreachable(other.to_string()),
    }
}

fn incompatible(error: UsbError) -> DriverError {
    DriverError::Incompatible(error.to_string())
}

#[async_trait]
impl Driver for K230Driver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(self
            .matching()
            .await?
            .into_iter()
            .map(|device| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: device.location,
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let port = port_of(&candidate.address)?;
        let Opened { mut io, device } = open_among(
            self.bus.as_ref(),
            &port,
            &self.ids,
            &self.options,
            Duration::ZERO,
        )
        .await
        .into_result()
        .map_err(DriverError::Flash)?;
        let stage = bootrom::stage(&mut *io).await.map_err(unreachable)?;
        let stage_name = match stage {
            Stage::BootRom => "boot-rom",
            Stage::Burner => "burner",
            Stage::Unknown(text) => {
                return Err(DriverError::Incompatible(format!(
                    "the device answered {text:?}, not a K230 in download mode"
                )));
            }
        };
        // The serial when the device has one, else its port: the port is
        // stable across the boot ROM's reset, a serial is not always there.
        let serial = device
            .serial
            .clone()
            .unwrap_or_else(|| format!("usb-{}", device.location));
        Ok(Identity {
            key: DeviceKey::board(FAMILY, serial)
                .map_err(|error| DriverError::Incompatible(error.reason))?,
            model: Some("Kendryte K230 in USB download mode".into()),
            state: DeviceState::BootRom,
            lemnos: atlas_types::LemnosState::Absent,
            versions: Default::default(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: [("stage".to_string(), stage_name.to_string())].into(),
        })
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds([CapabilityKind::Flash])
    }
}
