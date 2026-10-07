//! Finding running PD devices over their USB gadget network directly, for
//! when mDNS doesn't get through (a host or image with multicast DNS off on
//! the link). Every USB device whose strings match a device package, such as
//! "Prometheus Dynamics" / "Raze", is probed at the address its USB serial
//! gives under the package's addressing scheme (`serial-hash-v1`), so
//! several boards on one computer each have a subnet of their own. The link
//! ranks above the network link, so a board seen both ways is kept as a USB
//! board.
//!
//! Images from before per-board addressing all sit at the package's fixed
//! address; it is probed too, but only while exactly one matching board is
//! plugged in, since the host can't route to two boards on one subnet.

use async_trait::async_trait;
use atlas_devices::DeviceManifest;
use atlas_driver::attributes::normalize_board_serial;
use atlas_driver::{Link, LinkId, LinkKind, LinkSource};

pub(crate) const LINK_ID: &str = "usb-gadget";

/// A plugged-in USB device that may be a PD gadget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsbGadget {
    pub(crate) manufacturer: Option<String>,
    pub(crate) product: Option<String>,
    /// iSerialNumber: the board serial on PD packages.
    pub(crate) serial: Option<String>,
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
            serial: info.serial_number().map(str::to_string),
        })
        .collect()
}

/// Where to look for one plugged-in board's identity endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GadgetProbe {
    /// The identity URL at the board's own gadget address, or at the fixed
    /// address when its serial gives none. This is the candidate's address.
    pub(crate) url: String,
    /// The fixed address, tried when `url` doesn't answer as this board (an
    /// image from before per-board addressing). Only with one board present.
    pub(crate) fallback: Option<String>,
    /// The board serial from the USB descriptor, normalized; the identity
    /// must report the same one.
    pub(crate) serial: Option<String>,
}

impl GadgetProbe {
    /// The URLs to try, in order.
    pub(crate) fn urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.url.as_str()).chain(self.fallback.as_deref())
    }
}

fn identity_url(address: &str) -> String {
    format!("http://{address}:5899{}", crate::IDENTITY_PATH)
}

/// One probe per plugged-in gadget of this model.
pub(crate) fn probes(manifest: &DeviceManifest, gadgets: &[UsbGadget]) -> Vec<GadgetProbe> {
    let mine: Vec<&UsbGadget> = gadgets
        .iter()
        .filter(|gadget| {
            manifest.matches_gadget(gadget.manufacturer.as_deref(), gadget.product.as_deref())
        })
        .collect();
    let addressing = manifest.gadget_addressing();
    let legacy = manifest
        .legacy_gadget_address()
        .filter(|_| mine.len() == 1)
        .map(|address| identity_url(&address));
    mine.into_iter()
        .filter_map(|gadget| {
            let serial = gadget.serial.as_deref().and_then(normalize_board_serial);
            let own = serial
                .as_deref()
                .zip(addressing.as_ref())
                .and_then(|(serial, addressing)| addressing.subnet_for(serial))
                .map(|subnet| identity_url(&subnet.address.to_string()));
            let (url, fallback) = match own {
                Some(url) => (url, legacy.clone()),
                None => (legacy.clone()?, None),
            };
            Some(GadgetProbe {
                url,
                fallback,
                serial,
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> DeviceManifest {
        serde_json::from_str(
            r#"{ "contract": 1, "model": "raze",
                 "identity": { "usb_gadget": { "manufacturer": "Prometheus Dynamics", "product": "Raze" } },
                 "capabilities": { "gadget-net": {
                     "address": "172.31.250.1/24",
                     "addressing": { "scheme": "serial-hash-v1", "base": "172.31.0.0/16",
                                     "prefix": 29, "exclude": ["172.31.250.0/24"] } } } }"#,
        )
        .unwrap()
    }

    fn raze(serial: Option<&str>) -> UsbGadget {
        UsbGadget {
            manufacturer: Some("Prometheus Dynamics".into()),
            product: Some("Raze".into()),
            serial: serial.map(str::to_string),
        }
    }

    const LEGACY: &str = "http://172.31.250.1:5899/.well-known/pd-device";

    #[test]
    fn one_board_is_probed_at_its_own_address_then_the_fixed_one() {
        let probes = probes(&manifest(), &[raze(Some("A317BCBEE5226D57"))]);
        assert_eq!(
            probes,
            vec![GadgetProbe {
                url: "http://172.31.209.217:5899/.well-known/pd-device".into(),
                fallback: Some(LEGACY.into()),
                serial: Some("e5226d57".into()),
            }]
        );
    }

    #[test]
    fn several_boards_each_get_their_own_address_and_no_fallback() {
        let other = UsbGadget {
            product: Some("Something else".into()),
            ..raze(Some("deadbeef"))
        };
        let probes = probes(
            &manifest(),
            &[
                raze(Some("a317bcbee5226d57")),
                raze(Some("10000000abcdef01")),
                raze(Some("not-hex")),
                other,
            ],
        );
        let urls: Vec<&str> = probes.iter().flat_map(GadgetProbe::urls).collect();
        assert_eq!(
            urls,
            vec![
                "http://172.31.209.217:5899/.well-known/pd-device",
                "http://172.31.0.113:5899/.well-known/pd-device",
            ]
        );
    }

    #[test]
    fn a_lone_board_without_a_hex_serial_is_probed_at_the_fixed_address() {
        for serial in [None, Some("my-board")] {
            let probes = probes(&manifest(), &[raze(serial)]);
            assert_eq!(
                probes,
                vec![GadgetProbe {
                    url: LEGACY.into(),
                    fallback: None,
                    serial: None,
                }]
            );
        }
    }

    #[test]
    fn a_package_without_addressing_keeps_the_single_board_rule() {
        let mut manifest = manifest();
        manifest.capabilities["gadget-net"]
            .as_object_mut()
            .unwrap()
            .remove("addressing");
        let one = probes(&manifest, &[raze(Some("a317bcbee5226d57"))]);
        assert_eq!(one[0].url, LEGACY);
        assert_eq!(one[0].serial.as_deref(), Some("e5226d57"));
        let two = probes(&manifest, &[raze(Some("a317bcbee5226d57")), raze(None)]);
        assert!(two.is_empty());
    }
}
