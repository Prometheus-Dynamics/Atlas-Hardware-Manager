//! The Pi's loader in the flash flow (D6 step 3). The Pi has no loader in the Rockchip sense: its boot files start
//! the flasher directly. So the loader is the two USB boot rounds through [`RpiDriver::boot`], serving the flasher
//! image's boot set. They run in RAM and write nothing to the board's storage, so a try may be made again after any
//! failure.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError};
use atlas_flash_flow::{Loaded, Loader, Say, StepError, Target};
use atlas_rpiboot::{BootEvent, FileServerOutcome};
use atlas_types::{Candidate, Family, LinkId};

use crate::{FAMILY, RpiDriver};

/// The link the device list finds boot ROMs on; the driver reads only the candidate's address.
const USB_BOOT_LINK: &str = "usb-boot";

/// [`RpiDriver`] as the flow's [`Loader`].
#[derive(Debug, Clone)]
pub struct RpiLoader {
    driver: Arc<RpiDriver>,
}

impl RpiLoader {
    #[must_use]
    pub fn new(driver: Arc<RpiDriver>) -> Self {
        Self { driver }
    }
}

/// How a driver error is tried: what can change by itself is tried again, and a configuration or identity
/// problem waits for the person.
fn step_error(error: DriverError) -> StepError {
    match error {
        DriverError::Cancelled => StepError::Cancelled,
        DriverError::Unsupported(words) | DriverError::Incompatible(words) => {
            StepError::Refused(words)
        }
        other => StepError::Again(other.to_string()),
    }
}

#[async_trait]
impl Loader for RpiLoader {
    fn family(&self) -> &str {
        FAMILY
    }

    fn label(&self) -> &str {
        "Raspberry Pi USB boot"
    }

    async fn load(&self, target: &Target, say: &Say<'_>) -> Result<Loaded, StepError> {
        let candidate = Candidate {
            link: LinkId::new(USB_BOOT_LINK),
            family: Family::new(FAMILY),
            address: target.port.clone(),
        };
        let identity = self.driver.identify(&candidate).await.map_err(step_error)?;
        // The port may hold another board by now: never boot a board that is not the target.
        if identity.key.to_string() != target.key {
            return Err(StepError::Refused(format!(
                "USB {} now holds {}, not {}: nothing was sent",
                target.port, identity.key, target.key
            )));
        }

        let facts = Mutex::new(Vec::new());
        let sent = Mutex::new(0usize);
        let on_event = |event: BootEvent| match event {
            BootEvent::SecondStage { chip, bytes } => {
                say(format!(
                    "{} took the second stage ({bytes} bytes)",
                    chip.label()
                ));
            }
            BootEvent::FileSent { name, bytes } => {
                *sent.lock().unwrap_or_else(PoisonError::into_inner) += 1;
                say(format!("sent {name} ({bytes} bytes)"));
            }
            BootEvent::FileMissing { name } => {
                say(format!("asked for {name}, which the flasher does not have"))
            }
            BootEvent::Retried { what } => say(format!("asked again: {what}")),
            BootEvent::Metadata { name, value } => {
                say(format!("the bootloader says {name} = {value}"));
                facts
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((name, value));
            }
            BootEvent::WaitingForDevice
            | BootEvent::FileRequested { .. }
            | BootEvent::FileServerDone => {}
        };
        // The flow cancels by dropping this future, so the boot's own check never needs to say yes.
        let outcome = self
            .driver
            .boot(&identity, &on_event, &|| false)
            .await
            .map_err(step_error)?;

        let sent = *sent.lock().unwrap_or_else(PoisonError::into_inner);
        let words = match outcome {
            FileServerOutcome::Done => {
                format!("the flasher's boot set was served ({sent} files) and the board took it")
            }
            FileServerOutcome::Disconnected => {
                format!(
                    "the flasher's boot set was served ({sent} files); the board left USB boot to start it"
                )
            }
        };
        let mut facts = facts.into_inner().unwrap_or_else(PoisonError::into_inner);
        if let Some(soc) = identity.attributes.get("soc") {
            facts.insert(0, ("SoC".into(), soc.clone()));
        }
        Ok(Loaded { words, facts })
    }
}
