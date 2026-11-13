//! Who a node is, from its hello document.
//!
//! Orion signs the hello document and invents no board facts. The program on the board that
//! knows a fact publishes it as a topic `hello.<fact>`, and the document carries each one's
//! newest text value. Atlas reads these facts:
//!
//! | Fact | Needed | Means |
//! |---|---|---|
//! | `family` | yes | the device family (with `serial`, the device's [`DeviceKey`]) |
//! | `serial` | yes | the hardware serial number |
//! | `state` | yes | `running`, `recovery` or `flasher` (a freshly loaded flasher image) |
//! | `model` | no | the hardware model, as data |
//! | `name` | no | the name stored on the device |
//! | `version.<name>` | no | a named version, such as `os` or `bootloader` |
//! | `offers` | no | comma-separated capability kinds the node offers, kebab-case |
//!
//! Any other fact is shown as an attribute. The names above are Atlas's reading of the
//! hello document; Lemnos and the board's images publish them.

use std::collections::BTreeMap;

use atlas_driver::DriverError;
use atlas_types::{CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Identity, LinkId};
use orion::node::Node;

/// The `UTF-8 text` field type number in Orion's topic declarations.
const TEXT: u8 = 4;
/// The attribute that carries the offers to [`capabilities`].
const OFFERS: &str = "offers";

/// The identity of `node`, as found on `link` at `address`.
///
/// # Errors
///
/// If the node does not publish its family, serial or state, or publishes a state Atlas does
/// not know. The message says what is missing.
pub fn identity(node: &Node, link: &LinkId, address: &str) -> Result<Identity, DriverError> {
    let text = |name: &str| -> Option<String> {
        node.facts
            .iter()
            .find(|fact| fact.name == name && fact.ty == TEXT)
            .and_then(|fact| String::from_utf8(fact.value.clone()).ok())
    };
    let required = |name: &str| {
        text(name).ok_or_else(|| {
            DriverError::Incompatible(format!(
                "the node does not publish hello.{name}, so it cannot be identified"
            ))
        })
    };

    let family = required("family")?;
    let serial = required("serial")?;
    let state = match required("state")?.as_str() {
        "running" => DeviceState::Running,
        "recovery" | "flasher" => DeviceState::Recovery,
        other => {
            return Err(DriverError::Incompatible(format!(
                "the node reports state {other:?}, which Atlas does not know"
            )));
        }
    };

    let mut versions = BTreeMap::new();
    let mut attributes = BTreeMap::new();
    for fact in &node.facts {
        let Some(value) = text(&fact.name) else {
            continue;
        };
        match fact.name.as_str() {
            "family" | "serial" | "state" | "model" | "name" => {}
            name => match name.strip_prefix("version.") {
                Some(version) => {
                    versions.insert(version.to_owned(), value);
                }
                None => {
                    attributes.insert(name.to_owned(), value);
                }
            },
        }
    }
    attributes.insert("fingerprint".into(), node.fingerprint.clone());
    attributes.insert("cluster".into(), node.cluster.clone());
    attributes.insert("node".into(), node.name.clone());

    Ok(Identity {
        key: DeviceKey::new(family, serial),
        model: text("model").unwrap_or_default(),
        state,
        versions,
        name: text("name"),
        link: link.clone(),
        address: address.to_owned(),
        attributes,
    })
}

/// The capabilities an identified node grants: the kinds it offers, read from `offers`.
/// Unknown names are ignored, and `info` is always there without being offered.
pub fn capabilities(identity: &Identity) -> CapabilitySet {
    let offered = identity
        .attributes
        .get(OFFERS)
        .map(String::as_str)
        .unwrap_or_default();
    CapabilitySet::from_kinds(offered.split(',').map(str::trim).filter_map(kind_named))
}

/// The kind a kebab-case name stands for. Exhaustive, so a new kind must be named here.
pub fn kind_named(name: &str) -> Option<CapabilityKind> {
    use CapabilityKind::*;
    let kind = match name {
        "info" => Info,
        "update" => Update,
        "recover" => Recover,
        "config" => Config,
        "logs" => Logs,
        "telemetry" => Telemetry,
        "actions" => Actions,
        "gateway" => Gateway,
        "open-ui" => OpenUi,
        "self-test" => SelfTest,
        "status" => Status,
        "hardware-control" => HardwareControl,
        "slots" => Slots,
        "install" => Install,
        "files" => Files,
        "shell" => Shell,
        "flash" => Flash,
        _ => return None,
    };
    Some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orion::node::Fact;

    fn fact(name: &str, value: &str) -> Fact {
        Fact {
            name: name.into(),
            ty: TEXT,
            value: value.as_bytes().to_vec(),
        }
    }

    fn node(facts: Vec<Fact>) -> Node {
        Node {
            name: "board".into(),
            cluster: "lab".into(),
            public_key: [7; 32],
            fingerprint: "ab:cd".into(),
            revision: 1,
            time_ms: 0,
            facts,
        }
    }

    fn link() -> LinkId {
        LinkId::new("usbnet:a")
    }

    #[test]
    fn family_and_serial_make_the_key_and_state_is_read() {
        let found = identity(
            &node(vec![
                fact("family", "test-family"),
                fact("serial", "0001"),
                fact("state", "running"),
                fact("model", "test-model"),
                fact("version.os", "1.2.3"),
                fact("offers", "logs,update"),
            ]),
            &link(),
            "orion+tcp://10.0.0.2:9200",
        )
        .unwrap();
        assert_eq!(found.key, DeviceKey::new("test-family", "0001"));
        assert_eq!(found.state, DeviceState::Running);
        assert_eq!(found.model, "test-model");
        assert_eq!(found.versions.get("os").map(String::as_str), Some("1.2.3"));
        assert_eq!(
            found.attributes.get("fingerprint").map(String::as_str),
            Some("ab:cd")
        );
        assert_eq!(found.link, link());
    }

    #[test]
    fn a_freshly_loaded_flasher_is_in_recovery() {
        let found = identity(
            &node(vec![
                fact("family", "f"),
                fact("serial", "1"),
                fact("state", "flasher"),
            ]),
            &link(),
            "x",
        )
        .unwrap();
        assert_eq!(found.state, DeviceState::Recovery);
    }

    #[test]
    fn a_node_without_its_key_or_state_is_refused_with_the_missing_fact() {
        let no_serial = identity(
            &node(vec![fact("family", "f"), fact("state", "running")]),
            &link(),
            "x",
        );
        match no_serial {
            Err(DriverError::Incompatible(message)) => assert!(message.contains("hello.serial")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        let unknown = identity(
            &node(vec![
                fact("family", "f"),
                fact("serial", "1"),
                fact("state", "boot-rom"),
            ]),
            &link(),
            "x",
        );
        assert!(matches!(unknown, Err(DriverError::Incompatible(_))));
    }

    #[test]
    fn a_non_text_fact_is_not_read_as_a_name() {
        let mut serial = fact("serial", "1");
        serial.ty = 2;
        let refused = identity(
            &node(vec![fact("family", "f"), serial, fact("state", "running")]),
            &link(),
            "x",
        );
        assert!(matches!(refused, Err(DriverError::Incompatible(_))));
    }

    #[test]
    fn capabilities_are_the_offered_kinds_and_nothing_else() {
        let found = identity(
            &node(vec![
                fact("family", "f"),
                fact("serial", "1"),
                fact("state", "running"),
                fact("offers", "logs, update, not-a-kind"),
            ]),
            &link(),
            "x",
        )
        .unwrap();
        let grants = capabilities(&found);
        assert!(grants.grants(CapabilityKind::Logs));
        assert!(grants.grants(CapabilityKind::Update));
        assert!(!grants.grants(CapabilityKind::Slots));
    }

    #[test]
    fn a_node_that_offers_nothing_grants_only_info() {
        let found = identity(
            &node(vec![
                fact("family", "f"),
                fact("serial", "1"),
                fact("state", "running"),
            ]),
            &link(),
            "x",
        )
        .unwrap();
        assert_eq!(
            capabilities(&found).shown(DeviceState::Running),
            vec![CapabilityKind::Info]
        );
    }

    #[test]
    fn every_kind_has_its_name() {
        for kind in CapabilityKind::ALL {
            let name = serde_json::to_value(kind).unwrap();
            assert_eq!(kind_named(name.as_str().unwrap()), Some(kind));
        }
    }
}
