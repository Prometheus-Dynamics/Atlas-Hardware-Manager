//! The board's own address on its USB network, scheme serial-hash-v1, and the [`Reach`] that finds a flasher there.
//!
//! The board and Atlas read ONE data file for the scheme, `devices/components/lemnos0-addressing/files/
//! lemnos0-addressing.conf`, built in here, so the PC side has no copy of its own and needs no configuration: the
//! board takes the address from its serial at boot (`lemnos0-address`) and listens there, and Atlas works out the
//! same address from the same serial. No network profile is made on the PC; its stock DHCP client takes a lease on
//! the rest of the board's /29. `scripts/check-lemnos0-address.py` runs the board's script on the same vectors this
//! module's tests use.

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_flash_flow::{Say, StepError, Target};
use atlas_session::{Handle, Sessions};
use orion::Address;
use sha2::{Digest, Sha256};

use crate::orion::Reach;

/// The scheme's data file, as the board image installs it.
pub const SCHEME_FILE: &str =
    include_str!("../../../../devices/components/lemnos0-addressing/files/lemnos0-addressing.conf");
/// The scheme this reads.
pub const SCHEME: &str = "serial-hash-v1";

/// The scheme's parameters, read from its data file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheme {
    base: u32,
    base_prefix: u8,
    prefix: u8,
    exclude: Option<(u32, u8)>,
    /// The port of the comm core's TCP way in on the board's address.
    pub peer_port: u16,
}

fn dotted(text: &str) -> Result<(u32, u8), String> {
    let (ip, prefix) = text
        .split_once('/')
        .ok_or_else(|| format!("{text:?} has no prefix"))?;
    let ip = Ipv4Addr::from_str(ip).map_err(|error| format!("{text:?}: {error}"))?;
    let prefix: u8 = prefix
        .parse()
        .map_err(|_| format!("{text:?}: the prefix is not a number"))?;
    if prefix > 32 {
        return Err(format!("{text:?}: the prefix is over 32"));
    }
    Ok((u32::from(ip), prefix))
}

impl Scheme {
    /// The scheme built into this build.
    ///
    /// # Errors
    ///
    /// When the built-in file does not read (the tests make sure it does).
    pub fn built_in() -> Result<Self, String> {
        Self::parse(SCHEME_FILE)
    }

    /// Reads a scheme file: `key=value` lines, `#` comments.
    ///
    /// # Errors
    ///
    /// When it names another scheme, or a value is missing or does not read.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value = |key: &str| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
        };
        match value("scheme") {
            Some(SCHEME) => {}
            other => {
                return Err(format!(
                    "the addressing file names scheme {other:?}, not {SCHEME}"
                ));
            }
        }
        let (base, base_prefix) = dotted(value("base").ok_or("the addressing file has no base")?)?;
        let prefix: u8 = value("prefix")
            .ok_or("the addressing file has no prefix")?
            .parse()
            .map_err(|_| "the addressing file's prefix is not a number")?;
        if base_prefix > prefix || prefix > 30 {
            return Err(format!(
                "the prefix {prefix} is outside the base /{base_prefix}"
            ));
        }
        let exclude = value("exclude")
            .filter(|text| !text.is_empty())
            .map(dotted)
            .transpose()?;
        if exclude.is_some_and(|(_, exclude_prefix)| {
            exclude_prefix < base_prefix || exclude_prefix > prefix
        }) {
            return Err("the exclude range is outside the base".to_owned());
        }
        let peer_port = value("peer_port")
            .ok_or("the addressing file has no peer_port")?
            .parse()
            .map_err(|_| "the addressing file's peer_port is not a port")?;
        Ok(Self {
            base,
            base_prefix,
            prefix,
            exclude,
            peer_port,
        })
    }

    /// The board's own address for the board serial `serial`, with its prefix.
    ///
    /// # Errors
    ///
    /// When the serial is not hex or has fewer than 8 digits: the board makes no address then either.
    pub fn address(&self, serial: &str) -> Result<(Ipv4Addr, u8), String> {
        let serial = serial.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if serial.is_empty() || !serial.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("the board serial {serial:?} is not hex"));
        }
        if serial.len() < 8 {
            return Err(format!(
                "the board serial {serial:?} has fewer than 8 hex digits"
            ));
        }
        let serial8 = serial[serial.len() - 8..].to_ascii_lowercase();
        let digest = Sha256::digest(serial8.as_bytes());
        let hash = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);

        let size = 1u64 << (32 - self.prefix);
        let slots = 1u64 << (self.prefix - self.base_prefix);
        let span = size * slots;
        let base = u64::from(self.base) - u64::from(self.base) % span;
        let (start, count) = match self.exclude {
            Some((ip, exclude_prefix)) => {
                let range = u64::from(ip) - u64::from(ip) % (1u64 << (32 - exclude_prefix));
                let offset = range
                    .checked_sub(base)
                    .filter(|offset| *offset < span)
                    .ok_or("the exclude range is outside the base")?;
                (Some(offset / size), 1u64 << (self.prefix - exclude_prefix))
            }
            None => (None, 0),
        };
        let usable = slots - count;
        if usable == 0 {
            return Err("no usable subnet is left in the base".to_owned());
        }
        let mut slot = u64::from(hash) % usable;
        if start.is_some_and(|start| start <= slot) {
            slot += count;
        }
        let address =
            u32::try_from(base + slot * size + 1).map_err(|_| "the address is past IPv4")?;
        Ok((Ipv4Addr::from(address), self.prefix))
    }

    /// The comm core's TCP way in on the board of serial `serial`.
    ///
    /// # Errors
    ///
    /// As [`Scheme::address`].
    pub fn peer(&self, serial: &str) -> Result<Address, String> {
        let (address, _) = self.address(serial)?;
        Address::from_str(&format!("orion+tcp://{address}:{}", self.peer_port))
    }
}

/// The serial of a device key, `family:serial`.
fn serial_of(target: &Target) -> &str {
    target
        .key
        .split_once(':')
        .map_or(target.key.as_str(), |(_, serial)| serial)
}

/// Reaches a flasher at its board's own address, worked out from the target's serial.
#[derive(Clone, Debug)]
pub struct SerialReach {
    scheme: Scheme,
    sessions: Arc<Sessions>,
}

impl SerialReach {
    /// The flasher's session through `sessions`, at the address `scheme` gives.
    #[must_use]
    pub const fn new(scheme: Scheme, sessions: Arc<Sessions>) -> Self {
        Self { scheme, sessions }
    }
}

#[async_trait]
impl Reach for SerialReach {
    async fn reach(&self, target: &Target, say: &Say<'_>) -> Result<Handle, StepError> {
        let peer = self
            .scheme
            .peer(serial_of(target))
            .map_err(StepError::Refused)?;
        say(format!(
            "looking for board {}'s flasher at {peer}",
            target.key
        ));
        self.sessions
            .handle(&peer)
            .ok_or_else(|| StepError::Refused(format!("the node at {peer} is turned off in Atlas")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `scripts/check-lemnos0-address.py`'s vectors: the board's script gives the same.
    const VECTORS: [(&str, &str); 7] = [
        ("a317bcbee5226d57", "172.31.209.217"),
        ("abcdef01", "172.31.0.113"),
        ("00000000", "172.31.43.201"),
        ("ffffffff", "172.31.141.25"),
        ("deadbeef", "172.31.209.1"),
        ("cfc0641d", "172.31.254.9"),
        ("0708c973b00a37bc", "172.31.12.81"),
    ];

    #[test]
    fn the_built_in_scheme_gives_the_board_s_addresses() {
        let scheme = Scheme::built_in().expect("the built-in file reads");
        for (serial, want) in VECTORS {
            assert_eq!(
                scheme.address(serial),
                Ok((Ipv4Addr::from_str(want).unwrap(), 29)),
                "{serial}"
            );
        }
        assert_eq!(
            scheme.address(" A317BCBEE5226D57\0\n"),
            Ok((Ipv4Addr::new(172, 31, 209, 217), 29)),
            "trimmed and lowercased as the board does"
        );
        assert_eq!(
            scheme.peer("a317bcbee5226d57").unwrap().to_string(),
            "orion+tcp://172.31.209.217:9200"
        );
    }

    #[test]
    fn a_serial_the_board_refuses_gives_no_address() {
        let scheme = Scheme::built_in().unwrap();
        for serial in ["", "zzzz1234", "abc123"] {
            assert!(scheme.address(serial).is_err(), "{serial:?}");
        }
    }

    #[test]
    fn a_file_of_another_scheme_is_refused() {
        assert!(Scheme::parse(&SCHEME_FILE.replace("serial-hash-v1", "serial-hash-v2")).is_err());
        assert!(Scheme::parse(&SCHEME_FILE.replace("peer_port=9200", "")).is_err());
    }

    #[test]
    fn the_serial_is_the_key_s() {
        let target = Target {
            key: "rpi:a317bcbee5226d57".into(),
            board: "reference".into(),
            family: "rpi".into(),
            port: "3-4".into(),
        };
        assert_eq!(serial_of(&target), "a317bcbee5226d57");
    }
}
