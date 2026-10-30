//! Boards reached through a robot controller's HeliOS (a SystemCore), the
//! way HeliOS deliberately offers them (its docs/device-profiles.md,
//! "Atlas's routes to a peer"): `GET /v1/peers` lists the robot's other
//! boards, `GET /v1/peers/<node>/board/identity` is a peer board's identity
//! document, and `/v1/peers/<node>/orion` (HTTP Upgrade `orion`) a tunnel
//! to its orion-node, only while the robot's security is off, with Orion's
//! own auth end to end. Nothing else of the board is reachable that way: no
//! SSH, no board-agent endpoints. A board's address in Atlas is that
//! identity URL.

/// A board reached through a controller: the controller's HeliOS at
/// `host:port`, the board as node `node` there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ControllerPeer {
    pub host: String,
    pub port: u16,
    pub node: String,
}

const IDENTITY: &str = "/board/identity";

impl ControllerPeer {
    /// The peer behind the address Atlas reached a board at, if it is a
    /// controller's identity URL.
    pub fn from_address(address: &str) -> Option<Self> {
        let rest = address.strip_prefix("http://")?;
        let (authority, path) = rest.split_once('/')?;
        let (host, port) = authority.rsplit_once(':')?;
        let node = path.strip_prefix("v1/peers/")?.strip_suffix(IDENTITY)?;
        let port = port.parse().ok()?;
        valid_node(node).then(|| Self {
            host: host.to_string(),
            port,
            node: node.to_string(),
        })
    }

    /// The board's identity document through the controller.
    pub fn identity_url(&self) -> String {
        format!(
            "http://{}:{}/v1/peers/{}{IDENTITY}",
            self.host, self.port, self.node
        )
    }

    /// Where a file is uploaded to the board's own HeliOS
    /// (`/v1/update/uploads`, through the controller): it answers with the
    /// upload's `image_url`, a `file://` path on the board.
    pub fn upload_url(&self) -> String {
        format!(
            "http://{}:{}/v1/peers/{}/update/uploads",
            self.host, self.port, self.node
        )
    }

    /// The path of the tunnel to the board's orion-node.
    pub fn orion_path(&self) -> String {
        format!("/v1/peers/{}/orion", self.node)
    }
}

/// A node id as HeliOS names peers (`raze-37bc`): nothing that could leave
/// the path.
pub fn valid_node(node: &str) -> bool {
    !node.is_empty()
        && node.len() <= 64
        && node
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !node.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_controllers_identity_url_names_the_peer() {
        let peer = ControllerPeer::from_address(
            "http://172.27.0.1:5880/v1/peers/raze-37bc/board/identity",
        )
        .unwrap();
        assert_eq!(peer.host, "172.27.0.1");
        assert_eq!(peer.port, 5880);
        assert_eq!(peer.node, "raze-37bc");
        assert_eq!(
            peer.identity_url(),
            "http://172.27.0.1:5880/v1/peers/raze-37bc/board/identity"
        );
        assert_eq!(peer.orion_path(), "/v1/peers/raze-37bc/orion");
        assert_eq!(
            peer.upload_url(),
            "http://172.27.0.1:5880/v1/peers/raze-37bc/update/uploads"
        );
        for not in [
            "http://10.0.0.5:5899/.well-known/pd-device",
            "http://h:5880/v1/peers/../board/identity",
            "http://h:5880/v1/peers/a/b/board/identity",
            "usb:1-4",
        ] {
            assert_eq!(ControllerPeer::from_address(not), None, "{not}");
        }
    }
}
