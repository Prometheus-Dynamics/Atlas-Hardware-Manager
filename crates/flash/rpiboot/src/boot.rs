//! Both rounds on one device: find it, send the second stage, wait out the
//! reset, then serve the boot files.

use std::time::Duration;

use std::str::FromStr;

use atlas_usb::{
    Expect, OpenOptions, Opened, Outcome, Party, PortPath, Reason, Refused, UsbBus, UsbId,
    observe_stop, open_expected,
};

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

/// Boots the device on port `location` with the boot files `files`: the second
/// stage, then the file server. Returns how the file server ended. The
/// device resets between the rounds and comes back on the same port, so the
/// second round selects it by its port alone.
///
/// Round one needs `expected` on the port: another identity there is refused
/// and nothing is sent. Missing second-stage files are reported before the
/// device is touched. `cancelled` is checked between steps and before every
/// file request. A bus stop is [`RpiBootError::Stop`], with what was observed.
#[allow(clippy::too_many_arguments)]
pub async fn boot_device(
    bus: &dyn UsbBus,
    location: &str,
    expected: UsbId,
    chip: Chip,
    files: &BootFiles,
    settings: &BootSettings,
    on_event: &(dyn Fn(BootEvent) + Send + Sync),
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<FileServerOutcome, RpiBootError> {
    let port = PortPath::from_str(location).map_err(|error| {
        RpiBootError::Stop(Outcome::Refused(Refused {
            by: Party::Host,
            reason: Reason::Message(error.to_string()),
        }))
    })?;
    let bootcode = files.second_stage(chip)?;
    if cancelled() {
        return Err(RpiBootError::Cancelled);
    }
    on_event(BootEvent::WaitingForDevice);
    let bound = settings.open.timeouts.bulk;

    let Opened { mut io, .. } = open_expected(
        bus,
        &port,
        Expect::Exactly(expected),
        &settings.open,
        settings.appear_timeout,
    )
    .await
    .into_result()
    .map_err(RpiBootError::Stop)?;
    observed(
        bus,
        &port,
        expected,
        "the second stage",
        bound,
        second_stage(io.as_mut(), &bootcode).await,
    )
    .await?;
    drop(io);
    on_event(BootEvent::SecondStage {
        chip,
        bytes: bootcode.len(),
    });

    // The boot ROM reset. Its old handle is dead; the port now holds the file
    // server, whatever its identity.
    if cancelled() {
        return Err(RpiBootError::Cancelled);
    }
    let Opened { mut io, .. } = open_expected(
        bus,
        &port,
        Expect::AnyIdentity,
        &settings.open,
        settings.appear_timeout,
    )
    .await
    .into_result()
    .map_err(RpiBootError::Stop)?;
    let outcome = file_server(io.as_mut(), files, chip, on_event, cancelled).await;
    observed_serve(bus, &port, bound, outcome).await
}

/// A USB error inside a round becomes the stop the bus shows on the port.
async fn observed<T>(
    bus: &dyn UsbBus,
    port: &PortPath,
    expected: UsbId,
    step: &str,
    bound: std::time::Duration,
    result: Result<T, RpiBootError>,
) -> Result<T, RpiBootError> {
    match result {
        Ok(value) => Ok(value),
        Err(RpiBootError::Usb(error)) => Err(RpiBootError::Stop(
            observe_stop(
                bus,
                port,
                Expect::Exactly(expected),
                step,
                bound,
                true,
                error,
            )
            .await,
        )),
        Err(other) => Err(other),
    }
}

async fn observed_serve(
    bus: &dyn UsbBus,
    port: &PortPath,
    bound: std::time::Duration,
    result: Result<FileServerOutcome, RpiBootError>,
) -> Result<FileServerOutcome, RpiBootError> {
    match result {
        Err(RpiBootError::Usb(error)) => Err(RpiBootError::Stop(
            observe_stop(
                bus,
                port,
                Expect::AnyIdentity,
                "serving the boot files",
                bound,
                true,
                error,
            )
            .await,
        )),
        other => other,
    }
}
