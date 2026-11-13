//! The real bus, on nusb. The same code runs on Linux, Windows and macOS;
//! where an operating system differs, the difference is a runtime check on
//! the error it reports, not a separate code path.

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_core::Stream;
use nusb::descriptors::{TransferType, language_id};
use nusb::hotplug::HotplugWatch;
use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, Direction, In, Out, Recipient, TransferError,
};
use nusb::{Device, DeviceId, DeviceInfo, Endpoint, ErrorKind, GetDescriptorError, Interface};

use crate::bus::UsbBus;
use crate::gauge::{Gauges, Kind, Tracked};
use crate::hotplug::HotplugEvent;
use crate::{
    DeviceSummary, EndpointKind, EndpointSummary, InterfaceSummary, OpenOptions, Timeouts,
    UsbError, UsbIo,
};

/// The USB bus of this machine.
#[derive(Clone, Debug)]
pub struct NusbBus {
    gauges: Gauges,
}

impl NusbBus {
    /// A bus counted in [`Gauges::global`].
    pub fn new() -> Self {
        Self::with_gauges(Gauges::global())
    }

    /// A bus counted in `gauges`.
    pub fn with_gauges(gauges: Gauges) -> Self {
        Self { gauges }
    }
}

impl Default for NusbBus {
    fn default() -> Self {
        Self::new()
    }
}

/// `1-2.3`: the bus and port path.
fn location_of(info: &DeviceInfo) -> String {
    let ports: Vec<String> = info.port_chain().iter().map(u8::to_string).collect();
    format!("{}-{}", info.bus_id(), ports.join("."))
}

fn summary(info: &DeviceInfo) -> DeviceSummary {
    let summary = DeviceSummary::new(location_of(info), info.vendor_id(), info.product_id())
        .with_versions(info.usb_version(), info.device_version());
    match info
        .serial_number()
        .map(str::trim)
        .filter(|serial| !serial.is_empty())
    {
        Some(serial) => summary.with_serial(serial),
        None => summary,
    }
}

async fn list() -> Result<Vec<DeviceInfo>, UsbError> {
    let devices = nusb::list_devices()
        .await
        .map_err(|error| UsbError::Other(error.to_string()))?;
    Ok(devices.collect())
}

fn access_error(error: &nusb::Error, location: &str) -> UsbError {
    match error.kind() {
        ErrorKind::PermissionDenied => UsbError::Access {
            message: format!("No permission to open the USB device at {location}."),
            fix: if cfg!(target_os = "linux") {
                "Install a udev rule that grants access to this device, then replug it.".into()
            } else {
                "Run with rights that can open USB devices, then replug it.".into()
            },
        },
        ErrorKind::Busy => UsbError::Access {
            message: format!("Another program is using the USB device at {location}."),
            fix: "Close the other program that holds the device and try again.".into(),
        },
        ErrorKind::Unsupported if cfg!(target_os = "windows") => UsbError::Access {
            message: format!("Windows has no WinUSB driver bound to the USB device at {location}."),
            fix: "Bind WinUSB to the device, then replug it.".into(),
        },
        ErrorKind::Disconnected => UsbError::Disconnected,
        _ => UsbError::Other(error.to_string()),
    }
}

/// Reads one interface string. nusb 0.2.7 reads it from the device with
/// the descriptor request, on Linux, Windows and macOS alike. It also keeps
/// the string the OS cached when it enumerated the device
/// (`InterfaceInfo::interface_string`): sysfs on Linux, `kUSBString` on
/// macOS, and on Windows only for a composite device under `usbccgp`. The
/// device's own answer is used, because the cache is missing on some systems.
async fn interface_string(
    device: &Device,
    index: std::num::NonZeroU8,
    timeout: std::time::Duration,
) -> Result<String, UsbError> {
    let request = device.get_string_descriptor(index, language_id::US_ENGLISH, timeout);
    let answer = tokio::time::timeout(timeout, request)
        .await
        .map_err(|_| UsbError::Timeout("the interface string request".into()))?;
    answer.map_err(|error| match error {
        GetDescriptorError::Transfer(error) => transfer_error(error),
        GetDescriptorError::InvalidDescriptor => {
            UsbError::Protocol("the interface string is not a valid string descriptor".into())
        }
    })
}

fn endpoint_kind(kind: TransferType) -> EndpointKind {
    match kind {
        TransferType::Control => EndpointKind::Control,
        TransferType::Isochronous => EndpointKind::Isochronous,
        TransferType::Bulk => EndpointKind::Bulk,
        TransferType::Interrupt => EndpointKind::Interrupt,
    }
}

fn transfer_error(error: TransferError) -> UsbError {
    match error {
        TransferError::Disconnected => UsbError::Disconnected,
        TransferError::Stall => UsbError::Stall,
        TransferError::Cancelled => UsbError::Timeout("a USB transfer".into()),
        other => UsbError::Other(other.to_string()),
    }
}

#[async_trait]
impl UsbBus for NusbBus {
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
        Ok(list().await?.iter().map(summary).collect())
    }

    async fn interfaces(
        &self,
        location: &str,
        timeouts: Timeouts,
    ) -> Result<Vec<InterfaceSummary>, UsbError> {
        let info = list()
            .await?
            .into_iter()
            .find(|info| location_of(info) == location)
            .ok_or_else(|| UsbError::NotFound(location.to_string()))?;
        let device = info
            .open()
            .await
            .map_err(|error| access_error(&error, location))?;
        let config = device
            .active_configuration()
            .map_err(|error| UsbError::Other(error.to_string()))?;
        let mut interfaces = Vec::new();
        for interface in config.interfaces() {
            // The default setting: alternate settings are not listed.
            let Some(alt) = interface.alt_settings().next() else {
                continue;
            };
            let mut summary = InterfaceSummary::new(
                alt.interface_number(),
                alt.class(),
                alt.subclass(),
                alt.protocol(),
            );
            if let Some(index) = alt.string_index() {
                summary =
                    summary.with_string(interface_string(&device, index, timeouts.control).await?);
            }
            for endpoint in alt.endpoints() {
                summary = summary.with_endpoint(EndpointSummary::new(
                    endpoint.address(),
                    endpoint_kind(endpoint.transfer_type()),
                ));
            }
            interfaces.push(summary);
        }
        Ok(interfaces)
    }

    async fn open(
        &self,
        location: &str,
        options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        let info = list()
            .await?
            .into_iter()
            .find(|info| location_of(info) == location)
            .ok_or_else(|| UsbError::NotFound(location.to_string()))?;
        let device = info
            .open()
            .await
            .map_err(|error| access_error(&error, location))?;
        let config = device
            .active_configuration()
            .map_err(|error| UsbError::Other(error.to_string()))?;

        // The first interface (or the one asked for) with a bulk endpoint,
        // and its bulk OUT and IN endpoints if it has them.
        let mut found = None;
        'search: for interface in config.interfaces() {
            for alt in interface.alt_settings() {
                if options
                    .interface
                    .is_some_and(|n| n != alt.interface_number())
                {
                    continue;
                }
                let bulk = |direction| {
                    alt.endpoints()
                        .find(|ep| {
                            ep.transfer_type() == TransferType::Bulk && ep.direction() == direction
                        })
                        .map(|ep| ep.address())
                };
                let (out, input) = (bulk(Direction::Out), bulk(Direction::In));
                if out.is_some() || input.is_some() {
                    found = Some((alt.interface_number(), out, input));
                    break 'search;
                }
            }
        }
        let (number, out_address, in_address) = found.ok_or_else(|| {
            UsbError::Protocol(format!("the device at {location} has no bulk endpoint"))
        })?;

        // Detaches a kernel driver where the OS has one (Linux); elsewhere
        // this is a plain claim.
        let interface = device
            .detach_and_claim_interface(number)
            .await
            .map_err(|error| access_error(&error, location))?;
        let out = out_address
            .map(|address| interface.endpoint::<Bulk, Out>(address))
            .transpose()
            .map_err(|error| UsbError::Other(error.to_string()))?;
        let input = in_address
            .map(|address| interface.endpoint::<Bulk, In>(address))
            .transpose()
            .map_err(|error| UsbError::Other(error.to_string()))?;
        let io: Box<dyn UsbIo> = Box::new(NusbIo {
            interface,
            out,
            input,
            timeouts: options.timeouts,
            _tracked: Tracked::new(&self.gauges, Kind::Device),
        });
        Ok(io)
    }
}

/// An open, claimed device. Dropping it releases the interface.
pub struct NusbIo {
    interface: Interface,
    out: Option<Endpoint<Bulk, Out>>,
    input: Option<Endpoint<Bulk, In>>,
    timeouts: Timeouts,
    _tracked: Tracked,
}

impl std::fmt::Debug for NusbIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NusbIo")
            .field("timeouts", &self.timeouts)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl UsbIo for NusbIo {
    fn max_packet_out(&self) -> usize {
        self.out
            .as_ref()
            .map_or(512, |out| out.max_packet_size().max(1))
    }

    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        self.interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request,
                    value,
                    index,
                    data,
                },
                self.timeouts.control,
            )
            .await
            .map_err(transfer_error)
    }

    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        self.interface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request,
                    value,
                    index,
                    length,
                },
                self.timeouts.control,
            )
            .await
            .map_err(transfer_error)
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        let timeout = self.timeouts.bulk;
        let out = self
            .out
            .as_mut()
            .ok_or_else(|| UsbError::Protocol("the device has no bulk OUT endpoint".into()))?;
        out.submit(Buffer::from(data.to_vec()));
        let Ok(completion) = tokio::time::timeout(timeout, out.next_complete()).await else {
            // Cancel, then wait for the cancelled transfer so the endpoint is
            // clean for the next one.
            out.cancel_all();
            while out.pending() > 0 {
                let _ = out.next_complete().await;
            }
            return Err(UsbError::Timeout("a bulk write".into()));
        };
        completion.status.map_err(transfer_error)?;
        if completion.actual_len != data.len() {
            return Err(UsbError::Protocol(format!(
                "took {} of {} bytes",
                completion.actual_len,
                data.len()
            )));
        }
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        if max == 0 {
            return Ok(Vec::new());
        }
        let timeout = self.timeouts.bulk;
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| UsbError::Protocol("the device has no bulk IN endpoint".into()))?;
        // IN transfers are whole packets.
        let packet = input.max_packet_size().max(1);
        let buffer = input.allocate(max.div_ceil(packet) * packet);
        input.submit(buffer);
        let Ok(completion) = tokio::time::timeout(timeout, input.next_complete()).await else {
            input.cancel_all();
            while input.pending() > 0 {
                let _ = input.next_complete().await;
            }
            return Err(UsbError::Timeout("a bulk read".into()));
        };
        completion.status.map_err(transfer_error)?;
        let mut data = completion.buffer.into_vec();
        data.truncate(completion.actual_len.min(max));
        Ok(data)
    }
}

/// Every hotplug event of the bus as a [`HotplugEvent`]. nusb reports a
/// departure by id alone, so this remembers each arrival's location.
pub(crate) fn usb_events() -> Result<NusbEvents, UsbError> {
    let watch = nusb::watch_devices().map_err(|error| UsbError::Other(error.to_string()))?;
    Ok(NusbEvents {
        watch: Box::pin(watch),
        known: HashMap::new(),
    })
}

pub(crate) struct NusbEvents {
    watch: Pin<Box<HotplugWatch>>,
    known: HashMap<DeviceId, String>,
}

impl Stream for NusbEvents {
    type Item = HotplugEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<HotplugEvent>> {
        loop {
            let event = match self.watch.as_mut().poll_next(cx) {
                Poll::Ready(Some(event)) => event,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            match event {
                nusb::hotplug::HotplugEvent::Connected(info) => {
                    let summary = summary(&info);
                    self.known.insert(info.id(), summary.location.clone());
                    return Poll::Ready(Some(HotplugEvent::Attached(summary)));
                }
                nusb::hotplug::HotplugEvent::Disconnected(id) => {
                    if let Some(location) = self.known.remove(&id) {
                        return Poll::Ready(Some(HotplugEvent::Detached { location }));
                    }
                }
            }
        }
    }
}
