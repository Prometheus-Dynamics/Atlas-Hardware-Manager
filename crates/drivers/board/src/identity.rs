//! Who a node is, from its hello document and what its Lemnos says.
//!
//! The hello document is Orion's signed answer. Its fields are `name`, `revision`, the key
//! fingerprint and `state` (Orion's own, `running`, `recovery` or `flasher`). Hardware facts
//! are published by Lemnos as topics `hello.<fact>`: `family`, `serial`, `model`,
//! `version.<name>` and `offers`. A fact that cannot be known is absent, never guessed.
//!
//! The device key is `family:serial` when the board publishes both, else the node's Orion key
//! fingerprint (under this driver's label, see [`FAMILY`](crate::FAMILY)). A node without
//! Lemnos is not refused; only jobs that need a board identity refuse, and they check
//! [`has_board_identity`].

use std::collections::BTreeMap;

use atlas_driver::DriverError;
use atlas_types::{CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Identity, LinkId};
use orion::node::Node;

use crate::lemnos::Lemnos;

/// The `UTF-8 text` field type number in Orion's topic declarations.
const TEXT: u8 = 4;
/// The attribute that carries the offers, as the node published them.
pub const OFFERS: &str = "offers";
/// The attribute that says where the key comes from: `hardware` or `fingerprint`.
pub const KEY_SOURCE: &str = "key-source";
/// The attribute that carries Lemnos's state (see [`Lemnos::word`]).
pub const LEMNOS: &str = "lemnos";
/// The attribute that carries the reason behind a Lemnos state that has one.
pub const LEMNOS_DETAIL: &str = "lemnos-detail";

/// What every Orion node provides, Lemnos or not: its files, shell, logs, topics and actions.
const ORION_OWN: [CapabilityKind; 5] = [
    CapabilityKind::Files,
    CapabilityKind::Shell,
    CapabilityKind::Logs,
    CapabilityKind::Telemetry,
    CapabilityKind::Actions,
];

/// The identity of `node`, as found at `address` on `link`, with what its Lemnos is doing.
///
/// # Errors
///
/// If the node publishes a state Atlas does not know. A node with no state published is
/// running: Orion publishes `recovery` and a flasher publishes `flasher`.
pub fn identity(
    node: &Node,
    lemnos: &Lemnos,
    family_label: &str,
    link: &LinkId,
    address: &str,
) -> Result<Identity, DriverError> {
    let fact = |name: &str| -> Option<String> {
        node.facts
            .iter()
            .find(|fact| fact.name == name && fact.ty == TEXT)
            .and_then(|fact| String::from_utf8(fact.value.clone()).ok())
            .filter(|value| !value.is_empty())
    };
    let state = match fact("state").as_deref() {
        None | Some("running") => DeviceState::Running,
        Some("recovery" | "flasher") => DeviceState::Recovery,
        Some(other) => {
            return Err(DriverError::Incompatible(format!(
                "the node reports state {other:?}, which Atlas does not know"
            )));
        }
    };

    let (key, key_source) = match (fact("family"), fact("serial")) {
        (Some(family), Some(serial)) => (DeviceKey::new(family, serial), "hardware"),
        _ => (
            DeviceKey::new(family_label, node.fingerprint.clone()),
            "fingerprint",
        ),
    };

    let mut versions = BTreeMap::new();
    let mut attributes = BTreeMap::new();
    for item in &node.facts {
        let Some(value) = fact(&item.name) else {
            continue;
        };
        match item.name.as_str() {
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
    attributes.insert(KEY_SOURCE.into(), key_source.into());
    attributes.insert(LEMNOS.into(), lemnos.word().into());
    if let Some(detail) = lemnos.detail() {
        attributes.insert(LEMNOS_DETAIL.into(), detail.into());
    }
    attributes.insert("fingerprint".into(), node.fingerprint.clone());
    attributes.insert("cluster".into(), node.cluster.clone());
    attributes.insert("revision".into(), node.revision.to_string());

    Ok(Identity {
        key,
        model: fact("model").unwrap_or_default(),
        state,
        lemnos: atlas_types::LemnosState::Absent,
        versions,
        name: Some(node.name.clone()),
        link: link.clone(),
        address: address.to_owned(),
        attributes,
    })
}

/// Whether the identity is a board's own (family and serial), which flashing and slot installs
/// need. A plain Orion node is identified by its fingerprint and cannot be flashed.
pub fn has_board_identity(identity: &Identity) -> bool {
    identity.attributes.get(KEY_SOURCE).map(String::as_str) == Some("hardware")
}

/// The capabilities an identified node grants.
///
/// Orion's own services are granted for every node. The Lemnos contracts are granted only
/// when the node offers them by name: `slots`, `update`, `recover`, `hardware` (hardware
/// control), `status`, `self-test`, `install`, `config`. Reboot and the other Lemnos actions
/// are actions, granted with Orion's own.
pub fn capabilities(identity: &Identity) -> CapabilitySet {
    let offered = identity
        .attributes
        .get(OFFERS)
        .map(String::as_str)
        .unwrap_or_default();
    CapabilitySet::from_kinds(
        ORION_OWN
            .into_iter()
            .chain(offered.split(',').map(str::trim).filter_map(offer_kind)),
    )
}

/// The kind an offer names, if it is one of the Lemnos contracts.
fn offer_kind(name: &str) -> Option<CapabilityKind> {
    match name {
        "hardware" => Some(CapabilityKind::HardwareControl),
        "reboot" | "" => None,
        other => kind_named(other),
    }
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

    const LABEL: &str = "orion";

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
            revision: 3,
            time_ms: 0,
            facts,
        }
    }

    fn link() -> LinkId {
        LinkId::new("usbnet:a")
    }

    fn id(facts: Vec<Fact>, lemnos: &Lemnos) -> Identity {
        identity(&node(facts), lemnos, LABEL, &link(), "x").unwrap()
    }

    #[test]
    fn family_and_serial_make_the_key_and_the_node_name_and_revision_come_from_the_document() {
        let found = id(
            vec![
                fact("family", "test-family"),
                fact("serial", "0001"),
                fact("model", "test-model"),
                fact("version.os", "1.2.3"),
                fact("offers", "slots"),
            ],
            &Lemnos::Answering,
        );
        assert_eq!(found.key, DeviceKey::new("test-family", "0001"));
        assert!(has_board_identity(&found));
        assert_eq!(found.model, "test-model");
        assert_eq!(found.versions.get("os").map(String::as_str), Some("1.2.3"));
        assert_eq!(found.name.as_deref(), Some("board"));
        assert_eq!(
            found.attributes.get("revision").map(String::as_str),
            Some("3")
        );
        assert_eq!(
            found.attributes.get(LEMNOS).map(String::as_str),
            Some("answering")
        );
    }

    #[test]
    fn a_node_without_hardware_facts_is_a_plain_node_keyed_by_its_fingerprint() {
        let plain = id(Vec::new(), &Lemnos::Absent);
        assert_eq!(plain.key, DeviceKey::new(LABEL, "ab:cd"));
        assert!(!has_board_identity(&plain));
        assert_eq!(plain.state, DeviceState::Running);
        assert_eq!(
            plain.attributes.get(LEMNOS).map(String::as_str),
            Some("absent")
        );
    }

    #[test]
    fn a_node_with_only_a_serial_falls_back_to_its_fingerprint() {
        let found = id(vec![fact("serial", "1")], &Lemnos::Absent);
        assert_eq!(found.key, DeviceKey::new(LABEL, "ab:cd"));
    }

    #[test]
    fn recovery_and_flasher_are_recovery_and_an_unknown_state_is_refused() {
        assert_eq!(
            id(vec![fact("state", "recovery")], &Lemnos::Absent).state,
            DeviceState::Recovery
        );
        assert_eq!(
            id(vec![fact("state", "flasher")], &Lemnos::Absent).state,
            DeviceState::Recovery
        );
        let unknown = identity(
            &node(vec![fact("state", "boot-rom")]),
            &Lemnos::Absent,
            LABEL,
            &link(),
            "x",
        );
        assert!(matches!(unknown, Err(DriverError::Incompatible(_))));
    }

    #[test]
    fn orion_services_are_granted_to_every_node_and_lemnos_contracts_only_when_offered() {
        let plain = capabilities(&id(Vec::new(), &Lemnos::Absent));
        for kind in ORION_OWN {
            assert!(plain.grants(kind), "{kind:?} for a plain node");
        }
        assert!(!plain.grants(CapabilityKind::Slots));
        assert!(!plain.grants(CapabilityKind::Update));
        assert!(!plain.grants(CapabilityKind::HardwareControl));

        let board = capabilities(&id(
            vec![fact(
                "offers",
                "slots, update, hardware, reboot, not-a-kind",
            )],
            &Lemnos::Answering,
        ));
        assert!(board.grants(CapabilityKind::Slots));
        assert!(board.grants(CapabilityKind::Update));
        assert!(board.grants(CapabilityKind::HardwareControl));
        assert!(!board.grants(CapabilityKind::Recover));
    }

    #[test]
    fn every_kind_has_its_name() {
        for kind in CapabilityKind::ALL {
            let name = serde_json::to_value(kind).unwrap();
            assert_eq!(kind_named(name.as_str().unwrap()), Some(kind));
        }
    }
}
