//! Which robot networks to look on, and what answers at the robot's fixed
//! addresses: `.2` (the robot controller) and `.1` or `.4` (the radio).
//!
//! A probe is a TCP connect to port 80 on those three addresses of each
//! robot network: cheap, read-only, and enough to tell a host is there
//! before a driver asks it who it is. Probes are cached: the first look at
//! a network waits for its answer (at most [`TIMEOUT`], all addresses at
//! once), later ones hand out the last answer and refresh it in the
//! background at most every [`EVERY`].

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use atlas_driver::robot_net::{CONTROLLER_FAMILY, Net, RADIO_FAMILY};
use atlas_driver::{Candidate, Family, LinkId};

/// How often a network's probe is refreshed.
const EVERY: Duration = Duration::from_secs(20);
/// How long one address may take to accept a connection.
pub(crate) const TIMEOUT: Duration = Duration::from_millis(700);
/// The port every robot-network device here serves its web page on.
const PORT: u16 = 80;

/// The robot controller's host number on a robot network.
pub const CONTROLLER_HOST: u8 = 2;
/// The radio's host numbers: `.1` as the robot radio, `.4` as the team's
/// access point.
pub const RADIO_HOSTS: [u8; 2] = [1, 4];

/// What a robot network has at its fixed addresses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RobotHosts {
    pub controller: Option<Ipv4Addr>,
    pub radios: Vec<Ipv4Addr>,
}

impl RobotHosts {
    /// The candidates these hosts are, on `link`.
    pub fn candidates(&self, link: &LinkId) -> Vec<Candidate> {
        let controller = self.controller.map(|ip| Candidate {
            link: link.clone(),
            family: Family::new(CONTROLLER_FAMILY),
            address: ip.to_string(),
        });
        let radios = self.radios.iter().map(|ip| Candidate {
            link: link.clone(),
            family: Family::new(RADIO_FAMILY),
            address: ip.to_string(),
        });
        controller.into_iter().chain(radios).collect()
    }
}

#[derive(Clone, Default)]
struct Probed {
    hosts: RobotHosts,
    at: Option<Instant>,
    running: bool,
}

/// The cached probes, one per robot network. One per process.
#[derive(Default)]
pub struct RobotProbe {
    nets: Mutex<BTreeMap<Net, Probed>>,
}

impl RobotProbe {
    pub fn shared() -> &'static Arc<Self> {
        static SHARED: OnceLock<Arc<RobotProbe>> = OnceLock::new();
        SHARED.get_or_init(Arc::default)
    }

    /// What answers on each of `nets`, as candidates on `link`.
    pub async fn candidates(self: &Arc<Self>, nets: &[Net], link: &LinkId) -> Vec<Candidate> {
        let mut out = Vec::new();
        for net in nets.iter().collect::<BTreeSet<_>>() {
            out.extend(self.hosts(*net).await.candidates(link));
        }
        out
    }

    /// What answers on `net`: waits the first time, then refreshes in the
    /// background when due.
    pub async fn hosts(self: &Arc<Self>, net: Net) -> RobotHosts {
        let (known, due) = {
            let mut nets = self.lock();
            let probed = nets.entry(net).or_default();
            let due = !probed.running && probed.at.is_none_or(|at| at.elapsed() >= EVERY);
            if due {
                probed.running = true;
            }
            (probed.at.map(|_| probed.hosts.clone()), due)
        };
        match (known, due) {
            (Some(hosts), false) => hosts,
            (Some(hosts), true) => {
                let probe = Arc::clone(self);
                tokio::spawn(async move { probe.refresh(net).await });
                hosts
            }
            (None, true) => self.refresh(net).await,
            // Another caller's first probe is running; don't wait for it.
            (None, false) => RobotHosts::default(),
        }
    }

    async fn refresh(&self, net: Net) -> RobotHosts {
        let hosts = probe(net, PORT).await;
        let mut nets = self.lock();
        let probed = nets.entry(net).or_default();
        probed.hosts = hosts.clone();
        probed.at = Some(Instant::now());
        probed.running = false;
        hosts
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Net, Probed>> {
        self.nets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Connects to `port` on the robot's fixed addresses of `net`, all at once.
async fn probe(net: Net, port: u16) -> RobotHosts {
    let open = |host: u8| async move {
        let ip = net.host(host);
        let address = SocketAddr::from((ip, port));
        let connect = tokio::net::TcpStream::connect(address);
        matches!(tokio::time::timeout(TIMEOUT, connect).await, Ok(Ok(_))).then_some(ip)
    };
    let (controller, radio_1, radio_4) = tokio::join!(
        open(CONTROLLER_HOST),
        open(RADIO_HOSTS[0]),
        open(RADIO_HOSTS[1])
    );
    RobotHosts {
        controller,
        radios: [radio_1, radio_4].into_iter().flatten().collect(),
    }
}

/// This computer's robot networks: its IPv4 addresses on a 10.TE.AM.x
/// network with a /24 or narrower mask.
pub fn own_robot_nets() -> Vec<Net> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4),
            if_addrs::IfAddr::V6(_) => None,
        })
        .filter(|v4| atlas_driver::robot_net::team_of(v4.ip).is_some())
        .filter_map(|v4| {
            let prefix = u8::try_from(u32::from(v4.netmask).count_ones()).ok()?;
            (prefix >= 24).then(|| Net::of(v4.ip, 24)).flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_become_candidates_of_their_family() {
        let hosts = RobotHosts {
            controller: Some(Ipv4Addr::new(10, 63, 90, 2)),
            radios: vec![Ipv4Addr::new(10, 63, 90, 4)],
        };
        let found = hosts.candidates(&LinkId("robot-net".into()));
        let found: Vec<(&str, &str)> = found
            .iter()
            .map(|c| (c.family.as_str(), c.address.as_str()))
            .collect();
        assert_eq!(
            found,
            [("roborio", "10.63.90.2"), ("frc-radio", "10.63.90.4")]
        );
    }

    /// The probe finds what listens on the loopback "robot network"
    /// (127.0.0.0/24: .2 listens, .1 and .4 don't).
    #[tokio::test]
    async fn a_probe_finds_what_accepts_a_connection() {
        let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.2:0").await else {
            return; // No 127.0.0.2 on this system (macOS without an alias).
        };
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { while listener.accept().await.is_ok() {} });
        let hosts = probe(Net::parse("127.0.0.0/24").unwrap(), port).await;
        assert_eq!(hosts.controller, Some(Ipv4Addr::new(127, 0, 0, 2)));
        assert!(hosts.radios.is_empty());
    }
}
