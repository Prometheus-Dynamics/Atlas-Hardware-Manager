//! Who a node is, from its hello document and what its Lemnos says.
//!
//! The hello document is Orion's signed answer. Its fields are `name`, `revision`, the key
//! fingerprint and `state` (Orion's own, `running`, `recovery` or `flasher`). Hardware facts
//! are published by Lemnos as topics `hello.<fact>`: `family`, `serial`, `model`,
//! `version.<name>` and `offers`. A fact that cannot be known is absent, never guessed.
//!
//! The device key is a board key (`family:serial`) when the board publishes both, else a node
//! key from the node's Orion key fingerprint. A node without Lemnos is not refused; only jobs
//! that need a board identity refuse a node key (see [`atlas_types::DeviceKey::board_identity`]).

use std::collections::BTreeMap;

use atlas_driver::DriverError;
use atlas_types::{
    CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Identity, LemnosState, LinkId,
};
use orion::node::Node;

/// The `UTF-8 text` field type number in Orion's topic declarations.
const TEXT: u8 = 4;
/// The attribute that carries the offers, as the node published them.
pub const OFFERS: &str = "offers";

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
/// If the node publishes a state Atlas does not know, or a family or serial that cannot make a
/// key. A node with no state published is running: Orion publishes `recovery` and a flasher
/// publishes `flasher`.
pub fn identity(
    node: &Node,
    lemnos: &LemnosState,
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

    let key = match (fact("family"), fact("serial")) {
        (Some(family), Some(serial)) => DeviceKey::board(family, serial).map_err(|error| {
            DriverError::Incompatible(format!("the node's hardware identity: {error}"))
        })?,
        _ => DeviceKey::node(node.fingerprint.clone())
            .map_err(|error| DriverError::Incompatible(format!("the node's identity: {error}")))?,
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
    attributes.insert("cluster".into(), node.cluster.clone());
    attributes.insert("revision".into(), node.revision.to_string());

    Ok(Identity {
        key,
        model: fact("model").unwrap_or_default(),
        state,
        lemnos: lemnos.clone(),
        versions,
        name: Some(node.name.clone()),
        link: link.clone(),
        address: address.to_owned(),
        attributes,
    })
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

    fn id(facts: Vec<Fact>, lemnos: &LemnosState) -> Identity {
        identity(&node(facts), lemnos, &link(), "x").unwrap()
    }

    #[test]
    fn family_and_serial_make_a_board_key_and_the_node_name_and_revision_come_from_the_document() {
        let found = id(
            vec![
                fact("family", "test-family"),
                fact("serial", "0001"),
                fact("model", "test-model"),
                fact("version.os", "1.2.3"),
                fact("offers", "slots"),
            ],
            &LemnosState::Answering,
        );
        assert_eq!(found.key, DeviceKey::from_static("test-family", "0001"));
        assert!(found.key.is_board());
        assert_eq!(found.model, "test-model");
        assert_eq!(found.versions.get("os").map(String::as_str), Some("1.2.3"));
        assert_eq!(found.name.as_deref(), Some("board"));
        assert_eq!(
            found.attributes.get("revision").map(String::as_str),
            Some("3")
        );
        assert_eq!(found.lemnos, LemnosState::Answering);
    }

    #[test]
    fn a_node_without_hardware_facts_is_keyed_by_its_fingerprint_and_is_not_a_board() {
        let plain = id(Vec::new(), &LemnosState::Absent);
        assert_eq!(plain.key, DeviceKey::node("ab:cd").unwrap());
        assert!(!plain.key.is_board());
        assert_eq!(plain.state, DeviceState::Running);
        assert_eq!(plain.lemnos, LemnosState::Absent);
    }

    #[test]
    fn a_node_with_only_a_serial_is_keyed_by_its_fingerprint() {
        let found = id(vec![fact("serial", "1")], &LemnosState::Absent);
        assert_eq!(found.key, DeviceKey::node("ab:cd").unwrap());
    }

    #[test]
    fn a_family_that_cannot_make_a_key_is_refused_rather_than_guessed() {
        let refused = identity(
            &node(vec![fact("family", "bad:family"), fact("serial", "1")]),
            &LemnosState::Absent,
            &link(),
            "x",
        );
        assert!(matches!(refused, Err(DriverError::Incompatible(_))));
    }

    #[test]
    fn every_lemnos_state_reaches_the_identity_unchanged() {
        let states = [
            LemnosState::Absent,
            LemnosState::Answering,
            LemnosState::NotAnswering {
                reason: "-4: not answering".into(),
            },
            LemnosState::RevisionMismatch {
                reason: "built from different Lemnos revisions".into(),
            },
        ];
        for state in states {
            let found = id(
                vec![fact("family", "test-family"), fact("serial", "1")],
                &state,
            );
            assert_eq!(found.lemnos, state);
        }
    }

    #[test]
    fn recovery_and_flasher_are_recovery_and_an_unknown_state_is_refused() {
        assert_eq!(
            id(vec![fact("state", "recovery")], &LemnosState::Absent).state,
            DeviceState::Recovery
        );
        assert_eq!(
            id(vec![fact("state", "flasher")], &LemnosState::Absent).state,
            DeviceState::Recovery
        );
        let unknown = identity(
            &node(vec![fact("state", "boot-rom")]),
            &LemnosState::Absent,
            &link(),
            "x",
        );
        assert!(matches!(unknown, Err(DriverError::Incompatible(_))));
    }

    #[test]
    fn orion_services_are_granted_to_every_node_and_lemnos_contracts_only_when_offered() {
        let plain = capabilities(&id(Vec::new(), &LemnosState::Absent));
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
            &LemnosState::Answering,
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
