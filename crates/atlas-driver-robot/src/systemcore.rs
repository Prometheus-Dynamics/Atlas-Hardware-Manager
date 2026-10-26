//! A WPILib SystemCore (FRC 2027 controller, Limelight OS), read-only:
//! found over its USB link or at `.2` on the robot network, and named by
//! the team its own API reports. No login.
//!
//! - USB: the gadget is `1d6b:0104` with manufacturer `Limelight`; the
//!   unit is `172.27.0.1` on its ECM link (Linux and macOS hosts) or
//!   `172.26.0.1` on RNDIS (Windows).
//! - `GET :4800/api/team` answers `{"team":"6390","team_number":6390,…}`.
//!
//! The key is the team (`systemcore:6390`), as for the roboRIO, so the same
//! unit seen over USB and on the robot network is one device.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use atlas_driver::{DeviceKey, DeviceMode, DriverError, Identity, LinkId};
use serde::Deserialize;

pub const SYSTEMCORE_FAMILY: &str = "systemcore";
/// The SystemCore's own API (team number).
pub const API_PORT: u16 = 4800;

/// The unit's address on its USB link, as this computer's OS reaches it.
pub fn usb_address() -> Ipv4Addr {
    if cfg!(windows) {
        Ipv4Addr::new(172, 26, 0, 1)
    } else {
        Ipv4Addr::new(172, 27, 0, 1)
    }
}

/// A SystemCore's USB gadget, by its descriptor strings.
pub fn is_systemcore_gadget(vendor: u16, product: u16, manufacturer: Option<&str>) -> bool {
    vendor == 0x1d6b
        && product == 0x0104
        && manufacturer.is_some_and(|m| m.trim().eq_ignore_ascii_case("limelight"))
}

/// The USB serials of the SystemCores plugged into this computer.
pub async fn plugged_in() -> Vec<String> {
    let Ok(devices) = nusb::list_devices().await else {
        return Vec::new();
    };
    devices
        .filter(|d| is_systemcore_gadget(d.vendor_id(), d.product_id(), d.manufacturer_string()))
        .map(|d| d.serial_number().unwrap_or_default().to_string())
        .collect()
}

/// Whether something answers on the API port, before an identify that
/// would fail (a roboRIO at `.2`).
pub async fn answers(ip: Ipv4Addr) -> bool {
    tokio::time::timeout(
        Duration::from_millis(400),
        tokio::net::TcpStream::connect(SocketAddr::from((ip, API_PORT))),
    )
    .await
    .is_ok_and(|connected| connected.is_ok())
}

#[derive(Debug, Deserialize)]
struct Team {
    team_number: Option<u16>,
}

/// The team from `:4800/api/team`'s answer.
pub fn parse_team(body: &str) -> Result<Option<u16>, String> {
    serde_json::from_str::<Team>(body)
        .map(|team| team.team_number.filter(|n| *n > 0))
        .map_err(|error| format!("its team answer isn't what a SystemCore sends: {error}"))
}

/// Reads the unit at `ip` and names it.
pub async fn read(
    http: &reqwest::Client,
    ip: Ipv4Addr,
    link: LinkId,
    usb_serial: Option<String>,
) -> Result<Identity, DriverError> {
    let url = format!("http://{ip}:{API_PORT}/api/team");
    let body = http
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?
        .text()
        .await
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?;
    let team = parse_team(&body).map_err(DriverError::Other)?;
    Ok(identity(team, ip, link, usb_serial))
}

fn identity(team: Option<u16>, ip: Ipv4Addr, link: LinkId, usb_serial: Option<String>) -> Identity {
    let mut attributes = std::collections::BTreeMap::new();
    attributes.insert("manage_url".to_string(), format!("http://{ip}/"));
    if let Some(team) = team {
        attributes.insert("team".to_string(), team.to_string());
    }
    if let Some(serial) = usb_serial.filter(|s| !s.is_empty()) {
        attributes.insert("usb_serial".to_string(), serial);
    }
    Identity {
        key: DeviceKey::new(
            SYSTEMCORE_FAMILY,
            team.map_or_else(|| "no-team".to_string(), |team| team.to_string()),
        ),
        model: "SystemCore".into(),
        mode: DeviceMode::Normal,
        versions: Default::default(),
        name: Some(team.map_or_else(
            || "SystemCore".to_string(),
            |team| format!("SystemCore-{team}"),
        )),
        link,
        address: ip.to_string(),
        attributes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_comes_from_its_api() {
        // A real answer (alpha18, 2026-10-26).
        let body = r#"{"team":"6390","team_number":6390,"timestamp":1760399233337}"#;
        assert_eq!(parse_team(body).unwrap(), Some(6390));
        assert_eq!(parse_team(r#"{"team_number":0}"#).unwrap(), None);
        assert!(parse_team("<html>").is_err());
        let id = identity(
            Some(6390),
            usb_address(),
            LinkId("usb-gadget".into()),
            Some("e9957f8c880f3c99".into()),
        );
        assert_eq!(id.key.to_string(), "systemcore:6390");
        assert_eq!(id.attributes["team"], "6390");
        assert_eq!(
            id.attributes["manage_url"],
            format!("http://{}/", usb_address())
        );
    }

    #[test]
    fn its_gadget_is_told_from_a_raze() {
        assert!(is_systemcore_gadget(0x1d6b, 0x0104, Some("Limelight")));
        assert!(!is_systemcore_gadget(
            0x1d6b,
            0x0104,
            Some("Prometheus Dynamics")
        ));
        assert!(!is_systemcore_gadget(0x1d6b, 0x0104, None));
    }
}

/// One of this computer's IPv4 addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAddress {
    pub interface: String,
    pub ip: Ipv4Addr,
    pub prefix: u8,
}

impl HostAddress {
    fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(self.prefix.min(32)))
        };
        u32::from(self.ip) & mask == u32::from(ip) & mask
    }
}

/// The host side of a SystemCore's USB link: its interface and MAC, as
/// the OS names them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbLink {
    pub interface: String,
    pub mac: Option<String>,
}

/// Whether this computer can reach a plugged-in SystemCore over USB, from
/// its interfaces: the link must have an address in the unit's subnet, and
/// no other interface may hold one that covers the unit's own address (a
/// Docker bridge from the 172.16.0.0/12 pool answers it instead).
pub fn usb_link_checks(
    unit: Ipv4Addr,
    links: &[UsbLink],
    addresses: &[HostAddress],
) -> Vec<atlas_driver::HealthCheck> {
    use atlas_driver::HealthCheck;
    let mut checks = Vec::new();
    let ours = |name: &str| links.iter().any(|l| l.interface == name);
    for other in addresses
        .iter()
        .filter(|a| !ours(&a.interface) && a.contains(unit))
    {
        checks.push(HealthCheck::error(
            "systemcore-address-taken",
            "SystemCore over USB",
            format!(
                "{} holds {}/{}, which covers the SystemCore's address {unit}: this computer answers it itself, so the SystemCore can't be reached",
                other.interface, other.ip, other.prefix
            ),
            format!(
                "Remove that network (for a Docker bridge: `docker network ls`, then `docker network rm` the one on {}), or give Docker an address pool that leaves {unit} out (`default-address-pools` in /etc/docker/daemon.json)",
                other.interface
            ),
        ));
    }
    for link in links {
        let addressed = addresses
            .iter()
            .any(|a| a.interface == link.interface && a.contains(unit));
        if addressed {
            continue;
        }
        let bind = link.mac.as_deref().map_or_else(String::new, |mac| {
            format!(" 802-3-ethernet.mac-address {mac}")
        });
        checks.push(HealthCheck::warning(
            "systemcore-usb-unconfigured",
            "SystemCore over USB",
            format!(
                "The SystemCore is plugged in, but its USB link ({}) has no address: the network manager left it unconfigured",
                link.interface
            ),
            format!(
                "Give it a DHCP profile that never becomes the default route, for example: nmcli connection add type ethernet con-name systemcore-usb ifname {}{bind} ipv4.method auto ipv4.never-default yes",
                link.interface
            ),
        ));
    }
    if checks.is_empty() && !links.is_empty() {
        checks.push(HealthCheck::ok(
            "systemcore-usb",
            "SystemCore over USB",
            format!("Its USB link has an address; the SystemCore is at {unit}."),
        ));
    }
    checks
}

/// The network interfaces of plugged-in SystemCores (Linux: the net
/// devices under a USB device with its descriptor). Elsewhere, none.
pub fn usb_links() -> Vec<UsbLink> {
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let device = std::fs::canonicalize(entry.path().join("device")).ok()?;
                let usb = device
                    .ancestors()
                    .find(|dir| dir.join("idVendor").is_file())?;
                let read = |file: &str| {
                    std::fs::read_to_string(usb.join(file))
                        .ok()
                        .map(|s| s.trim().to_string())
                };
                let vendor = u16::from_str_radix(&read("idVendor")?, 16).ok()?;
                let product = u16::from_str_radix(&read("idProduct")?, 16).ok()?;
                is_systemcore_gadget(vendor, product, read("manufacturer").as_deref()).then(|| {
                    UsbLink {
                        mac: std::fs::read_to_string(entry.path().join("address"))
                            .ok()
                            .map(|s| s.trim().to_string()),
                        interface: name,
                    }
                })
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// This computer's IPv4 addresses.
pub fn host_addresses() -> Vec<HostAddress> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) => Some(HostAddress {
                prefix: u32::from(v4.netmask).count_ones() as u8,
                ip: v4.ip,
                interface: iface.name,
            }),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod host_tests {
    use super::*;

    fn addr(interface: &str, ip: [u8; 4], prefix: u8) -> HostAddress {
        HostAddress {
            interface: interface.into(),
            ip: Ipv4Addr::from(ip),
            prefix,
        }
    }

    #[test]
    fn a_docker_bridge_on_the_units_address_and_an_unconfigured_link_are_told() {
        let unit = Ipv4Addr::new(172, 27, 0, 1);
        let link = UsbLink {
            interface: "enp17s0u3".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
        };
        // As found on a real host: fine, and a bridge on the Windows subnet doesn't matter.
        let fine = usb_link_checks(
            unit,
            std::slice::from_ref(&link),
            &[
                addr("enp17s0u3", [172, 27, 0, 25], 24),
                addr("br-d655", [172, 26, 0, 1], 16),
            ],
        );
        assert_eq!(fine.len(), 1);
        assert_eq!(fine[0].id, "systemcore-usb");
        let taken = usb_link_checks(
            unit,
            std::slice::from_ref(&link),
            &[
                addr("enp17s0u3", [172, 27, 0, 25], 24),
                addr("br-1234", [172, 27, 0, 1], 16),
            ],
        );
        assert_eq!(taken[0].id, "systemcore-address-taken");
        assert!(taken[0].detail.contains("br-1234"));
        let bare = usb_link_checks(unit, &[link], &[]);
        assert_eq!(bare[0].id, "systemcore-usb-unconfigured");
        assert!(
            bare[0]
                .fix
                .as_deref()
                .unwrap()
                .contains("ipv4.never-default yes")
        );
        assert!(
            usb_link_checks(unit, &[], &[]).is_empty(),
            "nothing plugged in, nothing said"
        );
    }
}
