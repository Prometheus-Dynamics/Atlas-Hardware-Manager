//! FRC team numbers and the robot networks they name.
//!
//! A team's robot network is `10.TE.AM.0/24`: team 6390 is `10.63.90.0/24`,
//! team 1 is `10.0.1.0/24`. Which teams Atlas looks for is not this crate's
//! to keep: the inventory knows them and passes them in through [`Teams`].

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

/// The highest team number the `10.TE.AM` convention holds (`10.255.99`).
pub const MAX_TEAM: u16 = 25_599;

/// The teams whose robot networks to look on. Read on every discovery, so a
/// team the user adds is found on the next scan.
pub trait Teams: Send + Sync {
    fn teams(&self) -> BTreeSet<u16>;
}

impl Teams for BTreeSet<u16> {
    fn teams(&self) -> BTreeSet<u16> {
        self.clone()
    }
}

/// The team whose robot network `ip` is on: `10.63.90.2` is team 6390,
/// `10.0.1.5` is team 1. `None` off `10/8`, for team 0, or past [`MAX_TEAM`].
pub fn team_of(ip: Ipv4Addr) -> Option<u16> {
    let [a, te, am, _] = ip.octets();
    if a != 10 || am > 99 {
        return None;
    }
    let team = u16::from(te) * 100 + u16::from(am);
    (1..=MAX_TEAM).contains(&team).then_some(team)
}

/// The address `.host` on team `team`'s robot network: `address(6390, 2)` is `10.63.90.2`.
pub fn address(team: u16, host: u8) -> Option<Ipv4Addr> {
    if !(1..=MAX_TEAM).contains(&team) {
        return None;
    }
    let te = u8::try_from(team / 100).ok()?;
    let am = u8::try_from(team % 100).ok()?;
    Some(Ipv4Addr::new(10, te, am, host))
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
    }

    #[test]
    fn addresses_are_team_hosts() {
        assert_eq!(address(6390, 2), Some(ip("10.63.90.2")));
        assert_eq!(address(1, 4), Some(ip("10.0.1.4")));
        assert_eq!(address(0, 2), None);
        assert_eq!(address(30_000, 2), None);
    }

    #[test]
    fn a_set_of_teams_is_a_teams_source() {
        let known = BTreeSet::from([6390u16]);
        assert_eq!(known.teams(), BTreeSet::from([6390]));
    }
}
