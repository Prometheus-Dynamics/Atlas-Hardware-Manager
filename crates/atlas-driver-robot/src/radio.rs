//! The FRC radio (a Vivid-Hosting VH-109), read without credentials.
//!
//! Its web server (seen on team 6390's radio, firmware 2.0.1-02062026)
//! redirects `/` to `/configuration`, the configuration page, and serves
//! `GET /status` to anyone: the mode, channel, team, firmware version and
//! each band's link (SSID, signal, noise, rates). Configuring it is a
//! `POST /configuration` and a firmware upload a `POST /firmware`, neither
//! of which Atlas sends. SSH (dropbear) and DNS (`radio.lan`,
//! `roboRIO-TEAM-FRC.lan`) also answer.
//!
//! The status also carries each band's WPA key, hashed and salted. Atlas
//! never keeps or shows them.
//!
//! The radio has no serial number in its status, so its key is its team
//! number (`frc-radio:6390`), like the robot controller's.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use atlas_driver::robot_net::{RADIO_FAMILY, team_of};
use atlas_driver::{DeviceKey, DeviceMode, DriverError, Identity, LinkId, Metric, attributes};
use serde::{Deserialize, Serialize};

/// One band's link, as `/status` reports it (the WPA key fields left out).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    #[serde(default)]
    pub ssid: String,
    #[serde(default)]
    pub is_enabled: bool,
    #[serde(default)]
    pub is_linked: bool,
    /// The linked peer's MAC address.
    #[serde(default)]
    pub mac_address: String,
    #[serde(default)]
    pub signal_dbm: f64,
    #[serde(default)]
    pub noise_dbm: f64,
    #[serde(default)]
    pub signal_noise_ratio: f64,
    #[serde(default)]
    pub rx_rate_mbps: f64,
    #[serde(default)]
    pub tx_rate_mbps: f64,
    #[serde(default)]
    pub bandwidth_used_mbps: f64,
    #[serde(default)]
    pub rx_bytes: u64,
    #[serde(default)]
    pub tx_bytes: u64,
    /// `excellent`, `good`, …; empty when not linked.
    #[serde(default)]
    pub connection_quality: String,
}

/// `GET /status`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    pub ssid_suffix: String,
    #[serde(default, rename = "networkStatus24")]
    pub band_24: Option<Band>,
    #[serde(default, rename = "networkStatus6")]
    pub band_6: Option<Band>,
    /// `ACTIVE`, …
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
}

impl RadioStatus {
    pub fn parse(json: &str) -> Result<Self, String> {
        let status: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if status.version.is_empty() && status.mode.is_empty() {
            return Err("not a radio status (no version or mode)".into());
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

    fn bands(&self) -> [(&'static str, &'static str, Option<&Band>); 2] {
        [
            ("2.4", "2.4 GHz", self.band_24.as_ref()),
            ("6", "6 GHz", self.band_6.as_ref()),
        ]
    }

    /// Each linked band's signal, noise, SNR, rates and use.
    pub fn metrics(&self) -> Vec<Metric> {
        let mut metrics = Vec::new();
        for (id, label, band) in self.bands() {
            let Some(band) = band.filter(|band| band.is_linked) else {
                continue;
            };
            let metric = |name: &str, what: &str, value: f64, unit: &str| {
                Metric::new(
                    &format!("radio.{id}.{name}"),
                    &format!("{label} {what}"),
                    value,
                    Some(unit),
                )
            };
            metrics.push(
                metric("signal", "signal", band.signal_dbm, "dBm")
                    .detail(band.connection_quality.clone()),
            );
            metrics.push(metric("noise", "noise", band.noise_dbm, "dBm"));
            metrics.push(metric("snr", "SNR", band.signal_noise_ratio, "dB"));
            metrics.push(metric("rx", "receive rate", band.rx_rate_mbps, "Mbps"));
            metrics.push(metric("tx", "send rate", band.tx_rate_mbps, "Mbps"));
            metrics.push(metric(
                "used",
                "bandwidth used",
                band.bandwidth_used_mbps,
                "Mbps",
            ));
        }
        metrics
    }

    /// The radio's identity at `address`: its team from the status, or
    /// from its address when the status has none.
    pub fn to_identity(&self, link: LinkId, address: String) -> Result<Identity, DriverError> {
        let team = Some(self.team_number)
            .filter(|team| *team > 0)
            .or_else(|| address.parse::<Ipv4Addr>().ok().and_then(team_of))
            .ok_or_else(|| DriverError::Other(format!("the radio at {address} has no team")))?;
        let mut versions = BTreeMap::new();
        if let Some(firmware) = self.firmware() {
            versions.insert("firmware".to_string(), firmware.to_string());
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("team".to_string(), team.to_string());
        attrs.insert(attributes::WEB_UI.into(), format!("http://{address}/"));
        attrs.insert("radio.mode".into(), self.mode_label());
        attrs.insert("radio.status".into(), self.status.to_ascii_lowercase());
        if !self.channel.is_empty() {
            attrs.insert("radio.channel".into(), self.channel.clone());
        }
        for (id, _, band) in self.bands() {
            let Some(band) = band else { continue };
            if !band.ssid.is_empty() {
                attrs.insert(format!("radio.{id}.ssid"), band.ssid.clone());
            }
            let link = if !band.is_enabled {
                "off".to_string()
            } else if band.is_linked {
                format!(
                    "linked, {} dBm{}",
                    band.signal_dbm,
                    Some(&band.connection_quality)
                        .filter(|quality| !quality.is_empty())
                        .map(|quality| format!(", {quality}"))
                        .unwrap_or_default()
                )
            } else {
                "not linked".to_string()
            };
            attrs.insert(format!("radio.{id}.link"), link);
        }
        attrs.insert(
            "radio.6ghz_bypassed".into(),
            self.six_ghz_bypassed.to_string(),
        );
        attrs.insert("radio.qos".into(), self.qos_enabled.to_string());
        attrs.insert(
            "radio.systemcore".into(),
            self.systemcore_enabled.to_string(),
        );
        Ok(Identity {
            key: DeviceKey::new(RADIO_FAMILY, team.to_string()),
            model: self.model().to_string(),
            mode: DeviceMode::Normal,
            versions,
            name: Some(format!("radio-{team}")),
            link,
            address,
            attributes: attrs,
        })
    }
}

/// `GET http://ip/status`.
pub async fn read(http: &reqwest::Client, ip: Ipv4Addr) -> Result<RadioStatus, DriverError> {
    let body = http
        .get(format!("http://{ip}/status"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?
        .text()
        .await
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?;
    RadioStatus::parse(&body).map_err(|error| DriverError::Other(format!("{ip}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = include_str!("../tests/fixtures/vh109-status.json");

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
    fn a_radio_is_keyed_by_its_team_and_never_shows_its_keys() {
        let status = RadioStatus::parse(STATUS).unwrap();
        let identity = status
            .to_identity(LinkId("robot-net".into()), "10.63.90.4".into())
            .unwrap();
        assert_eq!(identity.key.to_string(), "frc-radio:6390");
        assert_eq!(identity.model, "VH-109");
        assert_eq!(identity.versions["firmware"], "2.0.1-02062026");
        let attrs = &identity.attributes;
        assert_eq!(attrs["manage_url"], "http://10.63.90.4/");
        assert_eq!(attrs["radio.channel"], "53");
        assert_eq!(attrs["radio.2.4.ssid"], "6390");
        assert_eq!(attrs["radio.2.4.link"], "linked, -65 dBm, excellent");
        assert_eq!(attrs["radio.6.link"], "not linked");
        assert_eq!(attrs["radio.status"], "active");
        let text = serde_json::to_string(&identity).unwrap().to_lowercase();
        assert!(!text.contains("wpa"), "{text}");
        assert!(!serde_json::to_string(&status).unwrap().contains("Wpa"));
    }

    #[test]
    fn only_linked_bands_have_metrics() {
        let metrics = RadioStatus::parse(STATUS).unwrap().metrics();
        let ids: Vec<&str> = metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "radio.2.4.signal",
                "radio.2.4.noise",
                "radio.2.4.snr",
                "radio.2.4.rx",
                "radio.2.4.tx",
                "radio.2.4.used"
            ]
        );
        assert_eq!(metrics[0].value, -65.0);
        assert_eq!(metrics[2].value, 29.0);
    }
}
