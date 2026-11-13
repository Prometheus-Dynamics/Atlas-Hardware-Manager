use std::time::Duration;

use async_trait::async_trait;

use crate::UsbError;
use crate::link::LinkOptions;

/// The two halves of a split link.
pub type Halves = (Box<dyn BulkRead>, Box<dyn BulkWrite>);

/// One open device's transfers. A loader drives a boot ROM through this
/// trait only, so it can run against the simulated device in tests.
#[async_trait]
pub trait UsbIo: Send {
    /// A vendor request to the device, with `data` as its data stage.
    async fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError>;

    /// A vendor request that reads up to `length` bytes back.
    async fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError>;

    /// Writes all of `data` to the bulk OUT endpoint, or fails.
    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError>;

    /// One read of at most `max` bytes from the bulk IN endpoint. It may
    /// return fewer bytes; a short read is not an error.
    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError>;

    /// The bulk OUT endpoint's packet size. A write that fills its last
    /// packet exactly needs a zero-length packet after it.
    fn max_packet_out(&self) -> usize {
        512
    }

    /// Takes the claimed interface's bulk IN and OUT endpoints apart into two
    /// independent halves for a byte link (see [`crate::LinkOptions`]). Both
    /// halves keep the claim alive; it is released when both are dropped. A
    /// handle that cannot be split says so.
    fn into_halves(self: Box<Self>, _options: &LinkOptions) -> Result<Halves, UsbError> {
        Err(UsbError::Other(
            "this USB handle cannot be split into a byte link".into(),
        ))
    }
}

/// The read half of a claimed bulk IN endpoint. Each [`submit`](Self::submit)
/// queues one IN transfer of one max-packet-size read, so a transfer ends
/// with every packet and a read returns as soon as a packet arrives.
#[async_trait]
pub trait BulkRead: Send {
    /// The endpoint's max packet size. Every IN transfer asks for this many bytes.
    fn max_packet(&self) -> usize;

    /// Queues one IN transfer. Returns at once.
    fn submit(&mut self);

    /// Transfers queued and not yet completed.
    fn pending(&self) -> usize;

    /// Waits for the oldest queued transfer and returns its bytes. An empty
    /// result is a zero-length packet, not the end of the stream. Cancel-safe:
    /// dropping the future leaves the transfer queued.
    async fn complete(&mut self) -> Result<Vec<u8>, UsbError>;

    /// Cancels every queued transfer and waits until none is left. Bytes
    /// already received into them are dropped. Used when the reader is leaving.
    async fn cancel(&mut self);
}

/// The write half of a claimed bulk OUT endpoint. Each
/// [`submit`](Self::submit) queues one OUT transfer.
#[async_trait]
pub trait BulkWrite: Send {
    /// The endpoint's max packet size.
    fn max_packet(&self) -> usize;

    /// Queues one OUT transfer carrying `data`. Empty `data` is a zero-length
    /// packet. Returns at once.
    fn submit(&mut self, data: Vec<u8>);

    /// Transfers queued and not yet completed.
    fn pending(&self) -> usize;

    /// Waits for the oldest queued transfer to complete, or fails.
    async fn complete(&mut self) -> Result<(), UsbError>;
}

/// How long each kind of transfer may wait for the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub control: Duration,
    pub bulk: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            control: Duration::from_secs(5),
            bulk: Duration::from_secs(5),
        }
    }
}

/// What to claim when a device is opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    /// The interface to claim. `None` takes the first interface with a bulk
    /// endpoint. Some boot ROMs expose a storage interface first, so loaders
    /// that need the vendor one name it.
    pub interface: Option<u8>,
    pub timeouts: Timeouts,
}

/// A device on the bus, as far as it can be known without opening it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    /// Bus and port path, for example `1-2.3`. Stable while the device stays
    /// in the same port, including across a reset that re-enumerates it.
    pub location: String,
    pub vendor: u16,
    pub product: u16,
    pub serial: Option<String>,
    /// The device's USB specification version, `bcdUSB`, as the device
    /// reports it. Some boot ROMs flip bit 0 when a loader runs, so a caller
    /// can tell the two modes apart with it. A simulated device reports 0
    /// unless its summary says otherwise.
    pub bcd_usb: u16,
    /// The device release number, `bcdDevice`.
    pub bcd_device: u16,
}

impl DeviceSummary {
    /// A device at `location` with these ids, no serial, and zero versions.
    /// Set the rest with [`with_serial`](Self::with_serial) and
    /// [`with_versions`](Self::with_versions). Build with this, not a struct
    /// literal, so a new field does not break every caller.
    pub fn new(location: impl Into<String>, vendor: u16, product: u16) -> Self {
        Self {
            location: location.into(),
            vendor,
            product,
            serial: None,
            bcd_usb: 0,
            bcd_device: 0,
        }
    }

    /// Sets the serial number.
    pub fn with_serial(mut self, serial: impl Into<String>) -> Self {
        self.serial = Some(serial.into());
        self
    }

    /// Sets the serial number, or clears it when `serial` is `None`.
    pub fn with_serial_opt(mut self, serial: Option<String>) -> Self {
        self.serial = serial;
        self
    }

    /// Sets `bcdUSB` and `bcdDevice`.
    pub fn with_versions(mut self, bcd_usb: u16, bcd_device: u16) -> Self {
        self.bcd_usb = bcd_usb;
        self.bcd_device = bcd_device;
        self
    }
}

/// A vendor and product id pair a loader matches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
}

impl UsbId {
    pub fn matches(&self, device: &DeviceSummary) -> bool {
        self.vendor == device.vendor && self.product == device.product
    }
}

/// The direction of an endpoint, from bit 7 of its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointDirection {
    /// Device to host.
    In,
    /// Host to device.
    Out,
}

/// An endpoint's transfer type, from its descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointKind {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

/// One endpoint of an interface, as its descriptor describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointSummary {
    /// The endpoint address, with the direction in bit 7.
    pub address: u8,
    pub direction: EndpointDirection,
    pub kind: EndpointKind,
}

impl EndpointSummary {
    /// An endpoint at `address`. The direction is read from the address, so
    /// the two cannot disagree.
    pub fn new(address: u8, kind: EndpointKind) -> Self {
        let direction = if address & 0x80 != 0 {
            EndpointDirection::In
        } else {
            EndpointDirection::Out
        };
        Self {
            address,
            direction,
            kind,
        }
    }
}

/// One interface of a device's configuration: its class and the string the
/// device gives it, and its endpoints. The string is what a host searches
/// for, so a board names its vendor interface with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceSummary {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// The interface string, when the interface has one.
    pub string: Option<String>,
    pub endpoints: Vec<EndpointSummary>,
}

impl InterfaceSummary {
    /// An interface with these class codes, no string and no endpoints. Set
    /// the rest with [`with_string`](Self::with_string) and
    /// [`with_endpoint`](Self::with_endpoint).
    pub fn new(number: u8, class: u8, subclass: u8, protocol: u8) -> Self {
        Self {
            number,
            class,
            subclass,
            protocol,
            string: None,
            endpoints: Vec::new(),
        }
    }

    /// Sets the interface string.
    pub fn with_string(mut self, string: impl Into<String>) -> Self {
        self.string = Some(string.into());
        self
    }

    /// Adds an endpoint.
    pub fn with_endpoint(mut self, endpoint: EndpointSummary) -> Self {
        self.endpoints.push(endpoint);
        self
    }

    /// Whether the interface has a bulk OUT and a bulk IN endpoint.
    pub fn has_bulk_pair(&self) -> bool {
        let bulk = |direction| {
            self.endpoints
                .iter()
                .any(|ep| ep.kind == EndpointKind::Bulk && ep.direction == direction)
        };
        bulk(EndpointDirection::Out) && bulk(EndpointDirection::In)
    }
}
