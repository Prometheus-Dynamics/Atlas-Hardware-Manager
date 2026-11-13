//! Both rounds on one device: find it, send the second stage, wait out the
//! reset, then serve the boot files.

use std::time::Duration;

use atlas_usb::{OpenOptions, UsbBus, await_device};

use crate::protocol::{BootEvent, FileServerOutcome, file_server, second_stage};
use crate::{BootFiles, Chip, RpiBootError};

/// How to reach the device in each round.
#[derive(Clone, Debug)]
pub struct BootSettings {
    /// The interface to claim. `None` takes the first with a bulk endpoint.
    pub open: OpenOptions,
    /// How long to wait for the device to appear, in each round.
    pub appear_timeout: Duration,
}

impl Default for BootSettings {
    fn default() -> Self {
        Self {
            open: OpenOptions::default(),
            appear_timeout: Duration::from_secs(30),
        }
    }
}

/// Boots the device at `location` with the boot files `files`: the second
/// stage, then the file server. Returns how the file server ended. The
/// device resets between the rounds and comes back at the same location, so
/// the second round waits for it with [`await_device`].
///
/// Missing second-stage files are reported before the device is touched.
/// `cancelled` is checked between steps and before every file request.
pub async fn boot_device(
    bus: &dyn UsbBus,
    location: &str,
    chip: Chip,
    files: &BootFiles,
    settings: &BootSettings,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<FileServerOutcome, RpiBootError> {
    let bootcode = files.second_stage(chip)?;
    if cancelled() {
        return Err(RpiBootError::Cancelled);
    }
    on_event(BootEvent::WaitingForDevice);

    let mut round_one =
        await_device(bus, location, &settings.open, settings.appear_timeout).await?;
    second_stage(round_one.as_mut(), &bootcode).await?;
    drop(round_one);
    on_event(BootEvent::SecondStage {
        chip,
        bytes: bootcode.len(),
    });

    // The boot ROM reset. Its old handle is dead; open the new enumeration.
    if cancelled() {
        return Err(RpiBootError::Cancelled);
    }
    let mut round_two =
        await_device(bus, location, &settings.open, settings.appear_timeout).await?;
    file_server(round_two.as_mut(), files, chip, on_event, cancelled).await
}
