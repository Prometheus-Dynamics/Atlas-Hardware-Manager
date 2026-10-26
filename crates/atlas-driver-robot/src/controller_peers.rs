//! The boards behind a SystemCore, as its HeliOS lists them
//! (`atlas_driver::controller_peer`): each routed peer's identity document
//! through the controller, as a candidate for that board's own driver.

use std::net::Ipv4Addr;

use atlas_driver::controller_peer::{ControllerPeer, valid_node};
use atlas_driver::{Candidate, DriverError, Family, LinkId};
use serde::Deserialize;

/// HeliOS's API on the controller.
pub const HELIOS_PORT: u16 = 5880;
/// The link boards behind a controller are found on.
pub const LINK_ID: &str = "controller";

#[derive(Debug, Deserialize)]
struct Peers {
    #[serde(default)]
    peers: Vec<Peer>,
}

#[derive(Debug, Deserialize)]
struct Peer {
    node_id: String,
    #[serde(default)]
    routed: bool,
}

#[derive(Debug, Deserialize)]
struct Model {
    model: String,
}

/// The routed peers' nodes from `GET /v1/peers`'s answer.
pub fn parse_peers(body: &str) -> Result<Vec<String>, String> {
    let peers: Peers = serde_json::from_str(body)
        .map_err(|error| format!("its peer list isn't what HeliOS sends: {error}"))?;
    Ok(peers
        .peers
        .into_iter()
        .filter(|peer| peer.routed && valid_node(&peer.node_id))
        .map(|peer| peer.node_id)
        .collect())
}

/// The boards behind the controller at `ip`: none when its HeliOS doesn't
/// answer (another OS, or HeliOS without peers).
pub async fn candidates(
    http: &reqwest::Client,
    ip: Ipv4Addr,
) -> Result<Vec<Candidate>, DriverError> {
    let Ok(response) = http
        .get(format!("http://{ip}:{HELIOS_PORT}/v1/peers"))
        .send()
        .await
    else {
        return Ok(Vec::new());
    };
    if !response.status().is_success() {
        return Ok(Vec::new());
    }
    let body = response
        .text()
        .await
        .map_err(|error| DriverError::Unreachable(format!("{ip}: {error}")))?;
    let nodes = parse_peers(&body).map_err(DriverError::Other)?;
    let mut found = Vec::new();
    for node in nodes {
        let peer = ControllerPeer {
            host: ip.to_string(),
            port: HELIOS_PORT,
            node,
        };
        let url = peer.identity_url();
        // The model picks the board's driver; a peer that isn't a board
        // with an identity document (another controller's service) is left out.
        let Ok(response) = http.get(&url).send().await else {
            continue;
        };
        let Ok(model) = response.error_for_status() else {
            continue;
        };
        let Ok(model) = model.json::<Model>().await else {
            continue;
        };
        if model.model.is_empty() {
            continue;
        }
        found.push(Candidate {
            link: LinkId(LINK_ID.into()),
            family: Family::new(model.model.to_ascii_lowercase()),
            address: url,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routed_peers_are_the_boards_behind_it() {
        // A real answer (HeliOS eda1170c on the test bed, 2026-10-26).
        let body = r#"{"security":"open","cluster":null,"peers":[{"node_id":"raze-37bc","state":"discovered","address":"172.31.12.81","api_port":5800,"key_fingerprint":"","needs_enrollment":false,"routed":true,"found_by":"neighbour"},{"node_id":"cam-2","routed":false},{"node_id":"../x","routed":true}]}"#;
        assert_eq!(parse_peers(body).unwrap(), vec!["raze-37bc".to_string()]);
        assert!(parse_peers("<html>").is_err());
    }
}
