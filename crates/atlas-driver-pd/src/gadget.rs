//! Finding running PD devices over their USB gadget network directly, for
//! when mDNS doesn't get through (a host or image with multicast DNS off on
//! the link). A USB device whose strings match a device package, such as
//! "Prometheus Dynamics" / "Raze", is probed at the package's gadget
//! address. The link ranks above the network link, so a board seen both
//! ways is kept as a USB board.

use async_trait::async_trait;
use atlas_driver::{Link, LinkId, LinkKind, LinkSource};

pub(crate) const LINK_ID: &str = "usb-gadget";

/// A plugged-in USB device that may be a PD gadget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsbGadget {
    pub(crate) manufacturer: Option<String>,
    pub(crate) product: Option<String>,
}

/// USB devices with the Linux composite gadget id that PD packages use.
pub(crate) async fn gadgets() -> Vec<UsbGadget> {
    const LINUX_FOUNDATION: u16 = 0x1d6b;
    const COMPOSITE_GADGET: u16 = 0x0104;
    let Ok(devices) = nusb::list_devices().await else {
        return Vec::new();
    };
    devices
        .filter(|info| {
            info.vendor_id() == LINUX_FOUNDATION && info.product_id() == COMPOSITE_GADGET
        })
        .map(|info| UsbGadget {
            manufacturer: info.manufacturer_string().map(str::to_string),
            product: info.product_string().map(str::to_string),
        })
        .collect()
}

/// The USB gadget network of every PD device plugged into this computer.
/// USB hotplug (the USB boot link source watches it) triggers a scan.
pub struct UsbGadgetLinks;

#[async_trait]
impl LinkSource for UsbGadgetLinks {
    async fn links(&self) -> Vec<Link> {
        vec![Link {
            id: LinkId(LINK_ID.into()),
            kind: LinkKind::UsbNetwork,
            label: "USB gadget network".into(),
        }]
    }

    fn name(&self) -> &str {
        "USB network"
    }
}
