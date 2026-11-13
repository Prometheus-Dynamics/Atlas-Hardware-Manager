//! The robot radio (a Vivid-Hosting VH-109), read without credentials.
//!
//! Its web server serves `GET /status` to anyone on the robot network: the
//! mode, channel, team, firmware version, and each band's link (SSID, signal,
//! noise, quality). The configuration page and firmware upload take a POST;
//! this crate sends none.
//!
//! The same status carries each band's WPA key, hashed and salted. The parse
//! below names only the fields it shows, so those keys are never read into
//! memory past the body, never in the identity and never in a log.
//!
//! It is keyed by team (`frc-radio:6390`): the radio has no serial in its
//! status. Its state is `RobotDevice`. Its capabilities are `Status` (the
//! radio's own status), `Telemetry` (each linked band's readings, shown as
//! attributes) and `OpenUi` (its page).

use std::collections::BTreeMap;
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
    ATTR_MANAGE_URL, ATTR_TEAM, RADIO_FAMILY, RADIO_HOSTS, RADIO_STATUS_PATH, WEB_PORT,
};
use crate::network::{self, RobotNet};
use crate::team::{address, team_of};

/// One band's link, as `/status` reports it. Only these fields are read.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    #[serde(default)]
    pub ssid: String,
    #[serde(default)]
    pub is_enabled: bool,
    #[serde(default)]
    pub is_linked: bool,
    #[serde(default)]
    pub signal_dbm: f64,
    /// `excellent`, `good`, …; empty when not linked.
    #[serde(default)]
    pub connection_quality: String,
}

/// `GET /status`, the fields this crate shows.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioStatus {
    /// `TEAM_ACCESS_POINT` or `ROBOT_RADIO`.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub team_number: u16,
    #[serde(default)]
    pub status: String,
    /// `VH-109_2.0.1-02062026`: model, then firmware.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub six_ghz_bypassed: bool,
    #[serde(default)]
    pub qos_enabled: bool,
    #[serde(default)]
    pub systemcore_enabled: bool,
    #[serde(default, rename = "networkStatus24")]
    pub band_24: Option<Band>,
    #[serde(default, rename = "networkStatus6")]
    pub band_6: Option<Band>,
}

impl RadioStatus {
    /// Reads a status answer. Anything that is not one is an error that says why.
    pub fn parse(json: &str) -> Result<Self, String> {
        let status: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if status.version.is_empty() && status.mode.is_empty() {
            return Err("no version or mode in it".into());
        }
        Ok(status)
    }

    /// `VH-109`.
    pub fn model(&self) -> &str {
        self.version
            .split_once('_')
            .map_or("FRC radio", |(model, _)| model)
    }

    /// `2.0.1-02062026`.
    pub fn firmware(&self) -> Option<&str> {
        self.version.split_once('_').map(|(_, firmware)| firmware)
    }

    /// The mode in words: `team access point`.
    pub fn mode_label(&self) -> String {
        self.mode.to_ascii_lowercase().replace('_', " ")
    }

    /// The identity at `ip`. The team is the status's own, else the address's.
    pub fn identity(&self, ip: Ipv4Addr) -> Result<Identity, DriverError> {
        let team = Some(self.team_number)
            .filter(|team| *team > 0)
            .or_else(|| team_of(ip))
            .ok_or_else(|| DriverError::Incompatible(format!("the radio at {ip} names no team")))?;
        let mut versions = BTreeMap::new();
        if let Some(firmware) = self.firmware() {
            versions.insert("firmware".to_string(), firmware.to_string());
        }
        let mut attributes = BTreeMap::new();
        attributes.insert(ATTR_TEAM.to_string(), team.to_string());
        attributes.insert(ATTR_MANAGE_URL.to_string(), format!("http://{ip}/"));
        attributes.insert("radio.mode".into(), self.mode_label());
        attributes.insert("radio.status".into(), self.status.to_ascii_lowercase());
        if !self.channel.is_empty() {
            attributes.insert("radio.channel".into(), self.channel.clone());
        }
        for (id, band) in [("2.4", &self.band_24), ("6", &self.band_6)] {
            let Some(band) = band else { continue };
            if !band.ssid.is_empty() {
                attributes.insert(format!("radio.{id}.ssid"), band.ssid.clone());
            }
            attributes.insert(format!("radio.{id}.link"), band.link_text());
        }
        attributes.insert(
            "radio.6ghz_bypassed".into(),
            self.six_ghz_bypassed.to_string(),
        );
        attributes.insert("radio.qos".into(), self.qos_enabled.to_string());
        attributes.insert(
            "radio.systemcore".into(),
            self.systemcore_enabled.to_string(),
        );
        Ok(Identity {
            key: DeviceKey::new(RADIO_FAMILY, team.to_string()),
            model: self.model().to_string(),
            state: DeviceState::RobotDevice,
            versions,
            name: Some(format!("radio-{team}")),
            link: LinkId::new(""),
            address: ip.to_string(),
            attributes,
        })
    }
}

impl Band {
    /// The band's link in words: `off`, `not linked`, or `linked, -65 dBm, excellent`.
    fn link_text(&self) -> String {
        if !self.is_enabled {
            "off".into()
        } else if self.is_linked {
            let quality = Some(&self.connection_quality)
                .filter(|quality| !quality.is_empty())
                .map(|quality| format!(", {quality}"))
                .unwrap_or_default();
            format!("linked, {} dBm{quality}", self.signal_dbm)
        } else {
            "not linked".into()
        }
    }
}

/// The radio at `ip`: its status, read and checked.
async fn read(net: &RobotNet, ip: Ipv4Addr) -> Result<Identity, DriverError> {
    let answer = network::expect_ok(ip, WEB_PORT, net.get(ip, WEB_PORT, RADIO_STATUS_PATH).await)?;
    let status = RadioStatus::parse(&answer.body).map_err(|why| {
        DriverError::Incompatible(format!(
            "{ip} answers on port {WEB_PORT}, but not as a robot radio: {why}"
        ))
    })?;
    status.identity(ip)
}

/// The radios to look for: each known team's two radio addresses, where port 80 is open.
async fn discover(net: &RobotNet, link: &Link) -> Vec<Candidate> {
    let probes: Vec<Ipv4Addr> = net
        .teams()
        .into_iter()
        .flat_map(|team| RADIO_HOSTS.map(|host| address(team, host)))
        .flatten()
        .collect();
    let pairs: Vec<(Ipv4Addr, u16)> = probes.iter().map(|ip| (*ip, WEB_PORT)).collect();
    let open = net.open_all(&pairs).await;
    probes
        .into_iter()
        .zip(open)
        .filter(|(_, open)| *open)
        .map(|(ip, _)| network::candidate(&link.id, RADIO_FAMILY, ip))
        .collect()
}

/// The driver for robot radios.
pub struct RadioDriver {
    manifest: DriverManifest,
    net: Arc<RobotNet>,
}

impl RadioDriver {
    pub fn new(net: Arc<RobotNet>) -> Self {
        Self {
            manifest: manifest(),
            net,
        }
    }
}

pub(crate) fn manifest() -> DriverManifest {
    DriverManifest {
        family: Family::new(RADIO_FAMILY),
        name: "FRC radio".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        link_kinds: vec![LinkKind::Ethernet, gateway_kind()],
        priority: 0,
    }
}

impl std::fmt::Debug for RadioDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RadioDriver")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Driver for RadioDriver {
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
        let mut set =
            CapabilitySet::from_kinds([CapabilityKind::Status, CapabilityKind::Telemetry]);
        if identity.attributes.contains_key(ATTR_MANAGE_URL) {
            set.grant(CapabilityKind::OpenUi);
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::fake::Fake;

    const STATUS: &str = include_str!("../tests/fixtures/vh109-status.json");

    fn radio() -> Ipv4Addr {
        Ipv4Addr::new(10, 63, 90, 4)
    }

    #[test]
    fn the_status_names_the_model_firmware_and_mode() {
        let status = RadioStatus::parse(STATUS).unwrap();
        assert_eq!(status.model(), "VH-109");
        assert_eq!(status.firmware(), Some("2.0.1-02062026"));
        assert_eq!(status.mode_label(), "team access point");
        assert_eq!(status.team_number, 6390);
        assert!(RadioStatus::parse("{}").is_err());
        assert!(RadioStatus::parse("<html>").is_err());
    }

    #[test]
    fn a_radio_is_keyed_by_its_team_and_its_keys_never_reach_the_identity() {
        let identity = RadioStatus::parse(STATUS)
            .unwrap()
            .identity(radio())
            .unwrap();
        assert_eq!(identity.key.to_string(), "frc-radio:6390");
        assert_eq!(identity.state, DeviceState::RobotDevice);
        assert_eq!(identity.model, "VH-109");
        assert_eq!(identity.versions["firmware"], "2.0.1-02062026");
        let attrs = &identity.attributes;
        assert_eq!(attrs[ATTR_MANAGE_URL], "http://10.63.90.4/");
        assert_eq!(attrs["radio.channel"], "53");
        assert_eq!(attrs["radio.2.4.ssid"], "6390");
        assert_eq!(attrs["radio.2.4.link"], "linked, -65 dBm, excellent");
        assert_eq!(attrs["radio.6.link"], "not linked");
        assert_eq!(attrs["radio.status"], "active");
        let text = serde_json::to_string(&identity).unwrap().to_lowercase();
        assert!(!text.contains("wpa"), "{text}");
        assert!(!text.contains("redacted"), "{text}");
        assert!(!text.contains("hashed"), "{text}");
    }

    #[test]
    fn a_radio_with_no_team_in_its_status_takes_it_from_its_address() {
        let status = RadioStatus {
            team_number: 0,
            version: "VH-109_2.0.1-02062026".into(),
            ..RadioStatus::default()
        };
        assert_eq!(
            status.identity(radio()).unwrap().key.to_string(),
            "frc-radio:6390"
        );
        assert!(matches!(
            status.identity(Ipv4Addr::new(192, 168, 1, 4)),
            Err(DriverError::Incompatible(_))
        ));
    }

    #[tokio::test]
    async fn garbage_or_a_stranger_on_the_radio_port_is_settled() {
        for body in [
            "<html>not a radio</html>",
            "{}",
            "[1, 2, 3",
            "\u{0}\u{7f}\u{1b}",
        ] {
            let fake = Fake::new().page(radio(), 80, RADIO_STATUS_PATH, 200, None, body);
            let net = RobotNet::new(fake.reach(), Arc::new(BTreeSet::new()));
            let error = read(&net, radio()).await.unwrap_err();
            assert!(
                matches!(error, DriverError::Incompatible(_)),
                "{body}: {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_silent_radio_is_retried_not_settled() {
        let net = RobotNet::new(Fake::new().reach(), Arc::new(BTreeSet::new()));
        let error = read(&net, radio()).await.unwrap_err();
        assert!(matches!(error, DriverError::Unreachable(_)), "{error:?}");
    }

    #[test]
    fn a_linked_band_reads_as_its_link_and_an_off_band_as_off() {
        let band = Band {
            is_enabled: false,
            ..Band::default()
        };
        assert_eq!(band.link_text(), "off");
        assert_eq!(
            Band {
                is_enabled: true,
                ..Band::default()
            }
            .link_text(),
            "not linked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_finds_the_access_point_and_not_the_robot_radio_when_only_it_answers() {
        let fake = Fake::new().open(Ipv4Addr::new(10, 63, 90, 4), 80);
        let driver = RadioDriver::new(RobotNet::new(
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
        assert_eq!(addresses, ["10.63.90.4"]);
        assert_eq!(found[0].family.as_str(), RADIO_FAMILY);
    }

    #[test]
    fn a_radio_grants_status_and_telemetry_and_only_shows_them_for_a_robot_device() {
        let driver = RadioDriver::new(RobotNet::new(
            Fake::new().reach(),
            Arc::new(BTreeSet::new()),
        ));
        let identity = RadioStatus::parse(STATUS)
            .unwrap()
            .identity(radio())
            .unwrap();
        let set = driver.capabilities(&identity);
        assert_eq!(
            set.shown(DeviceState::RobotDevice),
            vec![
                CapabilityKind::Info,
                CapabilityKind::Telemetry,
                CapabilityKind::OpenUi,
                CapabilityKind::Status
            ]
        );
        assert!(!set.grants(CapabilityKind::Update));
    }
}
