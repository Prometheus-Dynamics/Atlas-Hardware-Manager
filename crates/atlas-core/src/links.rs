//! Which of a device's links its record follows. A board on USB and on
//! Ethernet answers on both; its record keeps one data link and changes it
//! only for a reason that lasts, so its data and address don't flicker
//! between the two each scan:
//!
//! - the link it follows missed two scans in a row (unplugged, gone), or
//! - a better link (see [`link_rank`]) has answered two scans in a row.
//!
//! A device seen for the first time takes its best link of that scan. The
//! addresses a device answers on, over every link, are kept together in a
//! stable order (IPv4 numerically, then IPv6), so a list the user copies from
//! doesn't reorder.

use std::collections::BTreeMap;
use std::net::IpAddr;

use atlas_driver::LinkKind;

use crate::inventory::link_rank;

/// Scans in a row a followed link may miss before another takes over.
pub(crate) const MISSES_TO_LEAVE: u32 = 2;
/// Scans in a row a better link must answer before it takes over.
pub(crate) const STREAK_TO_PREFER: u32 = 2;
/// A link not seen for this many scans is forgotten.
const MISSES_TO_FORGET: u32 = 6;

/// One link a device answered on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkHealth {
    pub(crate) rank: u8,
    /// Scans in a row it answered.
    pub(crate) streak: u32,
    /// Scans in a row it didn't.
    pub(crate) misses: u32,
    answered_this_scan: bool,
    /// The IPs the device has on it.
    pub(crate) addresses: Vec<IpAddr>,
}

/// A device's links, by the address each was reached at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeviceLinks {
    links: BTreeMap<String, LinkHealth>,
}

impl DeviceLinks {
    /// A sighting on `address` this scan, with the IPs it reported.
    pub(crate) fn answered(&mut self, address: &str, kind: &LinkKind, addresses: Vec<IpAddr>) {
        let link = self.links.entry(address.to_string()).or_insert(LinkHealth {
            rank: link_rank(kind),
            streak: 0,
            misses: 0,
            answered_this_scan: false,
            addresses: Vec::new(),
        });
        link.rank = link_rank(kind);
        if !link.answered_this_scan {
            link.answered_this_scan = true;
            link.streak += 1;
        }
        link.misses = 0;
        link.addresses = addresses;
    }

    /// Whether a sighting on `address` becomes (or stays) the data link,
    /// when the record follows `current`.
    pub(crate) fn takes_over(&self, current: Option<&str>, address: &str) -> bool {
        let Some(current) = current else {
            return true;
        };
        if current == address {
            return true;
        }
        let Some(followed) = self.links.get(current) else {
            return true;
        };
        if followed.misses >= MISSES_TO_LEAVE {
            return true;
        }
        self.links.get(address).is_some_and(|candidate| {
            candidate.rank < followed.rank && candidate.streak >= STREAK_TO_PREFER
        })
    }

    /// The scan is over: links that didn't answer count a miss; long-gone
    /// ones are forgotten.
    pub(crate) fn end_scan(&mut self) {
        for link in self.links.values_mut() {
            if link.answered_this_scan {
                link.answered_this_scan = false;
            } else {
                link.streak = 0;
                link.misses += 1;
            }
        }
        self.links.retain(|_, link| link.misses < MISSES_TO_FORGET);
    }

    /// Every IP the device has on its recent links, IPv4 numerically first,
    /// then IPv6, without repeats.
    pub(crate) fn addresses(&self) -> Vec<IpAddr> {
        let mut all: Vec<IpAddr> = self
            .links
            .values()
            .filter(|link| link.misses < MISSES_TO_LEAVE)
            .flat_map(|link| link.addresses.iter().copied())
            .collect();
        all.sort_by_key(|ip| (ip.is_ipv6(), *ip));
        all.dedup();
        all
    }
}

/// The IP in a device address: a URL's host, or a bare IP.
pub(crate) fn host_ip(address: &str) -> Option<IpAddr> {
    if let Ok(ip) = address.trim_matches(['[', ']']).parse() {
        return Some(ip);
    }
    let url = url_host(address)?;
    url.trim_matches(['[', ']']).parse().ok()
}

/// The host part of `scheme://host[:port]/...`, without a URL parser.
fn url_host(address: &str) -> Option<&str> {
    let rest = address.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split_once(']').map(|(host, _)| host);
    }
    Some(authority.split(':').next().unwrap_or(authority))
}

#[cfg(test)]
mod tests {
    use super::*;

    const USB: &str = "http://172.31.250.1/.well-known/pd-device";
    const ETH: &str = "http://10.53.38.11/.well-known/pd-device";

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_followed_link_stays_while_it_answers() {
        let mut links = DeviceLinks::default();
        links.answered(ETH, &LinkKind::Ethernet, vec![ip("10.53.38.11")]);
        links.end_scan();
        // USB (better) shows up: it must answer twice before taking over.
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        links.answered(USB, &LinkKind::UsbNetwork, vec![]);
        assert!(!links.takes_over(Some(ETH), USB), "one scan isn't enough");
        links.end_scan();
        links.answered(USB, &LinkKind::UsbNetwork, vec![]);
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        assert!(
            links.takes_over(Some(ETH), USB),
            "two scans: the better link takes over"
        );
    }

    #[test]
    fn a_worse_link_takes_over_only_when_the_followed_one_is_gone() {
        let mut links = DeviceLinks::default();
        links.answered(USB, &LinkKind::UsbNetwork, vec![]);
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        links.end_scan();
        // One missed USB identify: Ethernet doesn't take the record.
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        assert!(!links.takes_over(Some(USB), ETH));
        links.end_scan();
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        assert!(!links.takes_over(Some(USB), ETH), "missed once so far");
        links.end_scan();
        links.answered(ETH, &LinkKind::Ethernet, vec![]);
        assert!(
            links.takes_over(Some(USB), ETH),
            "USB missed two scans: unplugged"
        );
    }

    #[test]
    fn addresses_keep_one_order_whatever_answers_first() {
        let mut a = DeviceLinks::default();
        a.answered(
            ETH,
            &LinkKind::Ethernet,
            vec![ip("fe80::1"), ip("10.53.38.11")],
        );
        a.answered(USB, &LinkKind::UsbNetwork, vec![ip("172.31.250.1")]);
        let mut b = DeviceLinks::default();
        b.answered(USB, &LinkKind::UsbNetwork, vec![ip("172.31.250.1")]);
        b.answered(
            ETH,
            &LinkKind::Ethernet,
            vec![ip("10.53.38.11"), ip("fe80::1"), ip("10.53.38.11")],
        );
        let expected = vec![ip("10.53.38.11"), ip("172.31.250.1"), ip("fe80::1")];
        assert_eq!(a.addresses(), expected);
        assert_eq!(b.addresses(), expected);
    }

    #[test]
    fn hosts_come_out_of_urls_and_bare_ips() {
        assert_eq!(host_ip(USB), Some(ip("172.31.250.1")));
        assert_eq!(host_ip("http://[fe80::1]:80/x"), Some(ip("fe80::1")));
        assert_eq!(host_ip("10.0.0.2"), Some(ip("10.0.0.2")));
        assert_eq!(host_ip("http://raze-abc.local/x"), None);
        assert_eq!(host_ip("port-1-2"), None);
    }
}
