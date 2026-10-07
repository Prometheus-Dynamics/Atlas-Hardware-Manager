//! Per-board USB gadget addresses: `capabilities.gadget-net.addressing` in a
//! device manifest.
//!
//! Scheme `serial-hash-v1`, the same as `board_gadget_subnet` in the device
//! package's `lib.sh`:
//!
//! 1. Take the gadget's USB serial (iSerialNumber), normalized to Atlas's
//!    board-serial rule: its last 8 hex digits, lowercase.
//! 2. Hash those 8 ASCII characters with SHA-256 and read the first 4 bytes
//!    as a big-endian `u32`.
//! 3. Split `base` into `/prefix` subnets, drop the ones inside an `exclude`
//!    range, and take the remaining subnet at index `hash % count`, counting
//!    up from the lowest.
//! 4. The device has the subnet's first host address; the host on the other
//!    end of the cable gets a DHCP lease from the rest.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The scheme this Atlas computes.
pub const SERIAL_HASH_V1: &str = "serial-hash-v1";

/// How a model's boards pick their gadget network address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GadgetAddressing {
    pub scheme: String,
    /// The block subnets are taken from, for example `172.31.0.0/16`.
    pub base: String,
    /// Each board's subnet length, for example `29`.
    pub prefix: u8,
    /// Ranges inside `base` that are never handed out, as CIDRs.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// One board's gadget network: the device's address and the prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GadgetSubnet {
    pub address: Ipv4Addr,
    pub prefix: u8,
}

impl std::fmt::Display for GadgetSubnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

fn parse_cidr(text: &str) -> Option<(u32, u8)> {
    let (address, prefix) = text.trim().split_once('/')?;
    let address: Ipv4Addr = address.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    (prefix <= 32).then_some((u32::from(address), prefix))
}

impl GadgetAddressing {
    /// The subnet of the board whose normalized serial (8 lowercase hex
    /// digits, see `atlas_driver::attributes::normalize_board_serial`) is
    /// `serial8`. `None` for another scheme, invalid parameters, or a serial
    /// that isn't 8 hex digits.
    pub fn subnet_for(&self, serial8: &str) -> Option<GadgetSubnet> {
        if self.scheme != SERIAL_HASH_V1
            || serial8.len() != 8
            || !serial8.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let (base, base_prefix) = parse_cidr(&self.base)?;
        let prefix = self.prefix;
        if base_prefix > prefix || prefix > 30 {
            return None;
        }
        let size = 1u64 << (32 - prefix);
        let slots = 1u64 << (prefix - base_prefix);
        let base = u64::from(base) - u64::from(base) % (size * slots);

        // Excluded subnets as (first slot, count), ascending.
        let mut ranges = Vec::new();
        for cidr in &self.exclude {
            let (start, exclude_prefix) = parse_cidr(cidr)?;
            if exclude_prefix < base_prefix || exclude_prefix > prefix {
                return None;
            }
            let start = u64::from(start);
            let offset = (start - start % (1u64 << (32 - exclude_prefix))).checked_sub(base)?;
            if offset >= size * slots {
                return None;
            }
            ranges.push((offset / size, 1u64 << (prefix - exclude_prefix)));
        }
        ranges.sort_unstable();
        ranges.dedup();
        let usable = slots.checked_sub(ranges.iter().map(|(_, count)| count).sum())?;
        if usable == 0 {
            return None;
        }

        let serial8 = serial8.to_ascii_lowercase();
        let digest = Sha256::digest(serial8.as_bytes());
        let hash = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
        let mut slot = u64::from(hash) % usable;
        for (start, count) in ranges {
            if start <= slot {
                slot += count;
            }
        }
        let address = u32::try_from(base + slot * size + 1).ok()?;
        Some(GadgetSubnet {
            address: Ipv4Addr::from(address),
            prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raze() -> GadgetAddressing {
        GadgetAddressing {
            scheme: SERIAL_HASH_V1.into(),
            base: "172.31.0.0/16".into(),
            prefix: 29,
            exclude: vec!["172.31.250.0/24".into()],
        }
    }

    /// The same vectors as devices/raze/tests/gadget-address.sh, which runs
    /// them through lib.sh: the device and Atlas agree on every address.
    #[test]
    fn known_serials_map_to_known_subnets() {
        let addressing = raze();
        for (serial8, want) in [
            ("e5226d57", "172.31.209.217/29"), // a317bcbee5226d57
            ("abcdef01", "172.31.0.113/29"),   // 10000000ABCDEF01
            ("00000000", "172.31.43.201/29"),
            ("ffffffff", "172.31.141.25/29"),
            ("deadbeef", "172.31.209.1/29"),
            // Index 8097: past the excluded 172.31.250.0/24 (slots 8000..8032).
            ("cfc0641d", "172.31.254.9/29"),
        ] {
            let got = addressing.subnet_for(serial8).unwrap();
            assert_eq!(got.to_string(), want, "{serial8}");
        }
    }

    #[test]
    fn the_excluded_range_is_never_handed_out() {
        let addressing = raze();
        for i in 0u32..4096 {
            let serial8 = format!("{:08x}", i.wrapping_mul(2_654_435_761));
            let subnet = addressing.subnet_for(&serial8).unwrap();
            let octets = subnet.address.octets();
            assert_eq!(&octets[..2], &[172, 31]);
            assert_ne!(octets[2], 250, "{serial8} -> {subnet}");
            assert_eq!(octets[3] % 8, 1, "{serial8} -> {subnet}");
        }
    }

    #[test]
    fn bad_input_gives_no_subnet() {
        let addressing = raze();
        assert_eq!(addressing.subnet_for("e5226d5"), None);
        assert_eq!(addressing.subnet_for("e5226d5x"), None);
        assert_eq!(addressing.subnet_for("a317bcbee5226d57"), None);
        let other = GadgetAddressing {
            scheme: "serial-hash-v2".into(),
            ..raze()
        };
        assert_eq!(other.subnet_for("e5226d57"), None);
        let outside = GadgetAddressing {
            exclude: vec!["10.0.0.0/24".into()],
            ..raze()
        };
        assert_eq!(outside.subnet_for("e5226d57"), None);
    }
}
