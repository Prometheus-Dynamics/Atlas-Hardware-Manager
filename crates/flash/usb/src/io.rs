use std::time::Duration;

use async_trait::async_trait;

use crate::UsbError;

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
