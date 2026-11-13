//! The robot controller (a roboRIO running NI's FRC image), read without
//! credentials.
//!
//! What it serves to anyone on the robot network:
//! - port 80, `/`: NI's web configuration page (`Server: Embedthis-http`, title
//!   `HTML Device Configuration`). Its system facts come from a POST that the
//!   page sends on load. This crate sends no POST, so it reports what a GET
//!   gives: the hostname from the team, the page, and what answers.
//! - port 1250, `GET /?action=getversion`: CTRE Phoenix's version, inside robot
//!   code that uses Phoenix.
//! - ports 5810 (NetworkTables 4) and 1735 (NetworkTables 3): open while robot
//!   code runs. Read by connect only.
//!
//! It is keyed by team (`roborio:6390`): one robot controller per robot
//! network. Its state is `RobotDevice`: it is read, never written. Its
//! capabilities are `Status` (whether robot code runs) and `OpenUi` (its page).
//! Nothing else is granted; in particular no logs, which need a login or the
//! console stream, and no CAN device list.

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
    ATTR_HOSTNAME, ATTR_MANAGE_URL, ATTR_ROBOT_CODE, ATTR_SERVICES, ATTR_TEAM, CONTROLLER_HOST,
    NI_SERVER_PREFIX, NI_TITLE, PHOENIX_PORT, PHOENIX_VERSION_PATH, ROBORIO_DNS_SUFFIXES,
    ROBORIO_FAMILY, ROBORIO_HOSTNAME, ROBORIO_SERVICES, WEB_PATH, WEB_PORT,
};
use crate::network::{self, RobotNet};
use crate::team::{address, team_of};

/// `GET :1250/?action=getversion`. Fields missing from the answer are empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct PhoenixVersion {
    #[serde(rename = "System", default)]
    pub system: String,
    /// The FRC season the code was built for, `FRC2026`.
    #[serde(rename = "ReleaseInfo", default)]
    pub release: String,
    /// `2026.3.0 (May 22 2026, 21:26:11)`.
    #[serde(rename = "Version", default)]
    pub version: String,
}

impl PhoenixVersion {
    /// The version without its build date: `2026.3.0`.
    pub fn short_version(&self) -> &str {
        self.version
            .split_whitespace()
            .next()
            .unwrap_or(&self.version)
    }

    /// The season's year, `2026` from `FRC2026`.
    pub fn season(&self) -> Option<&str> {
        self.release
            .strip_prefix("FRC")
            .filter(|year| !year.is_empty())
    }
}

/// Whether an answer is NI's robot controller page.
fn is_ni_page(server: Option<&str>, body: &str) -> bool {
    server.is_some_and(|server| server.starts_with(NI_SERVER_PREFIX)) && body.contains(NI_TITLE)
}

/// The robot controller at `ip`, as its page and Phoenix say it is.
async fn read(net: &RobotNet, ip: Ipv4Addr, team: u16) -> Result<Identity, DriverError> {
    let page = network::expect_ok(ip, WEB_PORT, net.get(ip, WEB_PORT, WEB_PATH).await)?;
    if !is_ni_page(page.server.as_deref(), &page.body) {
        return Err(DriverError::Incompatible(format!(
            "{ip} answers on port {WEB_PORT}, but not as NI's robot controller page"
        )));
    }
    let phoenix = read_phoenix(net, ip).await;
    let mut services = Vec::new();
    if phoenix.is_some() {
        services.push("phoenix");
    }
    let probes: Vec<(Ipv4Addr, u16)> = ROBORIO_SERVICES
        .iter()
        .map(|(_, port)| (ip, *port))
        .collect();
    for ((name, _), open) in ROBORIO_SERVICES.iter().zip(net.open_all(&probes).await) {
        if open {
            services.push(*name);
        }
    }
    identity(ip, team, phoenix.as_ref(), &services)
}

/// Phoenix's version, when robot code runs its diagnostics server and the answer
/// is one. A garbled answer is no version, not an error.
async fn read_phoenix(net: &RobotNet, ip: Ipv4Addr) -> Option<PhoenixVersion> {
    let answer = net.get(ip, PHOENIX_PORT, PHOENIX_VERSION_PATH).await?;
    if !(200..300).contains(&answer.status) {
        return None;
    }
    serde_json::from_str::<PhoenixVersion>(&answer.body)
        .ok()
        .filter(|version| !version.version.is_empty())
}

/// The identity of a robot controller read this way.
fn identity(
    ip: Ipv4Addr,
    team: u16,
    phoenix: Option<&PhoenixVersion>,
    services: &[&str],
) -> Result<Identity, DriverError> {
    let hostname = ROBORIO_HOSTNAME.replace("{team}", &team.to_string());
    let mut versions = std::collections::BTreeMap::new();
    let mut model = "roboRIO".to_string();
    if let Some(version) = phoenix {
        if !version.system.is_empty() {
            model.clone_from(&version.system);
        }
        versions.insert("phoenix".to_string(), version.short_version().to_string());
        if let Some(season) = version.season() {
            versions.insert("season".to_string(), season.to_string());
        }
    }
    let robot_code = phoenix.is_some() || services.contains(&"nt4");
    let attributes = [
        (ATTR_TEAM, team.to_string()),
        (ATTR_HOSTNAME, hostname.clone()),
        (ATTR_MANAGE_URL, format!("http://{ip}/")),
        (
            ATTR_ROBOT_CODE,
            if robot_code { "running" } else { "not running" }.to_string(),
        ),
        (ATTR_SERVICES, services.join(",")),
    ];
    let key = DeviceKey::board(ROBORIO_FAMILY, team.to_string())
        .map_err(|error| DriverError::Incompatible(error.reason))?;
    Ok(Identity {
        key,
        model,
        state: DeviceState::RobotDevice,
        lemnos: atlas_types::LemnosState::Absent,
        versions,
        name: Some(hostname),
        link: LinkId::new(""),
        address: ip.to_string(),
        attributes: attributes
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    })
}

/// The robot controllers to look for on this link: the controller address of
/// each known team, and any address its DNS name resolves to on that team's
/// network, when port 80 is open there.
async fn discover(net: &RobotNet, link: &Link) -> Vec<Candidate> {
    let teams = net.teams();
    let names: Vec<(u16, String)> = teams
        .iter()
        .flat_map(|team| {
            let host = ROBORIO_HOSTNAME.replace("{team}", &team.to_string());
            ROBORIO_DNS_SUFFIXES
                .iter()
                .map(move |suffix| (*team, format!("{host}{suffix}")))
        })
        .collect();
    // Every name at once: a slow lookup delays only itself.
    let resolved = futures::future::join_all(names.iter().map(|(_, name)| net.resolve(name))).await;
    let mut probes: BTreeSet<Ipv4Addr> = teams
        .iter()
        .filter_map(|team| address(*team, CONTROLLER_HOST))
        .collect();
    for ((team, _), ips) in names.iter().zip(resolved) {
        probes.extend(ips.into_iter().filter(|ip| team_of(*ip) == Some(*team)));
    }
    let probes: Vec<Ipv4Addr> = probes.into_iter().collect();
    let pairs: Vec<(Ipv4Addr, u16)> = probes.iter().map(|ip| (*ip, WEB_PORT)).collect();
    let open = net.open_all(&pairs).await;
    probes
        .into_iter()
        .zip(open)
        .filter(|(_, open)| *open)
        .map(|(ip, _)| network::candidate(&link.id, ROBORIO_FAMILY, ip))
        .collect()
}

/// The driver for robot controllers. Holds no state of its own beyond the
/// shared network.
pub struct RoborioDriver {
    manifest: DriverManifest,
    net: Arc<RobotNet>,
}

impl RoborioDriver {
    pub fn new(net: Arc<RobotNet>) -> Self {
        Self {
            manifest: manifest(),
            net,
        }
    }
}

/// The manifest of the robot controller driver. Ethernet is the robot network
/// itself; a gateway is a board's USB route onto it.
pub(crate) fn manifest() -> DriverManifest {
    DriverManifest {
        family: Family::new(ROBORIO_FAMILY),
        name: "roboRIO (robot controller)".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        link_kinds: vec![LinkKind::Ethernet, gateway_kind()],
        priority: 0,
    }
}

impl std::fmt::Debug for RoborioDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoborioDriver")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Driver for RoborioDriver {
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
        let team = team_of(ip).ok_or_else(|| {
            DriverError::Incompatible(format!("{ip} is not on a robot network (10.TE.AM.x)"))
        })?;
        let mut identity = read(&self.net, ip, team).await?;
        identity.link = candidate.link.clone();
        Ok(identity)
    }

    fn capabilities(&self, identity: &Identity) -> CapabilitySet {
        let mut set = CapabilitySet::new();
        set.grant(CapabilityKind::Status);
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
    use atlas_types::{CapabilityKind as K, DeviceState as S};

    const ROOT: &str = include_str!("../tests/fixtures/roborio-root.html");
    const VERSION: &str = include_str!("../tests/fixtures/phoenix-getversion.json");

    fn rio() -> Ipv4Addr {
        Ipv4Addr::new(10, 63, 90, 2)
    }

    #[test]
    fn phoenix_reports_its_version_and_season() {
        let version: PhoenixVersion = serde_json::from_str(VERSION).unwrap();
        assert_eq!(version.system, "roboRIO");
        assert_eq!(version.short_version(), "2026.3.0");
        assert_eq!(version.season(), Some("2026"));
    }

    #[tokio::test]
    async fn a_controller_is_keyed_by_its_team_and_read_as_a_robot_device() {
        let fake = Fake::new()
            .page(rio(), 80, "/", 200, Some("Embedthis-http"), ROOT)
            .page(rio(), 1250, PHOENIX_VERSION_PATH, 200, None, VERSION)
            .open(rio(), 5810);
        let identity = read(
            &RobotNet::new(fake.reach(), Arc::new(BTreeSet::from([6390]))),
            rio(),
            6390,
        )
        .await
        .unwrap();
        assert_eq!(identity.key.to_string(), "roborio:6390");
        assert_eq!(identity.state, S::RobotDevice);
        assert_eq!(identity.model, "roboRIO");
        assert_eq!(identity.name.as_deref(), Some("roboRIO-6390-FRC"));
        assert_eq!(identity.versions["phoenix"], "2026.3.0");
        assert_eq!(identity.versions["season"], "2026");
        let attrs = &identity.attributes;
        assert_eq!(attrs[ATTR_MANAGE_URL], "http://10.63.90.2/");
        assert_eq!(attrs[ATTR_HOSTNAME], "roboRIO-6390-FRC");
        assert_eq!(attrs[ATTR_ROBOT_CODE], "running");
        assert_eq!(attrs[ATTR_SERVICES], "phoenix,nt4");
        assert_eq!(attrs[ATTR_TEAM], "6390");
    }

    #[tokio::test]
    async fn idle_controller_and_garbled_phoenix_are_not_running() {
        let fake = Fake::new()
            .page(rio(), 80, "/", 200, Some("Embedthis-http"), ROOT)
            .page(rio(), 1250, PHOENIX_VERSION_PATH, 200, None, "<<garbage>>");
        let identity = read(
            &RobotNet::new(fake.reach(), Arc::new(BTreeSet::new())),
            rio(),
            6390,
        )
        .await
        .unwrap();
        assert_eq!(identity.attributes[ATTR_ROBOT_CODE], "not running");
        assert_eq!(identity.attributes[ATTR_SERVICES], "");
        assert!(identity.versions.is_empty());
    }

    #[tokio::test]
    async fn a_server_that_is_not_ni_is_settled_not_retried() {
        let fake = Fake::new().page(rio(), 80, "/", 200, Some("nginx"), ROOT);
        let error = read(
            &RobotNet::new(fake.reach(), Arc::new(BTreeSet::new())),
            rio(),
            6390,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DriverError::Incompatible(_)), "{error:?}");
    }

    #[tokio::test]
    async fn garbage_on_the_page_port_is_settled_and_never_panics() {
        let garbage: String = (0..2000).map(|i| char::from((i % 251) as u8 + 1)).collect();
        let fake = Fake::new().page(rio(), 80, "/", 200, Some("Embedthis-http"), &garbage);
        let error = read(
            &RobotNet::new(fake.reach(), Arc::new(BTreeSet::new())),
            rio(),
            6390,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DriverError::Incompatible(_)), "{error:?}");
    }

    #[tokio::test]
    async fn no_answer_or_an_error_status_is_retried_not_settled() {
        let silent = Fake::new();
        let error = read(
            &RobotNet::new(silent.reach(), Arc::new(BTreeSet::new())),
            rio(),
            6390,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
        let failing = Fake::new().page(rio(), 80, "/", 503, Some("Embedthis-http"), ROOT);
        let error = read(
            &RobotNet::new(failing.reach(), Arc::new(BTreeSet::new())),
            rio(),
            6390,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
    }

    #[test]
    fn capabilities_are_read_only_and_only_as_they_apply() {
        let driver = RoborioDriver::new(RobotNet::new(
            Fake::new().reach(),
            Arc::new(BTreeSet::new()),
        ));
        let identity = identity(rio(), 6390, None, &[]).unwrap();
        let set = driver.capabilities(&identity);
        assert_eq!(
            set.shown(S::RobotDevice),
            vec![K::Info, K::OpenUi, K::Status]
        );
        assert!(!set.grants(K::Update) && !set.grants(K::Logs));
        let mut bare = identity;
        bare.attributes.remove(ATTR_MANAGE_URL);
        assert!(!driver.capabilities(&bare).grants(K::OpenUi));
    }

    /// A controller whose team name resolves on its own network is found by
    /// that name too; a name that resolves off the team's network is not asked.
    #[tokio::test(start_paused = true)]
    async fn a_controller_found_by_its_name_on_its_team_network() {
        let named = Ipv4Addr::new(10, 63, 90, 7);
        let stray = Ipv4Addr::new(10, 99, 1, 1);
        let fake = Fake::new()
            .name("roboRIO-6390-FRC.lan", &[named])
            .name("roboRIO-6390-FRC.local", &[stray])
            .open(named, 80);
        let driver = RoborioDriver::new(RobotNet::new(
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
        assert_eq!(addresses, ["10.63.90.7"]);
        assert!(
            !fake.requests().iter().any(|r| r.contains("10.99.1.1")),
            "{:?}",
            fake.requests()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_lists_a_controller_on_a_known_team_and_nothing_else() {
        let fake = Fake::new().open(Ipv4Addr::new(10, 63, 90, 2), 80);
        let net = RobotNet::new(fake.reach(), Arc::new(BTreeSet::from([6390, 1678])));
        let driver = RoborioDriver::new(net);
        let link = Link {
            id: LinkId::new("robot-net"),
            kind: LinkKind::Ethernet,
            label: "robot network".into(),
        };
        let found = driver.discover(&link).await.unwrap();
        let addresses: Vec<&str> = found.iter().map(|c| c.address.as_str()).collect();
        assert_eq!(addresses, ["10.63.90.2"]);
        assert_eq!(found[0].family.as_str(), ROBORIO_FAMILY);
    }
}
