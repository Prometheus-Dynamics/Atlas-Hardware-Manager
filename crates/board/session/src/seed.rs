//! Where a node is: the link it sits on and its Orion address there.

use std::str::FromStr;

use atlas_types::{Link, LinkId, LinkKind};
use atlas_types::socket::{SocketError, check_socket_path};
use orion::Address;

/// A local socket's path must be one a Unix socket takes. A network or serial address has no socket
/// path, so it passes. Call this where an address comes in, before anything connects to it.
///
/// # Errors
///
/// [`SocketError::TooLong`] (naming the path) when the local socket path is too long.
pub fn check_address(address: &Address) -> Result<(), SocketError> {
    match address {
        Address::Local(path) => check_socket_path(path).map(|_| ()),
        Address::Network { .. } | Address::Serial { .. } => Ok(()),
    }
}

/// A place a node was told to be: the link it sits on and its Orion address there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub link: LinkId,
    pub address: Address,
}

impl Seed {
    /// Parses an Orion address (`orion+tcp://host:port`, `serial:path@baud`, ...).
    ///
    /// # Errors
    ///
    /// If `address` is not an Orion address the client understands.
    pub fn new(link: LinkId, address: &str) -> Result<Self, String> {
        let address = Address::from_str(address)?;
        check_address(&address).map_err(|error| error.to_string())?;
        Ok(Self { link, address })
    }

    /// Whether this node can be reached over `link`: a network address on a USB network or
    /// Ethernet link, a serial address on a USB serial link. A local socket is never a link.
    pub fn is_on(&self, link: &Link) -> bool {
        self.link == link.id
            && matches!(
                (&self.address, &link.kind),
                (
                    Address::Network { .. },
                    LinkKind::UsbNetwork | LinkKind::Ethernet
                ) | (Address::Serial { .. }, LinkKind::UsbSerial)
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: &str, kind: LinkKind) -> Link {
        Link {
            id: LinkId::new(id),
            kind,
            label: id.into(),
        }
    }

    #[test]
    fn a_network_seed_is_on_a_usb_network_or_ethernet_link_only() {
        let seed = Seed::new(LinkId::new("usbnet:a"), "orion+tcp://10.0.0.2:9200").unwrap();
        assert!(seed.is_on(&link("usbnet:a", LinkKind::UsbNetwork)));
        assert!(seed.is_on(&link("usbnet:a", LinkKind::Ethernet)));
        assert!(!seed.is_on(&link("usbnet:a", LinkKind::UsbSerial)));
        assert!(!seed.is_on(&link("usbnet:b", LinkKind::UsbNetwork)));
    }

    #[test]
    fn a_serial_seed_is_on_a_usb_serial_link_only() {
        let seed = Seed::new(LinkId::new("ser:0"), "serial:/dev/ttyACM0").unwrap();
        assert!(seed.is_on(&link("ser:0", LinkKind::UsbSerial)));
        assert!(!seed.is_on(&link("ser:0", LinkKind::Ethernet)));
    }

    #[test]
    fn an_address_that_is_not_orion_is_refused() {
        assert!(Seed::new(LinkId::new("x"), "ssh://10.0.0.2").is_err());
    }
}
