//! Boards reached through a robot controller's HeliOS (a SystemCore), the
//! way HeliOS deliberately offers them (its docs/device-profiles.md,
//! "Atlas's routes to a peer"): `GET /v1/peers` lists the robot's other
//! boards, `GET /v1/peers/<node>/board/identity` is a peer board's identity
//! document, and `/v1/peers/<node>/orion` (HTTP Upgrade `orion`) a tunnel
//! to its orion-node, only while the robot's security is off, with Orion's
//! own auth end to end. `/v1/peers/<node>/<path>` passes the board's own
//! HeliOS API (`/v1/<path>`): Restart and slots (atlas-driver-board's
//! gateway.rs). No SSH, no other board-agent endpoints. A board's address in
//! Atlas is that identity URL.

/// A running board a controller sees on its USB (Atlas's USB agent lists
/// it) but whose HeliOS doesn't offer as a peer: Atlas knows it's there and
/// has no way to it. Its address is `seen-usb://<controller>/<usb port>/<serial>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenOnUsb {
    pub controller: String,
    pub usb_port: String,
    /// The board's serial as its USB descriptor gives it.
    pub serial: String,
}

const SEEN_SCHEME: &str = "seen-usb://";

impl SeenOnUsb {
    pub fn address(&self) -> String {
        format!(
            "{SEEN_SCHEME}{}/{}/{}",
            self.controller, self.usb_port, self.serial
        )
    }

    pub fn from_address(address: &str) -> Option<Self> {
        let mut parts = address.strip_prefix(SEEN_SCHEME)?.split('/');
        let (controller, usb_port, serial) = (parts.next()?, parts.next()?, parts.next()?);
        let plain = |part: &str| {
            part.chars().any(|c| c.is_ascii_alphanumeric())
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        };
        (parts.next().is_none() && plain(controller) && plain(usb_port) && plain(serial)).then(
            || Self {
                controller: controller.into(),
                usb_port: usb_port.into(),
                serial: serial.into(),
            },
        )
    }
}

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
    fn a_board_seen_on_a_controllers_usb_round_trips_its_address() {
        let seen = SeenOnUsb {
            controller: "172.27.0.1".into(),
            usb_port: "3-1.1.4.1".into(),
            serial: "a317bcbee5226d57".into(),
        };
        assert_eq!(
            seen.address(),
            "seen-usb://172.27.0.1/3-1.1.4.1/a317bcbee5226d57"
        );
        assert_eq!(SeenOnUsb::from_address(&seen.address()), Some(seen));
        assert_eq!(SeenOnUsb::from_address("seen-usb://h/3-1/../x"), None);
        assert_eq!(SeenOnUsb::from_address("http://172.31.209.217:5899/"), None);
    }

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
