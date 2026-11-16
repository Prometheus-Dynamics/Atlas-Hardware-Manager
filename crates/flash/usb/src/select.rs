//! Selecting a device by its port path, and reporting what a stop observed.
//!
//! A job names the port its device must be on and the identity it expects
//! there. [`open_expected`] opens the device on that port if, and only if, the
//! identity matches; it never opens anything on another port. When the device
//! re-enumerates on the same port with a new identity (a boot ROM handing over
//! to the stage it loaded), the next call names the new identity and the same
//! port: the selection follows the port.
//!
//! [`observe`] turns a transfer error into an [`Outcome`] by asking the bus who
//! is on the port now: a device still listed with the expected identity is
//! Deaf, nothing listed is Absent, and a different identity is a refusal.

use std::time::Duration;

use tokio::time::{Instant, sleep};

use crate::outcome::{Absent, Deaf, Expect, Outcome, Party, Reason, Refused, Stop, TimedOut};
use crate::{DeviceSummary, OpenOptions, PortPath, UsbBus, UsbError, UsbId, UsbIo};

const FIRST_RETRY: Duration = Duration::from_millis(100);
const MAX_RETRY: Duration = Duration::from_secs(5);

/// A device opened on the port it was selected by.
pub struct Opened {
    /// The device as the bus listed it when it was opened.
    pub device: DeviceSummary,
    pub io: Box<dyn UsbIo>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// The device at `port`, if the bus lists one there.
pub async fn at_port(bus: &dyn UsbBus, port: &PortPath) -> Result<Option<DeviceSummary>, UsbError> {
    let location = port.to_string();
    Ok(bus
        .devices()
        .await?
        .into_iter()
        .find(|device| device.location == location))
}

/// Opens the device on `port` with identity `expect`, waiting up to `wait` for
/// it to be there with that identity (a re-enumeration after a reset takes a
/// moment). `wait` of zero decides at once.
///
/// - Done: the device with the expected identity is open.
/// - Refused (host, wrong identity): the port holds another identity when the
///   wait ends. Nothing was opened.
/// - Refused (host, access): this machine may not open the device; the message
///   says how to fix it.
/// - Absent: nothing is on the port when the wait ends.
/// - TimedOut: the expected device is on the port but could not be opened
///   within the wait.
pub async fn open_expected(
    bus: &dyn UsbBus,
    port: &PortPath,
    expect: Expect,
    options: &OpenOptions,
    wait: Duration,
) -> Outcome<Opened> {
    let deadline = Instant::now() + wait;
    let location = port.to_string();
    let mut backoff = FIRST_RETRY;
    let mut transient: Option<UsbError> = None;
    loop {
        let listed = match at_port(bus, port).await {
            Ok(listed) => listed,
            Err(error) => return host_refusal(error),
        };
        if let Some(device) = listed
            .as_ref()
            .filter(|d| expect.accepts(d.vendor, d.product))
        {
            match bus.open_at(&location, expect.id(), options).await {
                Ok(io) => {
                    return Outcome::Done(Opened {
                        device: device.clone(),
                        io,
                    });
                }
                Err(error @ UsbError::Access { .. }) => return host_refusal(error),
                Err(error) => transient = Some(error),
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return match (listed, transient) {
                (None, _) => Outcome::Absent(Absent {
                    port: port.clone(),
                    looked_for: expect.id(),
                    during: "waiting for the device on its port".into(),
                    waited: wait,
                    may_have_applied: false,
                }),
                (Some(device), Some(_)) if expect.accepts(device.vendor, device.product) => {
                    Outcome::TimedOut(TimedOut {
                        port: port.clone(),
                        phase: "opening the device".into(),
                        bound: wait,
                        may_have_applied: false,
                    })
                }
                (Some(device), _) => wrong_identity(port, expect, &device),
            };
        }
        sleep(backoff.min(deadline - now)).await;
        backoff = (backoff * 2).min(MAX_RETRY);
    }
}

/// Turns a transfer error into the outcome it stands for. `step` says what the
/// host was doing (it is shown in the outcome), `bound` is the time the transfer
/// was allowed, and `may_have_applied` is true for a write or a start.
///
/// The bus is asked who is on the port now, so a silent device that is still
/// listed is Deaf, and a device that left the bus is Absent.
pub async fn observe<T>(
    bus: &dyn UsbBus,
    port: &PortPath,
    expect: Expect,
    step: &str,
    bound: Duration,
    may_have_applied: bool,
    error: UsbError,
) -> Outcome<T> {
    observe_stop(bus, port, expect, step, bound, may_have_applied, error)
        .await
        .typed()
}

/// [`observe`] for a driver that reports a stop: the same classification, as a
/// [`Stop`] that carries no value.
pub async fn observe_stop(
    bus: &dyn UsbBus,
    port: &PortPath,
    expect: Expect,
    step: &str,
    bound: Duration,
    may_have_applied: bool,
    error: UsbError,
) -> Stop {
    match error {
        UsbError::Disconnected | UsbError::NotFound(_) => Outcome::Absent(Absent {
            port: port.clone(),
            looked_for: expect.id(),
            during: step.into(),
            waited: bound,
            may_have_applied,
        }),
        UsbError::Timeout(_) => match at_port(bus, port).await {
            Ok(Some(device)) if expect.accepts(device.vendor, device.product) => {
                Outcome::Deaf(Deaf {
                    port: port.clone(),
                    expected: expect.id(),
                    sent: step.into(),
                    waited: bound,
                    may_have_applied,
                })
            }
            Ok(Some(device)) => wrong_identity(port, expect, &device),
            Ok(None) => Outcome::Absent(Absent {
                port: port.clone(),
                looked_for: expect.id(),
                during: step.into(),
                waited: bound,
                may_have_applied,
            }),
            Err(error) => host_refusal(error),
        },
        UsbError::Stall => Outcome::Refused(Refused {
            by: Party::Device,
            reason: Reason::Message(format!("the device stalled {step}")),
        }),
        UsbError::Protocol(message) => Outcome::Refused(Refused {
            by: Party::Device,
            reason: Reason::Message(message),
        }),
        UsbError::WrongIdentity {
            location,
            expected,
            found,
        } => Outcome::Refused(Refused {
            by: Party::Host,
            reason: Reason::WrongIdentity {
                location,
                expected,
                found,
            },
        }),
        error @ UsbError::Access { .. } => host_refusal(error),
        UsbError::Other(message) => Outcome::Refused(Refused {
            by: Party::Host,
            reason: Reason::Message(message),
        }),
    }
}

fn host_refusal<T>(error: UsbError) -> Outcome<T> {
    Outcome::Refused(Refused {
        by: Party::Host,
        reason: Reason::Message(match error {
            UsbError::Access { message, fix } => format!("{message} {fix}"),
            other => other.to_string(),
        }),
    })
}

fn wrong_identity<T>(port: &PortPath, expect: Expect, device: &DeviceSummary) -> Outcome<T> {
    let expected = expect.id().unwrap_or(UsbId {
        vendor: device.vendor,
        product: device.product,
    });
    Outcome::Refused(Refused {
        by: Party::Host,
        reason: Reason::WrongIdentity {
            location: port.clone(),
            expected,
            found: UsbId {
                vendor: device.vendor,
                product: device.product,
            },
        },
    })
}
