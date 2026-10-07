//! Which of this computer's addresses a board can reach.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

/// The local address the OS would send from to reach `peer`: the address
/// on the interface that routes there. Connecting a UDP socket sends
/// nothing; it only picks the route.
pub fn local_address_for(peer: IpAddr) -> Option<IpAddr> {
    let any: IpAddr = match peer {
        IpAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
        IpAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
    };
    let socket = UdpSocket::bind(SocketAddr::new(any, 0)).ok()?;
    socket.connect(SocketAddr::new(peer, 9)).ok()?;
    let local = socket.local_addr().ok()?.ip();
    (!local.is_unspecified()).then_some(local)
}

/// The addresses a server bound to `bind` answers on: the bound address,
/// or every interface address of its family when bound to all of them.
pub fn listening_addresses(bind: SocketAddr) -> Vec<IpAddr> {
    if !bind.ip().is_unspecified() {
        return vec![bind.ip()];
    }
    let mut addresses: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| !interface.is_loopback() && interface.is_oper_up())
        .map(|interface| interface.ip())
        .filter(|ip| serves(bind, *ip))
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

/// Whether a server bound to `bind` accepts connections to `ip`. An IPv4
/// wildcard takes IPv4 only; an IPv6 wildcard usually takes both.
pub fn serves(bind: SocketAddr, ip: IpAddr) -> bool {
    match bind.ip() {
        IpAddr::V4(any) if any.is_unspecified() => ip.is_ipv4(),
        IpAddr::V6(any) if any.is_unspecified() => true,
        bound => bound == ip,
    }
}

/// `ip` as the host part of a URL.
pub fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_routes_through_loopback() {
        assert_eq!(
            local_address_for(Ipv4Addr::LOCALHOST.into()),
            Some(Ipv4Addr::LOCALHOST.into())
        );
    }

    #[test]
    fn wildcards_and_hosts() {
        let any4: SocketAddr = "0.0.0.0:7700".parse().unwrap();
        assert!(serves(any4, "10.0.0.2".parse().unwrap()));
        assert!(!serves(any4, "fe80::1".parse().unwrap()));
        let one: SocketAddr = "10.0.0.2:7700".parse().unwrap();
        assert!(!serves(one, "10.0.0.3".parse().unwrap()));
        assert_eq!(listening_addresses(one), vec![one.ip()]);
        assert!(listening_addresses(any4).iter().all(IpAddr::is_ipv4));
        assert_eq!(url_host("fd00::2".parse().unwrap()), "[fd00::2]");
    }
}
