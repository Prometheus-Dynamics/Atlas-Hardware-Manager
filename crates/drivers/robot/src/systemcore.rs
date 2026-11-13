//! The SystemCore (the 2027 controller), read without credentials, on a robot
//! network only.
//!
//! `GET :4800/api/team` answers `{"team":"6390","team_number":6390,…}` to
//! anyone. Its page is a dashboard at `/`, which this crate only links to.
//!
//! A SystemCore that runs Orion is not this crate's: it is an Orion node, and
//! the board driver owns it. So a unit that answers Orion's port is left out of
//! discovery, and an identify of one is settled as "the board driver owns it".
//! A unit that does not run Orion yet is a robot device here, and that is all.
//!
//! It is keyed by team (`systemcore:6390`), as the roboRIO is, so the same
//! unit seen on a robot network is one device. Its state is `RobotDevice`, and
//! it grants `OpenUi` only: nothing here reads its logs, its packages or its
//! versions, all of which need a login or Orion.
//!
//! The USB link (the gadget at `172.27.0.1`) is not probed here. It is a
//! different link kind, and a later PR says when it is read.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkId, LinkKind,
};
use serde::Deserialize;

use crate::gateway_kind;
use crate::known::{
    ATTR_MANAGE_URL, ATTR_TEAM, CONTROLLER_HOST, ORION_PORT, SYSTEMCORE_API_PORT,
    SYSTEMCORE_DNS_NAME, SYSTEMCORE_FAMILY, SYSTEMCORE_TEAM_PATH,
};
use crate::network::{self, RobotNet};
use crate::team::{address, team_of};

/// `GET :4800/api/team`, the fields this crate reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TeamAnswer {
    #[serde(default)]
    pub team_number: u16,
}

/// The SystemCore at `ip`, as its team API says it is.
async fn read(net: &RobotNet, ip: Ipv4Addr) -> Result<Identity, DriverError> {
    if net.open(ip, ORION_PORT).await {
        return Err(DriverError::Incompatible(format!(
            "{ip} runs Orion, so the board driver owns it"
        )));
    }
    let answer = network::expect_ok(
        ip,
        SYSTEMCORE_API_PORT,
        net.get(ip, SYSTEMCORE_API_PORT, SYSTEMCORE_TEAM_PATH).await,
    )?;
    let team = serde_json::from_str::<TeamAnswer>(&answer.body)
        .map_err(|error| {
            DriverError::Incompatible(format!(
                "{ip}:{SYSTEMCORE_API_PORT} answers, but not with a team: {error}"
            ))
        })?
        .team_number;
    if team == 0 {
        // Not set yet: the user can set it, so it is asked again later.
        return Err(DriverError::Other(format!(
            "the SystemCore at {ip} has no team yet"
        )));
    }
    Ok(Identity {
        key: DeviceKey::board(SYSTEMCORE_FAMILY, team.to_string())
            .map_err(|error| DriverError::Incompatible(error.reason))?,
        model: "SystemCore".into(),
        state: DeviceState::RobotDevice,
        lemnos: atlas_types::LemnosState::Absent,
        versions: Default::default(),
        name: None,
        link: LinkId::new(""),
        address: ip.to_string(),
        attributes: [
            (ATTR_TEAM.to_string(), team.to_string()),
            (ATTR_MANAGE_URL.to_string(), format!("http://{ip}/")),
        ]
        .into_iter()
        .collect(),
    })
}

/// The SystemCores to look for: the controller address of each known team and
/// the `robot.local` name when it resolves on a known team's network. Each must
/// answer its team API and must not answer Orion.
async fn discover(net: &RobotNet, link: &Link) -> Vec<Candidate> {
    let teams = net.teams();
    let mut probes: BTreeSet<Ipv4Addr> = teams
        .iter()
        .filter_map(|team| address(*team, CONTROLLER_HOST))
        .collect();
    let named = net.resolve(SYSTEMCORE_DNS_NAME).await;
    probes.extend(
        named
            .into_iter()
            .filter(|ip| team_of(*ip).is_some_and(|team| teams.contains(&team))),
    );
    let probes: Vec<Ipv4Addr> = probes.into_iter().collect();

    let api: Vec<(Ipv4Addr, u16)> = probes.iter().map(|ip| (*ip, SYSTEMCORE_API_PORT)).collect();
    let answering: Vec<Ipv4Addr> = probes
        .iter()
        .zip(net.open_all(&api).await)
        .filter(|(_, open)| *open)
        .map(|(ip, _)| *ip)
        .collect();
    let orion: Vec<(Ipv4Addr, u16)> = answering.iter().map(|ip| (*ip, ORION_PORT)).collect();
    answering
        .into_iter()
        .zip(net.open_all(&orion).await)
        .filter(|(_, orion)| !*orion)
        .map(|(ip, _)| network::candidate(&link.id, SYSTEMCORE_FAMILY, ip))
        .collect()
}

/// The driver for SystemCores on a robot network.
pub struct SystemcoreDriver {
    manifest: DriverManifest,
    net: Arc<RobotNet>,
}

impl SystemcoreDriver {
    pub fn new(net: Arc<RobotNet>) -> Self {
        Self {
            manifest: manifest(),
            net,
        }
    }
}

pub(crate) fn manifest() -> DriverManifest {
    DriverManifest {
        family: Family::new(SYSTEMCORE_FAMILY),
        name: "SystemCore (robot network)".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        link_kinds: vec![LinkKind::Ethernet, gateway_kind()],
        priority: 0,
    }
}

impl std::fmt::Debug for SystemcoreDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemcoreDriver")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Driver for SystemcoreDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if !network::handles(&self.manifest, link) {
            return Ok(Vec::new());
        }
        Ok(discover(&self.net, link).await)
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let ip = network::ipv4(&candidate.address)?;
        let mut identity = read(&self.net, ip).await?;
        identity.link = candidate.link.clone();
        Ok(identity)
    }

    fn capabilities(&self, identity: &Identity) -> CapabilitySet {
        let mut set = CapabilitySet::new();
        if identity.attributes.contains_key(ATTR_MANAGE_URL) {
            set.grant(CapabilityKind::OpenUi);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Fake;

    const TEAM: &str = include_str!("../tests/fixtures/systemcore-team.json");

    fn unit() -> Ipv4Addr {
        Ipv4Addr::new(10, 63, 90, 2)
    }

    #[tokio::test]
    async fn a_unit_without_orion_is_a_robot_device_named_by_its_team() {
        let fake = Fake::new().page(
            unit(),
            SYSTEMCORE_API_PORT,
            SYSTEMCORE_TEAM_PATH,
            200,
            None,
            TEAM,
        );
        let identity = read(
            &RobotNet::new(fake.reach(), Arc::new(BTreeSet::new())),
            unit(),
        )
        .await
        .unwrap();
        assert_eq!(identity.key.to_string(), "systemcore:6390");
        assert_eq!(identity.state, DeviceState::RobotDevice);
        assert_eq!(identity.attributes[ATTR_MANAGE_URL], "http://10.63.90.2/");
    }

    #[tokio::test]
    async fn a_unit_that_runs_orion_belongs_to_the_board_driver() {
        let fake = Fake::new().open(unit(), ORION_PORT).page(
            unit(),
            SYSTEMCORE_API_PORT,
            SYSTEMCORE_TEAM_PATH,
            200,
            None,
            TEAM,
        );
        let net = RobotNet::new(fake.reach(), Arc::new(BTreeSet::from([6390])));
        let error = read(&net, unit()).await.unwrap_err();
        assert!(
            matches!(error, DriverError::Incompatible(ref why) if why.contains("board driver")),
            "{error:?}"
        );
        let driver = SystemcoreDriver::new(net);
        let link = Link {
            id: LinkId::new("robot-net"),
            kind: LinkKind::Ethernet,
            label: "robot network".into(),
        };
        assert!(
            driver.discover(&link).await.unwrap().is_empty(),
            "Orion units are not listed"
        );
    }

    #[tokio::test]
    async fn no_team_yet_is_retried_and_garbage_is_settled() {
        let unset = Fake::new().page(
            unit(),
            SYSTEMCORE_API_PORT,
            SYSTEMCORE_TEAM_PATH,
            200,
            None,
            r#"{"team_number":0}"#,
        );
        let error = read(
            &RobotNet::new(unset.reach(), Arc::new(BTreeSet::new())),
            unit(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DriverError::Other(_)), "{error:?}");
        for body in ["not json", "<html></html>", ""] {
            let fake = Fake::new().page(
                unit(),
                SYSTEMCORE_API_PORT,
                SYSTEMCORE_TEAM_PATH,
                200,
                None,
                body,
            );
            let error = read(
                &RobotNet::new(fake.reach(), Arc::new(BTreeSet::new())),
                unit(),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(error, DriverError::Incompatible(_)),
                "{body:?}: {error:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_lists_a_unit_on_a_known_team_that_does_not_run_orion() {
        let fake = Fake::new().open(unit(), SYSTEMCORE_API_PORT);
        let driver = SystemcoreDriver::new(RobotNet::new(
            fake.reach(),
            Arc::new(BTreeSet::from([6390])),
        ));
        let link = Link {
            id: LinkId::new("robot-net"),
            kind: LinkKind::Ethernet,
            label: "robot network".into(),
        };
        let found = driver.discover(&link).await.unwrap();
        let addresses: Vec<&str> = found.iter().map(|c| c.address.as_str()).collect();
        assert_eq!(addresses, ["10.63.90.2"]);
    }
}
