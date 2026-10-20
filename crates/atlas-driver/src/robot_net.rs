//! The FRC robot network: the 10.TE.AM.0/24 convention, and the teams Atlas
//! knows, which tell drivers which robot networks to look on.
//!
//! An FRC robot's network is `10.TE.AM.0/24` for team TEAM (team 6390:
//! `10.63.90.0/24`): the robot controller is `.2`, the radio `.1` (robot
//! radio) or `.4` (a VH-109 as the team's access point), and the rest is
//! the team's (cameras, coprocessors, laptops). A computer reaches it
//! directly (on the radio's Wi-Fi or Ethernet) or through a board's USB
//! route (`usb_route`), which may be wider than one robot's network (a
//! board with a static `10.63.90.12/8` routes `10.0.0.0/8`). Which /24s of
//! a wide route are robot networks Atlas can't sweep to find out (65 536 of
//! them in a /8), so it narrows a route to the teams it knows
//! ([`RobotTeams`]): the ones the user set, and the ones devices have been
//! seen on.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};

/// The robot controller's family (a roboRIO; SystemCore later).
pub const CONTROLLER_FAMILY: &str = "roborio";
/// The robot radio's family (a Vivid-Hosting VH-109).
pub const RADIO_FAMILY: &str = "frc-radio";
/// The highest FRC team number the 10.TE.AM convention can hold (10.255.99).
pub const MAX_TEAM: u16 = 25_599;

/// An IPv4 network, `a.b.c.d/n`, normalized to its base address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Net {
    pub base: Ipv4Addr,
    pub prefix: u8,
}

impl Net {
    /// `a.b.c.d/n`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        let (ip, prefix) = text.trim().split_once('/')?;
        Self::of(ip.trim().parse().ok()?, prefix.trim().parse().ok()?)
    }

    /// The network `ip` is on with this prefix.
    pub fn of(ip: Ipv4Addr, prefix: u8) -> Option<Self> {
        (prefix <= 32).then(|| Self {
            base: Ipv4Addr::from(u32::from(ip) & mask(prefix)),
            prefix,
        })
    }

    pub fn contains(self, ip: Ipv4Addr) -> bool {
        u32::from(ip) & mask(self.prefix) == u32::from(self.base)
    }

    /// Whether all of `other` is inside this network.
    pub fn covers(self, other: Self) -> bool {
        other.prefix >= self.prefix && self.contains(other.base)
    }

    /// Its host addresses, if it is small enough to sweep (a /24 or less).
    pub fn hosts(self) -> Vec<Ipv4Addr> {
        if self.prefix < 24 || self.prefix > 30 {
            return Vec::new();
        }
        let base = u32::from(self.base);
        let size = 1u32 << (32 - self.prefix);
        (1..size - 1)
            .map(|offset| Ipv4Addr::from(base + offset))
            .collect()
    }

    /// The address `.host` on this network (`10.63.90.0/24`, 2: `10.63.90.2`).
    pub fn host(self, host: u8) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.base) | u32::from(host))
    }
}

impl std::fmt::Display for Net {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.base, self.prefix)
    }
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix).min(32))
    }
}

/// The team whose robot network `ip` is on, by the 10.TE.AM.x convention:
/// `10.63.90.2` is team 6390, `10.0.1.5` team 1, `10.1.0.5` team 100.
/// `None` off 10/8, for team 0, or past what the convention holds.
pub fn team_of(ip: Ipv4Addr) -> Option<u16> {
    let [a, te, am, _] = ip.octets();
    if a != 10 || am > 99 {
        return None;
    }
    let team = u16::from(te) * 100 + u16::from(am);
    (1..=MAX_TEAM).contains(&team).then_some(team)
}

/// Team `team`'s robot network, `10.TE.AM.0/24`.
pub fn team_net(team: u16) -> Option<Net> {
    if !(1..=MAX_TEAM).contains(&team) {
        return None;
    }
    let te = u8::try_from(team / 100).ok()?;
    let am = u8::try_from(team % 100).ok()?;
    Net::of(Ipv4Addr::new(10, te, am, 0), 24)
}

/// The robot networks inside a route to `routed`: itself when it is a /24
/// or narrower, else the known teams' /24s it covers and the /24 of each of
/// the board's own addresses on it (`own`), when those are robot networks.
pub fn robot_nets_in(routed: Net, own: &[Ipv4Addr], teams: &BTreeSet<u16>) -> Vec<Net> {
    if routed.prefix >= 24 {
        return vec![routed];
    }
    let mut nets: BTreeSet<Net> = teams
        .iter()
        .filter_map(|team| team_net(*team))
        .filter(|net| routed.covers(*net))
        .collect();
    nets.extend(
        own.iter()
            .filter(|ip| routed.contains(**ip) && team_of(**ip).is_some())
            .filter_map(|ip| Net::of(*ip, 24)),
    );
    nets.into_iter().collect()
}

/// The teams Atlas knows this run: the user's (settings, `--team`) and the
/// ones devices were seen on. One per process, shared by every driver.
#[derive(Default)]
pub struct RobotTeams {
    teams: Mutex<BTreeSet<u16>>,
}

impl RobotTeams {
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<RobotTeams> = OnceLock::new();
        SHARED.get_or_init(Self::default)
    }

    pub fn add(&self, team: u16) {
        if (1..=MAX_TEAM).contains(&team) {
            self.lock().insert(team);
        }
    }

    /// Notes the team of a device's address, if it is on a robot network.
    pub fn note_address(&self, ip: Ipv4Addr) {
        if let Some(team) = team_of(ip) {
            self.add(team);
        }
    }

    pub fn all(&self) -> BTreeSet<u16> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<u16>> {
        self.teams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    #[test]
    fn teams_follow_the_10_te_am_convention() {
        assert_eq!(team_of(ip("10.63.90.2")), Some(6390));
        assert_eq!(team_of(ip("10.0.1.5")), Some(1));
        assert_eq!(team_of(ip("10.1.0.5")), Some(100));
        assert_eq!(team_of(ip("10.255.99.1")), Some(25_599));
        assert_eq!(team_of(ip("10.172.166.85")), None, "AM past 99");
        assert_eq!(team_of(ip("10.0.0.1")), None, "team 0");
        assert_eq!(team_of(ip("192.168.1.2")), None);
        assert_eq!(team_net(6390), Net::parse("10.63.90.0/24"));
        assert_eq!(team_net(1), Net::parse("10.0.1.0/24"));
        assert_eq!(team_net(0), None);
        assert_eq!(team_net(30_000), None);
    }

    #[test]
    fn networks_parse_to_their_base_and_small_ones_list_their_hosts() {
        let net = Net::parse("10.63.90.12/24").unwrap();
        assert_eq!(net.to_string(), "10.63.90.0/24");
        assert_eq!(net.hosts().len(), 254);
        assert_eq!(net.host(2), ip("10.63.90.2"));
        assert!(Net::parse("10.0.0.0/8").unwrap().hosts().is_empty());
        assert!(Net::parse("10.0.0.0/8").unwrap().covers(net));
        assert!(!net.covers(Net::parse("10.0.0.0/8").unwrap()));
        assert!(Net::parse("10.0.0.1/33").is_none());
    }

    /// A board with a static 10.63.90.12/8 routes all of 10/8; Atlas looks
    /// only on the robot networks it can name inside it.
    #[test]
    fn a_wide_route_narrows_to_the_known_robot_networks() {
        let wide = Net::parse("10.0.0.0/8").unwrap();
        let teams = BTreeSet::from([6390, 5338]);
        let nets: Vec<String> = robot_nets_in(wide, &[], &teams)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(nets, ["10.53.38.0/24", "10.63.90.0/24"]);
        // The board's own address names its network without a team.
        let nets = robot_nets_in(wide, &[ip("10.63.90.12")], &BTreeSet::new());
        assert_eq!(nets, vec![Net::parse("10.63.90.0/24").unwrap()]);
        // Nothing known: nothing to look on.
        assert!(robot_nets_in(wide, &[], &BTreeSet::new()).is_empty());
        // A /24 route is the robot network itself.
        let narrow = Net::parse("10.63.90.0/24").unwrap();
        assert_eq!(robot_nets_in(narrow, &[], &BTreeSet::new()), vec![narrow]);
        // A team outside the route isn't looked for through it.
        let other = Net::parse("192.168.0.0/16").unwrap();
        assert!(robot_nets_in(other, &[], &teams).is_empty());
    }

    #[test]
    fn known_teams_ignore_addresses_off_the_robot_networks() {
        let teams = RobotTeams::default();
        teams.note_address(ip("10.63.90.10"));
        teams.note_address(ip("10.172.166.85"));
        teams.note_address(ip("172.31.167.9"));
        teams.add(0);
        assert_eq!(teams.all(), BTreeSet::from([6390]));
    }
}
