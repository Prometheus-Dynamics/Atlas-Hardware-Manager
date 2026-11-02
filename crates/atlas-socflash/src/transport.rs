//! The USB a SoC's download mode needs: vendor control requests to the
//! device, and one bulk OUT and one bulk IN endpoint. [`DeviceIo`] is the
//! real one (lemnos-usb); tests drive the protocols through [`Script`], which checks
//! every transfer against a recorded trace.

use std::time::Duration;

use async_trait::async_trait;
pub use lemnos_usb::UsbDeviceInfo as DeviceInfo;
use lemnos_usb::{ControlSetup, Direction, ErrorKind, Interface, TransferType, UsbDevice};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsbError {
    Disconnected,
    Timeout(String),
    /// Opening the device failed in a way the user can fix.
    Access {
        message: String,
        fix: String,
    },
    /// The device answered, but not as the protocol says.
    Protocol(String),
    Other(String),
}

impl std::fmt::Display for UsbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("the board disconnected from USB"),
            Self::Timeout(what) => write!(f, "{what} timed out"),
            Self::Access { message, fix } => write!(f, "{message} {fix}"),
            Self::Protocol(message) => write!(f, "the board answered unexpectedly: {message}"),
            Self::Other(message) => write!(f, "USB error: {message}"),
        }
    }
}

impl std::error::Error for UsbError {}

/// One device's download interface.
#[async_trait]
pub trait UsbIo: Send {
    /// A vendor request to the device with `data` as its data stage.
    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError>;
    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError>;
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError>;
    /// One transfer of at most `max` bytes.
    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError>;
    /// The bulk OUT endpoint's packet size (a write that fills its last
    /// packet exactly ends with a zero-length one).
    fn max_packet_out(&self) -> usize {
        512
    }
}

/// `1-2.3`: the bus and port path, stable while the board stays in the same
/// port, across its re-enumeration between download stages.
pub fn location(info: &DeviceInfo) -> String {
    info.location().to_string()
}

/// Every USB device, listed off the async threads.
pub(crate) async fn list_all() -> Result<Vec<DeviceInfo>, UsbError> {
    tokio::task::spawn_blocking(lemnos_usb::list_devices)
        .await
        .map_err(|error| UsbError::Other(error.to_string()))?
        .map_err(|error| UsbError::Other(error.to_string()))
}

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

fn transfer_error(error: lemnos_usb::Error) -> UsbError {
    match error.kind() {
        ErrorKind::Disconnected => UsbError::Disconnected,
        _ => UsbError::Other(error.to_string()),
    }
}

fn access_error(error: &lemnos_usb::Error, what: &str) -> UsbError {
    match error.kind() {
        ErrorKind::PermissionDenied => UsbError::Access {
            message: format!("No permission to open the {what}."),
            fix: if cfg!(target_os = "linux") {
                "Install Atlas's USB rules (Settings > Host health, Fix on USB access), then replug the board."
                    .into()
            } else {
                "Run Atlas as an administrator, or bind WinUSB to the device.".into()
            },
        },
        ErrorKind::Busy => UsbError::Access {
            message: format!("Another program is using the {what}."),
            fix: "Close rkdeveloptool, RKDevTool, upgrade_tool or the K230 burning tool and try again."
                .into(),
        },
        ErrorKind::Unsupported if cfg!(target_os = "windows") => UsbError::Access {
            message: format!("Windows has no WinUSB driver bound to the {what}."),
            fix: "Use Fix in Settings > Host health (or reinstall Atlas) to install the driver, then replug the board."
                .into(),
        },
        ErrorKind::Disconnected => UsbError::Disconnected,
        _ => UsbError::Other(error.to_string()),
    }
}

/// A real device, its first interface with a bulk endpoint pair claimed.
pub struct DeviceIo {
    device: UsbDevice,
    /// Held for the claim; dropping it releases the interface.
    _interface: Interface,
    out: u8,
    out_packet: usize,
    input: u8,
    in_packet: usize,
    bulk_timeout: Duration,
}

impl DeviceIo {
    /// Opens `info`, `what` naming it in errors ("Rockchip maskrom device").
    pub async fn open(
        info: &DeviceInfo,
        what: &str,
        bulk_timeout: Duration,
    ) -> Result<Self, UsbError> {
        let info = info.clone();
        let what = what.to_string();
        tokio::task::spawn_blocking(move || Self::open_blocking(&info, &what, bulk_timeout))
            .await
            .map_err(|error| UsbError::Other(error.to_string()))?
    }

    fn open_blocking(
        info: &DeviceInfo,
        what: &str,
        bulk_timeout: Duration,
    ) -> Result<Self, UsbError> {
        let device = UsbDevice::open(info).map_err(|error| access_error(&error, what))?;
        let config = device
            .active_configuration()
            .map_err(|error| UsbError::Other(error.to_string()))?;
        // The first interface with both bulk directions.
        let found = config.interfaces.iter().find_map(|interface| {
            interface.alternates.iter().find_map(|alt| {
                let bulk = |direction| {
                    alt.endpoints.iter().find(|ep| {
                        ep.transfer_type == TransferType::Bulk && ep.direction == direction
                    })
                };
                Some((
                    interface.number,
                    *bulk(Direction::Out)?,
                    *bulk(Direction::In)?,
                ))
            })
        });
        let (number, out, input) = found
            .ok_or_else(|| UsbError::Protocol(format!("the {what} has no bulk endpoint pair")))?;
        // Linux unbinds a kernel driver first; elsewhere a plain claim.
        let interface = if cfg!(target_os = "linux") {
            device.detach_and_claim_interface(number)
        } else {
            device.claim_interface(number)
        }
        .map_err(|error| access_error(&error, what))?;
        Ok(Self {
            device,
            _interface: interface,
            out: out.address,
            out_packet: usize::from(out.max_packet_size).max(1),
            input: input.address,
            in_packet: usize::from(input.max_packet_size).max(1),
            bulk_timeout,
        })
    }
}

#[async_trait]
impl UsbIo for DeviceIo {
    fn max_packet_out(&self) -> usize {
        self.out_packet
    }

    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        let device = self.device.clone();
        let setup = ControlSetup::vendor(Direction::Out, request, value, index);
        let data = data.to_vec();
        device
            .submit_control_out(setup, data, CONTROL_TIMEOUT)
            .map_err(transfer_error)?
            .await
            .map(drop)
            .map_err(transfer_error)
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        let device = self.device.clone();
        let setup = ControlSetup::vendor(Direction::In, request, value, index);
        let outcome = device
            .submit_control_in(setup, length, CONTROL_TIMEOUT)
            .map_err(transfer_error)?
            .await
            .map_err(transfer_error)?;
        Ok(outcome.data().to_vec())
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let mut transfer = self
            .device
            .submit_bulk_out(self.out, data.to_vec())
            .map_err(transfer_error)?;
        let Ok(outcome) = tokio::time::timeout(self.bulk_timeout, &mut transfer).await else {
            transfer.cancel();
            return Err(UsbError::Timeout("a bulk write".into()));
        };
        let outcome = outcome.map_err(transfer_error)?;
        if outcome.actual_length != data.len() {
            return Err(UsbError::Protocol(format!(
                "took {} of {} bytes",
                outcome.actual_length,
                data.len()
            )));
        }
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        // IN transfers are whole packets.
        let buffer = vec![0; max.div_ceil(self.in_packet) * self.in_packet];
        let mut transfer = self
            .device
            .submit_bulk_in(self.input, buffer)
            .map_err(transfer_error)?;
        let Ok(outcome) = tokio::time::timeout(self.bulk_timeout, &mut transfer).await else {
            transfer.cancel();
            return Err(UsbError::Timeout("a bulk read".into()));
        };
        let outcome = outcome.map_err(transfer_error)?;
        let length = outcome.actual_length.min(max);
        let mut data = outcome.into_buffer().into_vec();
        data.truncate(length);
        Ok(data)
    }
}

/// A recorded trace a test plays the protocol against: each step is what
/// the host must send, and for reads what the device answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    ControlOut {
        request: u8,
        value: u16,
        index: u16,
        data: Vec<u8>,
    },
    ControlIn {
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        reply: Vec<u8>,
    },
    BulkOut(Vec<u8>),
    BulkIn {
        max: usize,
        reply: Vec<u8>,
    },
    /// The device drops off USB at this point.
    Disconnect,
}

/// Plays a trace; any transfer that differs from the next step fails the
/// test with both shown.
#[derive(Debug, Default)]
pub struct Script {
    steps: std::collections::VecDeque<Step>,
    pub done: Vec<Step>,
}

impl Script {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: steps.into(),
            done: Vec::new(),
        }
    }

    pub fn finished(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.steps.len()
    }

    fn next(&mut self, sent: &Step) -> Result<Step, UsbError> {
        let Some(step) = self.steps.pop_front() else {
            return Err(UsbError::Protocol(format!(
                "the trace ended; the host sent {sent:?}"
            )));
        };
        if step == Step::Disconnect {
            return Err(UsbError::Disconnected);
        }
        self.done.push(step.clone());
        Ok(step)
    }
}

fn mismatch(expected: &Step, sent: &Step) -> UsbError {
    UsbError::Protocol(format!("trace expected {expected:?}, host sent {sent:?}"))
}

#[async_trait]
impl UsbIo for Script {
    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        let sent = Step::ControlOut {
            request,
            value,
            index,
            data: data.to_vec(),
        };
        let step = self.next(&sent)?;
        if step != sent {
            return Err(mismatch(&step, &sent));
        }
        Ok(())
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        let sent = Step::ControlIn {
            request,
            value,
            index,
            length,
            reply: Vec::new(),
        };
        match self.next(&sent)? {
            Step::ControlIn {
                request: r,
                value: v,
                index: i,
                length: l,
                reply,
            } if (r, v, i, l) == (request, value, index, length) => Ok(reply),
            step => Err(mismatch(&step, &sent)),
        }
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let sent = Step::BulkOut(data.to_vec());
        let step = self.next(&sent)?;
        if step != sent {
            return Err(mismatch(&step, &sent));
        }
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        let sent = Step::BulkIn {
            max,
            reply: Vec::new(),
        };
        match self.next(&sent)? {
            Step::BulkIn { max: m, reply } if m == max => Ok(reply),
            step => Err(mismatch(&step, &sent)),
        }
    }
}
