//! The Allwinner FEL driver. It finds Allwinner devices in their USB boot ROM
//! (FEL), says what each one is, and grants `Flash` there. The flash itself
//! is `atlas-fel`; this crate is the contract between it and Atlas.
//!
//! The USB vendor and product ids come in from the caller ([`UsbId`]), as
//! data: this crate names no board. The flash images (the SPL, the image to
//! load after it), the SRAM layout and the expected SoC id come in as an
//! [`Images`]. Without one the driver still identifies devices, read-only,
//! but grants no `Flash`: the layout is not sourced for every SoC.
//!
//! The FEL flow is untested on a board. The A733's id, its layout and its
//! thunk's instruction set are UNCONFIRMED (see `atlas-fel`).
//!
//! This crate depends on no other driver.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_fel::client::Fel;
use atlas_fel::flash::{FlashError, Report, flash};
use atlas_fel::layout::Layout;
use atlas_fel::protocol::MODE_FEL;
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkKind,
};
use atlas_usb::{OpenOptions, UsbBus, UsbError, UsbId};

/// The family every Allwinner FEL device belongs to: the chip's download
/// mode, not a board.
pub const FAMILY: &str = "allwinner-fel";

/// What a flash loads. The caller reads the files and the layout (see
/// `atlas-fel`'s `Layout::from_value`) and passes them in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Images {
    pub layout: Layout,
    /// The SPL, loaded into SRAM through the thunk.
    pub spl: Vec<u8>,
    /// The image loaded to DRAM after the SPL, and run.
    pub image: Vec<u8>,
    /// The SoC id the description expects. A device reporting another id is
    /// refused before any write.
    pub expected_soc: Option<u32>,
}

pub struct FelDriver {
    manifest: DriverManifest,
    bus: Arc<dyn UsbBus>,
    ids: Vec<UsbId>,
    images: Option<Images>,
}

impl FelDriver {
    /// A driver for the devices with `ids`, on `bus`. `images` is what a
    /// flash loads; `None` leaves the driver read-only.
    pub fn new(bus: Arc<dyn UsbBus>, ids: Vec<UsbId>, images: Option<Images>) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(FAMILY),
                name: "Allwinner FEL".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::UsbBoot],
                priority: 0,
            },
            bus,
            ids,
            images,
        }
    }

    /// Flashes the device a candidate names: the SPL through the thunk, then
    /// the image. Needs an [`Images`]; refused otherwise.
    pub async fn flash(&self, candidate: &Candidate) -> Result<Report, DriverError> {
        let images = self.images.as_ref().ok_or_else(|| {
            DriverError::Incompatible(
                "no flash images are set: the description gives no SRAM layout for this SoC \
                 (UNCONFIRMED for the A733)"
                    .into(),
            )
        })?;
        let mut io = self
            .bus
            .open(&candidate.address, &OpenOptions::default())
            .await
            .map_err(usb_error)?;
        flash(
            io.as_mut(),
            &images.layout,
            &images.spl,
            &images.image,
            images.expected_soc,
        )
        .await
        .map_err(flash_error)
    }
}

impl fmt::Debug for FelDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FelDriver")
            .field("manifest", &self.manifest)
            .field("ids", &self.ids)
            .field("flashable", &self.images.is_some())
            .finish_non_exhaustive()
    }
}

fn usb_error(error: UsbError) -> DriverError {
    match error {
        UsbError::Access { message, fix } => DriverError::Unsupported(format!("{message} {fix}")),
        UsbError::Other(message) => DriverError::Other(message),
        other => DriverError::Unreachable(other.to_string()),
    }
}

fn flash_error(error: FlashError) -> DriverError {
    match error {
        FlashError::Usb(error) => usb_error(error),
        FlashError::ReturnToFel(_) | FlashError::Readback(_) => {
            DriverError::Other(error.to_string())
        }
        FlashError::NotFel { .. }
        | FlashError::WrongSoc { .. }
        | FlashError::Layout(_)
        | FlashError::Table(_) => DriverError::Incompatible(error.to_string()),
    }
}

#[async_trait]
impl Driver for FelDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if !matches!(link.kind, LinkKind::UsbBoot) {
            return Ok(Vec::new());
        }
        let devices = self.bus.devices().await.map_err(usb_error)?;
        Ok(devices
            .iter()
            .filter(|device| self.ids.iter().any(|id| id.matches(device)))
            .map(|device| Candidate {
                link: link.id.clone(),
                family: self.manifest.family.clone(),
                address: device.location.clone(),
            })
            .collect())
    }

    /// Opens the device and asks its SoC id and mode. Read-only.
    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let mut io = self
            .bus
            .open(&candidate.address, &OpenOptions::default())
            .await
            .map_err(usb_error)?;
        let verify = Fel::new(io.as_mut()).verify().await.map_err(usb_error)?;
        if verify.mode != MODE_FEL {
            return Err(DriverError::Incompatible(format!(
                "the device at {} reports mode {}, not FEL ({MODE_FEL}); put it in FEL and try again",
                candidate.address, verify.mode
            )));
        }
        Ok(Identity {
            key: DeviceKey::new(FAMILY, format!("usb-{}", candidate.address)),
            model: "Allwinner SoC in FEL".into(),
            state: DeviceState::BootRom,
            versions: BTreeMap::new(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: BTreeMap::from([
                ("soc-id".to_string(), format!("{:#010x}", verify.board)),
                ("usb-location".to_string(), candidate.address.clone()),
            ]),
        })
    }

    /// `Flash` in the boot ROM, and only when a flash has its images.
    fn capabilities(&self, device: &Identity) -> CapabilitySet {
        if device.state == DeviceState::BootRom && self.images.is_some() {
            CapabilitySet::from_kinds([CapabilityKind::Flash])
        } else {
            CapabilitySet::new()
        }
    }
}
