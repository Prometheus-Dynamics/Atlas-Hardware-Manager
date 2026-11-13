//! The robot's own devices: the roboRIO (robot controller), the robot radio
//! and the SystemCore, found on a robot network by team number and read
//! without credentials.
//!
//! This crate is read-only, and says so in every part. Its only requests are
//! HTTP GETs of the vendors' own status pages, TCP connects to see which ports
//! answer, and name lookups. It never writes, never logs in, never sends a
//! POST, never changes a setting and never touches the sshd, FTP, Driver
//! Station or console ports (see [`known`]). Reading a device on the robot
//! network this way is allowed because the device is someone else's to read
//! and nothing here changes it. It is not a board job: no board is configured
//! or updated through it, and nothing runs on a robot device.
//!
//! Three drivers, one per family, as the driver contract needs: [`RoborioDriver`],
//! [`RadioDriver`] and [`SystemcoreDriver`]. They share one [`RobotNet`], so
//! each address is probed once however many families look at it. Each device
//! is reported in the `RobotDevice` state and is granted only `Info`, `Status`,
//! `Telemetry` and `OpenUi`, as they apply. Its key is its team
//! (`roborio:6390`, `frc-radio:6390`, `systemcore:6390`): none of these report a
//! serial without a login.
//!
//! Discovery is by team number. For each team the inventory knows (passed in
//! through [`Teams`]), the drivers look at its robot network `10.TE.AM.0/24`:
//! `.2` for the controller, `.1` and `.4` for the radio, and the names the
//! controller and the SystemCore resolve to on that network. Storing teams and
//! robots is the inventory's job, not this crate's.
//!
//! Health: none. A robot that is off or away is the inventory's absence rule
//! (a power cycle is not a fault), so a driver never reports one as a host
//! problem. An answer that is not the device is settled (`Incompatible`); no
//! answer is retried with backoff (`Unreachable`).
//!
//! The gateway link kind is a board's USB route onto a robot network. The
//! [`Driver`] matches any gateway, so the device in the link's `via` is not
//! named here.

mod known;
mod network;
mod radio;
mod reach;
mod roborio;
mod systemcore;
mod team;

#[cfg(test)]
mod fake;

use std::sync::Arc;

use atlas_driver::Driver;
use atlas_types::{DeviceKey, LinkKind};

pub use known::{RADIO_FAMILY, ROBORIO_FAMILY, SYSTEMCORE_FAMILY};
pub use network::{CACHE_FOR, RobotNet};
pub use radio::{Band, RadioDriver, RadioStatus};
pub use reach::{Answer, CONNECT_LIMIT, HTTP_LIMIT, HttpReach, MAX_BODY, NAME_LIMIT, Reach};
pub use roborio::{PhoenixVersion, RoborioDriver};
pub use systemcore::{SystemcoreDriver, TeamAnswer};
pub use team::{MAX_TEAM, Teams, address, team_of};

/// The three drivers on one shared robot network, reaching devices through `reach`
/// and looking for the teams `teams` names.
pub fn drivers(reach: Arc<dyn Reach>, teams: Arc<dyn Teams>) -> Vec<Arc<dyn Driver>> {
    let net = RobotNet::new(reach, teams);
    vec![
        Arc::new(RoborioDriver::new(Arc::clone(&net))),
        Arc::new(RadioDriver::new(Arc::clone(&net))),
        Arc::new(SystemcoreDriver::new(net)),
    ]
}

/// The gateway link kind the drivers accept: any gateway matches, whichever
/// device relays it, so the placeholder `via` is never compared.
pub(crate) fn gateway_kind() -> LinkKind {
    LinkKind::Gateway {
        via: DeviceKey::from_static("relay", "placeholder"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use atlas_types::{CapabilityKind, DeviceState, Link, LinkId, LinkKind};

    use super::*;
    use crate::fake::Fake;

    const ROOT: &str = include_str!("../tests/fixtures/roborio-root.html");
    const VERSION: &str = include_str!("../tests/fixtures/phoenix-getversion.json");
    const STATUS: &str = include_str!("../tests/fixtures/vh109-status.json");
    const TEAM: &str = include_str!("../tests/fixtures/systemcore-team.json");

    fn link() -> Link {
        Link {
            id: LinkId::new("robot-net"),
            kind: LinkKind::Ethernet,
            label: "robot network".into(),
        }
    }

    /// Team 6390's network as the fixtures found it: a controller with robot
    /// code, a radio as access point, and a SystemCore that does not run Orion.
    fn team_6390() -> Fake {
        let rio = Ipv4Addr::new(10, 63, 90, 2);
        let radio = Ipv4Addr::new(10, 63, 90, 4);
        Fake::new()
            .page(rio, 80, "/", 200, Some("Embedthis-http"), ROOT)
            .page(rio, 1250, "/?action=getversion", 200, None, VERSION)
            .open(rio, 5810)
            .open(rio, 80)
            .page(radio, 80, "/status", 200, None, STATUS)
            .open(radio, 80)
            .page(rio, 4800, "/api/team", 200, None, TEAM)
            .open(rio, 4800)
    }

    /// Discovers and identifies everything on the fake, the way the scan does.
    async fn scan(fake: &Fake, teams: &[u16]) -> Vec<String> {
        let drivers = drivers(
            fake.reach(),
            Arc::new(teams.iter().copied().collect::<BTreeSet<u16>>()),
        );
        let mut keys = Vec::new();
        for driver in &drivers {
            for candidate in driver.discover(&link()).await.unwrap() {
                if let Ok(identity) = driver.identify(&candidate).await {
                    keys.push(identity.key.to_string());
                }
            }
        }
        keys.sort();
        keys
    }

    /// The read-only proof: every request a whole scan makes is a GET of one
    /// of the known paths, a connect or a GET on one of the known ports, or a
    /// lookup of a known name, and none goes to a port the table never probes.
    #[tokio::test(start_paused = true)]
    async fn a_whole_scan_asks_only_the_known_ports_and_paths() {
        let fake = team_6390();
        assert_eq!(
            scan(&fake, &[6390]).await,
            ["frc-radio:6390", "roborio:6390", "systemcore:6390"]
        );
        let known_ports: [u16; 6] = [80, 1250, 4800, 5810, 1735, 9200];
        let known_paths = ["/", "/?action=getversion", "/status", "/api/team"];
        let requests = fake.requests();
        assert!(!requests.is_empty());
        for request in &requests {
            let words: Vec<&str> = request.split(' ').collect();
            match words.as_slice() {
                ["GET", address, path] => {
                    let port: u16 = address.rsplit(':').next().unwrap().parse().unwrap();
                    assert!(known_ports.contains(&port), "{request}");
                    assert!(known_paths.contains(path), "{request}");
                }
                ["CONNECT", address] => {
                    let port: u16 = address.rsplit(':').next().unwrap().parse().unwrap();
                    assert!(known_ports.contains(&port), "{request}");
                }
                ["RESOLVE", name] => assert!(
                    name.ends_with(".lan")
                        || name.ends_with(".local")
                        || *name == "robot.local"
                        || name.starts_with("roboRIO-"),
                    "{request}"
                ),
                _ => panic!("unexpected request {request}"),
            }
            for never in crate::known::ROBORIO_NEVER_PROBED {
                assert!(!request.ends_with(&format!(":{never}")), "{request}");
            }
        }
    }

    /// An unreachable network: every probe times out, discovery finds nothing,
    /// and it takes about one limit, not one per address.
    #[tokio::test(start_paused = true)]
    async fn an_unreachable_network_finds_nothing_and_nothing_blocks() {
        let fake = Fake::new().hang();
        let started = tokio::time::Instant::now();
        let drivers = drivers(
            fake.reach(),
            Arc::new(BTreeSet::from([6390u16, 1678, 254, 1])),
        );
        for driver in &drivers {
            assert!(driver.discover(&link()).await.unwrap().is_empty());
        }
        let took = started.elapsed();
        assert!(took <= Duration::from_secs(5), "took {took:?}");
        let identity_error = drivers[0]
            .identify(&crate::network::candidate(
                &LinkId::new("robot-net"),
                ROBORIO_FAMILY,
                Ipv4Addr::new(10, 63, 90, 2),
            ))
            .await
            .unwrap_err();
        assert!(
            matches!(identity_error, atlas_driver::DriverError::Unreachable(_)),
            "{identity_error:?}"
        );
    }

    /// One address that never answers blocks only itself: the other team's
    /// controller is still found, and within one probe limit.
    #[tokio::test(start_paused = true)]
    async fn one_silent_address_does_not_hold_up_another() {
        let fake = Fake::new()
            .hang_for(Ipv4Addr::new(10, 63, 90, 2))
            .page(
                Ipv4Addr::new(10, 16, 78, 2),
                80,
                "/",
                200,
                Some("Embedthis-http"),
                ROOT,
            )
            .open(Ipv4Addr::new(10, 16, 78, 2), 80);
        let drivers = drivers(fake.reach(), Arc::new(BTreeSet::from([6390u16, 1678])));
        let started = tokio::time::Instant::now();
        let found = drivers[0].discover(&link()).await.unwrap();
        assert_eq!(
            found.iter().map(|c| c.address.as_str()).collect::<Vec<_>>(),
            ["10.16.78.2"]
        );
        assert!(
            started.elapsed() <= CONNECT_LIMIT * 2,
            "took {:?}",
            started.elapsed()
        );
    }

    /// A network that answers with garbage is settled or retried, never a panic.
    #[tokio::test(start_paused = true)]
    async fn a_device_that_answers_with_garbage_never_panics_a_scan() {
        let rio = Ipv4Addr::new(10, 63, 90, 2);
        let fake = Fake::new()
            .open(rio, 80)
            .open(rio, 4800)
            .page(rio, 80, "/", 200, Some("Embedthis-http"), "\u{1}\u{2}\u{3}")
            .page(rio, 4800, "/api/team", 200, None, "\u{0}\u{ff}{{{");
        assert!(scan(&fake, &[6390]).await.is_empty());
    }

    /// Every family's capabilities are a subset of the read-only kinds, in
    /// every state a robot device can be in.
    #[tokio::test]
    async fn no_robot_device_is_granted_anything_that_writes() {
        let drivers = drivers(Fake::new().reach(), Arc::new(BTreeSet::<u16>::new()));
        assert_eq!(drivers.len(), 3);
        let families: BTreeSet<&str> = drivers
            .iter()
            .map(|d| d.manifest().family.as_str())
            .collect();
        assert_eq!(
            families,
            BTreeSet::from([ROBORIO_FAMILY, RADIO_FAMILY, SYSTEMCORE_FAMILY])
        );
        let read_only = [
            CapabilityKind::Info,
            CapabilityKind::Status,
            CapabilityKind::Telemetry,
            CapabilityKind::OpenUi,
        ];
        for driver in &drivers {
            let all = CapabilityKind::ALL;
            let identity = atlas_types::Identity {
                key: DeviceKey::from_static("x", "1"),
                model: None,
                state: DeviceState::RobotDevice,
                lemnos: atlas_types::LemnosState::Absent,
                versions: Default::default(),
                name: None,
                link: LinkId::new("robot-net"),
                address: "10.63.90.2".into(),
                attributes: [(
                    known::ATTR_MANAGE_URL.to_string(),
                    "http://10.63.90.2/".to_string(),
                )]
                .into_iter()
                .collect(),
            };
            let granted = driver.capabilities(&identity);
            for kind in all {
                if granted.grants(kind) {
                    assert!(
                        read_only.contains(&kind),
                        "{:?} grants {kind:?}",
                        driver.manifest().family
                    );
                }
            }
            for shown in granted.shown(DeviceState::RobotDevice) {
                assert!(read_only.contains(&shown), "{shown:?} shown");
            }
        }
    }

    #[test]
    fn the_drivers_handle_the_robot_network_and_a_gateway_to_it_only() {
        let drivers = drivers(Fake::new().reach(), Arc::new(BTreeSet::<u16>::new()));
        let usb = LinkKind::UsbNetwork;
        for driver in &drivers {
            let manifest = driver.manifest();
            assert!(manifest.handles(&LinkKind::Ethernet));
            assert!(manifest.handles(&LinkKind::Gateway {
                via: DeviceKey::from_static("any", "x")
            }));
            assert!(!manifest.handles(&usb));
            assert!(!manifest.handles(&LinkKind::UsbSerial));
        }
    }

    #[tokio::test]
    async fn a_driver_ignores_a_link_kind_it_does_not_handle() {
        let drivers = drivers(Fake::new().reach(), Arc::new(BTreeSet::from([6390u16])));
        let link = Link {
            id: LinkId::new("usb0"),
            kind: LinkKind::UsbNetwork,
            label: "USB".into(),
        };
        for driver in &drivers {
            assert!(driver.discover(&link).await.unwrap().is_empty());
        }
    }
}
