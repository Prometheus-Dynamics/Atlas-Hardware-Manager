//! The USB port path: the bus and the chain of hub ports a device sits at, for
//! example `3-4.2.1.4.1`. A flash job names its device by this path, never by
//! vendor and product id alone, so two identical boards in boot-ROM mode on
//! different ports are told apart, and a job never opens a device on a port it
//! was not given.
//!
//! The text form is what the Linux sysfs and the bus listing show: the bus
//! number, a dash, then the port numbers from the root hub down, separated by
//! dots. Port numbers start at 1. A path has one to seven ports, as the USB
//! tier limit allows.

use std::fmt;
use std::str::FromStr;

/// The most hub tiers below the root hub a device may sit at.
pub const MAX_TIERS: usize = 7;

/// A device's place on the bus: the bus number and the port chain.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortPath {
    bus: u16,
    ports: Vec<u8>,
}

/// Why a port path is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortPathError {
    /// No dash between the bus and the ports.
    NoBus(String),
    /// The bus is not a number from 1 to 65535.
    BadBus(String, String),
    /// No port after the bus.
    NoPorts(String),
    /// A port is not a number from 1 to 255.
    BadPort(String, String),
    /// More ports than the USB tier limit.
    TooDeep(String, usize),
}

impl fmt::Display for PortPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBus(text) => {
                write!(
                    f,
                    "{text:?} is not a USB port path: it has no dash between the bus and the ports"
                )
            }
            Self::BadBus(text, bus) => write!(
                f,
                "{text:?} is not a USB port path: the bus {bus:?} is not a number from 1 to 65535"
            ),
            Self::NoPorts(text) => {
                write!(
                    f,
                    "{text:?} is not a USB port path: it has no port after the bus"
                )
            }
            Self::BadPort(text, port) => write!(
                f,
                "{text:?} is not a USB port path: the port {port:?} is not a number from 1 to 255"
            ),
            Self::TooDeep(text, count) => write!(
                f,
                "{text:?} is not a USB port path: it has {count} ports, and at most {MAX_TIERS} are allowed"
            ),
        }
    }
}

impl std::error::Error for PortPathError {}

impl PortPath {
    /// A port path from its parts. `ports` must have one to seven entries, each
    /// from 1 to 255.
    pub fn new(bus: u16, ports: &[u8]) -> Result<Self, PortPathError> {
        let text = |ports: &[u8]| {
            let chain: Vec<String> = ports.iter().map(u8::to_string).collect();
            format!("{bus}-{}", chain.join("."))
        };
        if ports.is_empty() {
            return Err(PortPathError::NoPorts(text(ports)));
        }
        if ports.len() > MAX_TIERS {
            return Err(PortPathError::TooDeep(text(ports), ports.len()));
        }
        if bus == 0 {
            return Err(PortPathError::BadBus(text(ports), "0".into()));
        }
        if let Some(port) = ports.iter().find(|port| **port == 0) {
            return Err(PortPathError::BadPort(text(ports), port.to_string()));
        }
        Ok(Self {
            bus,
            ports: ports.to_vec(),
        })
    }

    /// The bus number.
    pub fn bus(&self) -> u16 {
        self.bus
    }

    /// The port numbers, from the root hub down.
    pub fn ports(&self) -> &[u8] {
        &self.ports
    }
}

impl FromStr for PortPath {
    type Err = PortPathError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (bus, chain) = text
            .split_once('-')
            .ok_or_else(|| PortPathError::NoBus(text.to_string()))?;
        let number = bus
            .parse::<u16>()
            .ok()
            .filter(|bus| *bus != 0)
            .ok_or_else(|| PortPathError::BadBus(text.to_string(), bus.to_string()))?;
        if chain.is_empty() {
            return Err(PortPathError::NoPorts(text.to_string()));
        }
        let mut ports = Vec::new();
        for part in chain.split('.') {
            let port = part
                .parse::<u8>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| PortPathError::BadPort(text.to_string(), part.to_string()))?;
            ports.push(port);
        }
        Self::new(number, &ports).map_err(|error| match error {
            PortPathError::TooDeep(_, count) => PortPathError::TooDeep(text.to_string(), count),
            other => other,
        })
    }
}

impl fmt::Display for PortPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-", self.bus)?;
        for (index, port) in self.ports.iter().enumerate() {
            if index > 0 {
                f.write_str(".")?;
            }
            write!(f, "{port}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_reads_and_writes_as_the_bus_listing_shows_it() {
        let path: PortPath = "3-4.2.1.4.1".parse().unwrap();
        assert_eq!(path.bus(), 3);
        assert_eq!(path.ports(), &[4, 2, 1, 4, 1]);
        assert_eq!(path.to_string(), "3-4.2.1.4.1");
        assert_eq!("1-2".parse::<PortPath>().unwrap().to_string(), "1-2");
    }

    #[test]
    fn malformed_paths_are_refused_with_the_part_that_is_wrong() {
        assert!(matches!(
            "32".parse::<PortPath>(),
            Err(PortPathError::NoBus(_))
        ));
        assert!(matches!(
            "x-1".parse::<PortPath>(),
            Err(PortPathError::BadBus(_, _))
        ));
        assert!(matches!(
            "0-1".parse::<PortPath>(),
            Err(PortPathError::BadBus(_, _))
        ));
        assert!(matches!(
            "3-".parse::<PortPath>(),
            Err(PortPathError::NoPorts(_))
        ));
        assert!(matches!(
            "3-4..1".parse::<PortPath>(),
            Err(PortPathError::BadPort(_, _))
        ));
        assert!(matches!(
            "3-0".parse::<PortPath>(),
            Err(PortPathError::BadPort(_, _))
        ));
        assert!(matches!(
            "3-256".parse::<PortPath>(),
            Err(PortPathError::BadPort(_, _))
        ));
        assert!(matches!(
            "3-1.2.3.4.5.6.7.8".parse::<PortPath>(),
            Err(PortPathError::TooDeep(_, 8))
        ));
    }

    #[test]
    fn the_constructor_checks_the_same_rules() {
        assert!(PortPath::new(3, &[]).is_err());
        assert!(PortPath::new(0, &[1]).is_err());
        assert!(PortPath::new(3, &[1, 0]).is_err());
        assert_eq!(PortPath::new(3, &[1, 2]).unwrap().to_string(), "3-1.2");
    }
}
