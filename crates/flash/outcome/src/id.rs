//! The USB vendor and product id pair. Std only: whether a device has an id is
//! asked in `atlas-usb` (its `UsbIdMatch` trait), where the device type lives.

use std::fmt;

/// A vendor and product id pair a loader matches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
}

/// `vendor:product` in lowercase hex, as `lsusb` writes it.
impl fmt::Display for UsbId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04x}:{:04x}", self.vendor, self.product)
    }
}
