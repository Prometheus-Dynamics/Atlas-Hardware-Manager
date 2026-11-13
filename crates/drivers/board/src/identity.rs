//! Who a node is, from its hello document and what its Lemnos says.
//!
//! The hello document is Orion's signed answer. Its fields are `name`, `revision`, the key
//! fingerprint and the facts programs on the node published as `hello.<fact>` topics. Atlas reads
//! the facts Lemnos P1c defines: `family`, `serial`, `model`, `version.slot`, `version.product`,
//! `version.os`, `version.lemnos` and `offers`, plus Orion's own `state`. A fact the node does not
//! give is not reported. A fact that is malformed is refused: its reason is kept as the attribute
//! `refused.<fact>`, and the fact is not reported either.
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
/// The attribute that carries the node's offers: its contract ids, sorted and space-separated.
pub const OFFERS: &str = "offers";
/// The most bytes any fact's text may carry (the bridge's own limit).
const MAX_TEXT: usize = 256;
/// The most bytes a family, product or model name may carry (Lemnos docs/identity.md).
const MAX_NAME: usize = 64;
/// The fewest and most lowercase hex digits a serial may have.
const SERIAL_DIGITS: std::ops::RangeInclusive<usize> = 8..=32;

/// What every Orion node provides, Lemnos or not: its files, shell, logs, topics and actions.
const ORION_OWN: [CapabilityKind; 5] = [
    CapabilityKind::Files,
    CapabilityKind::Shell,
    CapabilityKind::Logs,
    CapabilityKind::Telemetry,
    CapabilityKind::Actions,
];

/// A check on a fact's text. `Err` names the rule the text breaks.
type Check = fn(&str) -> Result<(), String>;

/// The version facts: the hello fact, the version it names, and how its text is checked.
const VERSIONS: [(&str, &str, Check); 4] = [
    ("version.slot", "slot", text),
    ("version.product", "product", name_form),
    ("version.os", "os", text),
    ("version.lemnos", "lemnos", text),
];

/// The identity of `node`, as found at `address` on `link`, with what its Lemnos is doing.
///
/// A malformed fact does not fail the identity: it is refused, its reason goes in the attributes,
/// and the identity is read without it.
///
/// # Errors
///
/// If the node publishes a state Atlas does not know, or its own key fingerprint is blank. A node
/// with no state published is running: Orion publishes `recovery` and a flasher publishes
/// `flasher`.
pub fn identity(
    node: &Node,
    lemnos: &LemnosState,
    link: &LinkId,
    address: &str,
) -> Result<Identity, DriverError> {
    let mut read = Reading::new(node);
    let state = match read.text("state", accept).as_deref() {
        None | Some("running") => DeviceState::Running,
        Some("recovery" | "flasher") => DeviceState::Recovery,
        Some(other) => {
            return Err(DriverError::Incompatible(format!(
                "the node reports state {other:?}, which Atlas does not know"
            )));
        }
    };

    let family = read.text("family", name_form);
    let serial = read.text("serial", serial_form);
    let board = match (family, serial) {
        (Some(family), Some(serial)) => match DeviceKey::board(family, serial) {
            Ok(key) => Some(key),
            Err(error) => {
                read.refuse("serial", error.to_string());
                None
            }
        },
        _ => None,
    };
    let key = match board {
        Some(key) => key,
        None => DeviceKey::node(node.fingerprint.clone())
            .map_err(|error| DriverError::Incompatible(format!("the node's identity: {error}")))?,
    };

    let model = read.text("model", text);
    let mut versions = BTreeMap::new();
    for (fact, version, check) in VERSIONS {
        if let Some(value) = read.text(fact, check) {
            versions.insert(version.to_owned(), value);
        }
    }
    let offers = read
        .text(OFFERS, offers_form)
        .map(|value| contracts(&value).join(" "));

    let mut attributes = BTreeMap::new();
    if let Some(offers) = offers {
        attributes.insert(OFFERS.to_owned(), offers);
    }
    attributes.insert("cluster".into(), node.cluster.clone());
    attributes.insert("revision".into(), node.revision.to_string());
    for (fact, reason) in read.refused {
        attributes.insert(format!("refused.{fact}"), reason);
    }

    Ok(Identity {
        key,
        model,
        state,
        lemnos: lemnos.clone(),
        versions,
        name: Some(node.name.clone()),
        link: link.clone(),
        address: address.to_owned(),
        attributes,
    })
}

/// The facts of one node as they are read. Each is read once. A refused fact is kept with its
/// reason for the identity's attributes.
struct Reading<'a> {
    node: &'a Node,
    refused: BTreeMap<&'static str, String>,
}

impl<'a> Reading<'a> {
    fn new(node: &'a Node) -> Self {
        Self {
            node,
            refused: BTreeMap::new(),
        }
    }

    /// The text of fact `name`, checked by `check`. Absent and empty are not reported. A fact that
    /// is not UTF-8 text, or fails its check, is refused with its reason and not reported.
    fn text(&mut self, name: &'static str, check: Check) -> Option<String> {
        let node = self.node;
        let fact = node.facts.iter().find(|fact| fact.name == name)?;
        let value = match (fact.ty == TEXT, String::from_utf8(fact.value.clone())) {
            (true, Ok(value)) => value,
            _ => {
                self.refuse(name, "it is not UTF-8 text".into());
                return None;
            }
        };
        if value.is_empty() {
            return None;
        }
        match check(&value) {
            Ok(()) => Some(value),
            Err(reason) => {
                self.refuse(name, reason);
                None
            }
        }
    }

    fn refuse(&mut self, name: &'static str, reason: String) {
        self.refused.insert(name, reason);
    }
}

/// Accepts any text. For a fact whose text is only matched, not checked.
fn accept(_: &str) -> Result<(), String> {
    Ok(())
}

/// Text of at most [`MAX_TEXT`] bytes with no control character, so no newline either.
fn text(value: &str) -> Result<(), String> {
    if value.len() > MAX_TEXT {
        return Err(format!("it is {} bytes, more than {MAX_TEXT}", value.len()));
    }
    if value.chars().any(char::is_control) {
        return Err("it has a control character, such as a newline".into());
    }
    Ok(())
}

/// A name: `[a-z0-9-]`, at most [`MAX_NAME`] bytes.
fn name_form(value: &str) -> Result<(), String> {
    if value.len() > MAX_NAME {
        return Err(format!("it is {} bytes, more than {MAX_NAME}", value.len()));
    }
    let plain = value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !plain {
        return Err("it is not a name of lowercase letters, digits and '-'".into());
    }
    Ok(())
}

/// A serial: lowercase hex, 8 to 32 digits, leading zeros kept.
fn serial_form(value: &str) -> Result<(), String> {
    if !SERIAL_DIGITS.contains(&value.len()) {
        return Err(format!(
            "it is {} digits, not {} to {}",
            value.len(),
            SERIAL_DIGITS.start(),
            SERIAL_DIGITS.end()
        ));
    }
    let hex = value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !hex {
        return Err("it is not lowercase hex digits".into());
    }
    Ok(())
}

/// Offers: contract ids (`name/version`, such as `system.slots/1`), separated by single spaces.
fn offers_form(value: &str) -> Result<(), String> {
    text(value)?;
    for contract in value.split(' ') {
        if !is_contract_id(contract) {
            return Err(format!("{contract:?} is not a contract id, name/version"));
        }
    }
    Ok(())
}

/// Whether `contract` is a Lemnos contract id: lowercase letters, digits, `.` and `-`, then `/`
/// and a version number.
fn is_contract_id(contract: &str) -> bool {
    let Some((name, version)) = contract.rsplit_once('/') else {
        return false;
    };
    let name_ok = !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        });
    let version_ok = !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit());
    name_ok && version_ok
}

/// The distinct contract ids of an offers text, sorted. Checked by [`offers_form`] first.
fn contracts(value: &str) -> Vec<&str> {
    let mut found: Vec<&str> = value.split(' ').collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// The capabilities an identified node grants.
///
/// Orion's own services are granted for every node. A Lemnos contract the node offers grants the
/// kinds [`contract_kinds`] names. Its other actions are granted with Orion's own.
pub fn capabilities(identity: &Identity) -> CapabilitySet {
    let offered = identity
        .attributes
        .get(OFFERS)
        .map(String::as_str)
        .unwrap_or_default();
    CapabilitySet::from_kinds(
        ORION_OWN.into_iter().chain(
            contracts(offered)
                .into_iter()
                .flat_map(|contract| contract_kinds(contract).iter().copied()),
        ),
    )
}

/// The capability kinds a Lemnos contract grants, by the id Lemnos names it with. A contract Atlas
/// does not know grants nothing. `system.identity/1` grants nothing: `info` is every device's.
fn contract_kinds(contract: &str) -> &'static [CapabilityKind] {
    match contract {
        "system.slots/1" => &[CapabilityKind::Slots, CapabilityKind::Update],
        _ => &[],
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

    const SERIAL: &str = "0123456789abcdef";

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

    /// Every fact Lemnos P1c defines, with a valid value each.
    fn every_fact() -> Vec<Fact> {
        vec![
            fact("family", "test-family"),
            fact("serial", SERIAL),
            fact("model", "test-model r2"),
            fact("version.slot", "slot-1"),
            fact("version.product", "test-product"),
            fact("version.os", "1.2.3"),
            fact("version.lemnos", "0.4.0"),
            fact("offers", "system.reboot/1 system.slots/1"),
        ]
    }

    #[test]
    fn every_fact_lemnos_defines_is_read_into_its_field() {
        let found = id(every_fact(), &LemnosState::Answering);
        assert_eq!(found.key, DeviceKey::board("test-family", SERIAL).unwrap());
        assert!(found.key.is_board());
        assert_eq!(found.model.as_deref(), Some("test-model r2"));
        assert_eq!(found.state, DeviceState::Running);
        assert_eq!(found.name.as_deref(), Some("board"));
        assert_eq!(found.lemnos, LemnosState::Answering);
        let versions: Vec<(&str, &str)> = found
            .versions
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            versions,
            [
                ("lemnos", "0.4.0"),
                ("os", "1.2.3"),
                ("product", "test-product"),
                ("slot", "slot-1"),
            ]
        );
        assert_eq!(
            found.attributes.get(OFFERS).map(String::as_str),
            Some("system.reboot/1 system.slots/1")
        );
        assert_eq!(
            found.attributes.get("revision").map(String::as_str),
            Some("3")
        );
        assert!(
            !found
                .attributes
                .keys()
                .any(|key| key.starts_with("refused"))
        );
    }

    #[test]
    fn an_absent_fact_is_not_reported_not_an_empty_one() {
        let found = id(
            vec![fact("family", "test-family"), fact("serial", SERIAL)],
            &LemnosState::Absent,
        );
        assert_eq!(found.model, None);
        assert!(found.versions.is_empty());
        assert!(!found.attributes.contains_key(OFFERS));
        assert!(
            !found
                .attributes
                .keys()
                .any(|key| key.starts_with("refused"))
        );
        // An empty text is the same as absent.
        let empty = id(
            vec![
                fact("family", "test-family"),
                fact("serial", SERIAL),
                fact("version.os", ""),
            ],
            &LemnosState::Absent,
        );
        assert!(!empty.versions.contains_key("os"));
    }

    #[test]
    fn a_family_and_serial_make_a_board_key_and_a_node_without_them_is_keyed_by_its_fingerprint() {
        let plain = id(Vec::new(), &LemnosState::Absent);
        assert_eq!(plain.key, DeviceKey::node("ab:cd").unwrap());
        assert!(!plain.key.is_board());
        assert_eq!(plain.state, DeviceState::Running);
        assert_eq!(plain.lemnos, LemnosState::Absent);
        let serial_only = id(vec![fact("serial", SERIAL)], &LemnosState::Absent);
        assert_eq!(serial_only.key, DeviceKey::node("ab:cd").unwrap());
    }

    #[test]
    fn a_malformed_serial_is_refused_with_its_reason_and_never_makes_a_key() {
        let bad = [
            "0123456789ABCDEF",                  // not lowercase
            "0123",                              // too short
            "0123456789abcdef0123456789abcdef0", // 33 digits
            "0123456789abcdeg",                  // not hex
            "0123456789abcdef\n",                // a trailing newline
        ];
        for serial in bad {
            let found = id(
                vec![fact("family", "test-family"), fact("serial", serial)],
                &LemnosState::Absent,
            );
            assert_eq!(found.key, DeviceKey::node("ab:cd").unwrap(), "{serial:?}");
            assert!(
                found.attributes.contains_key("refused.serial"),
                "{serial:?}"
            );
        }
        // The edges of the length are fine.
        for serial in ["01234567", "0123456789abcdef0123456789abcdef"] {
            let found = id(
                vec![fact("family", "test-family"), fact("serial", serial)],
                &LemnosState::Absent,
            );
            assert!(found.key.is_board(), "{serial}");
        }
    }

    #[test]
    fn a_malformed_family_is_refused_and_a_serial_alone_makes_no_board_key() {
        for family in ["bad:family", "Sample", "x y", "@node", ""] {
            let found = id(
                vec![fact("family", family), fact("serial", SERIAL)],
                &LemnosState::Absent,
            );
            assert!(!found.key.is_board(), "{family:?}");
        }
        let refused = id(
            vec![fact("family", "Sample"), fact("serial", SERIAL)],
            &LemnosState::Absent,
        );
        assert!(refused.attributes.contains_key("refused.family"));
    }

    #[test]
    fn a_text_fact_with_a_control_character_or_too_long_is_refused_and_not_reported() {
        let found = id(
            vec![
                fact("family", "test-family"),
                fact("serial", SERIAL),
                fact("model", "test\nboard"),
                fact("version.os", &"9".repeat(257)),
            ],
            &LemnosState::Absent,
        );
        assert_eq!(found.model, None);
        assert!(!found.versions.contains_key("os"));
        assert!(found.attributes.contains_key("refused.model"));
        assert!(found.attributes.contains_key("refused.version.os"));
    }

    #[test]
    fn a_fact_that_is_not_text_is_refused() {
        let mut bytes = fact("model", "test-model");
        bytes.ty = 9;
        let found = id(
            vec![fact("family", "test-family"), fact("serial", SERIAL), bytes],
            &LemnosState::Absent,
        );
        assert_eq!(found.model, None);
        assert!(found.attributes.contains_key("refused.model"));
    }

    #[test]
    fn hello_name_and_revision_are_not_facts_and_are_not_read_as_any() {
        let found = id(
            vec![
                fact("family", "test-family"),
                fact("serial", SERIAL),
                fact("name", "Alpha"),
                fact("revision", "r2"),
            ],
            &LemnosState::Absent,
        );
        assert!(!found.attributes.contains_key("name"));
        assert!(!found.attributes.contains_key("revision-fact"));
        assert_eq!(found.name.as_deref(), Some("board"));
        assert!(found.versions.is_empty());
    }

    #[test]
    fn offers_are_contract_ids_sorted_and_grant_only_the_kinds_their_contracts_do() {
        let found = id(
            vec![fact(
                "offers",
                "system.slots/1 system.reboot/1 system.slots/1 system.identity/1",
            )],
            &LemnosState::Answering,
        );
        assert_eq!(
            found.attributes.get(OFFERS).map(String::as_str),
            Some("system.identity/1 system.reboot/1 system.slots/1")
        );
        let grants = capabilities(&found);
        assert!(grants.grants(CapabilityKind::Slots));
        assert!(grants.grants(CapabilityKind::Update));
        assert!(!grants.grants(CapabilityKind::Recover));
        assert!(!grants.grants(CapabilityKind::HardwareControl));
        assert!(!grants.grants(CapabilityKind::Status));
    }

    #[test]
    fn malformed_offers_are_refused_and_grant_no_lemnos_kind() {
        for offers in [
            "system.slots/1  system.reboot/1",
            "slots",
            "system.slots/x",
            "System.Slots/1",
            "system.slots/1,system.reboot/1",
            "system.slots/1\n",
        ] {
            let found = id(vec![fact("offers", offers)], &LemnosState::Answering);
            assert!(
                found.attributes.contains_key("refused.offers"),
                "{offers:?}"
            );
            assert!(!found.attributes.contains_key(OFFERS), "{offers:?}");
            assert!(
                !capabilities(&found).grants(CapabilityKind::Slots),
                "{offers:?}"
            );
        }
    }

    #[test]
    fn an_identity_that_cannot_be_read_with_offers_present_is_lemnos_there_and_not_reported() {
        // The identity device could not be read: none of its facts, but the offers are.
        let found = id(
            vec![fact("offers", "system.slots/1 system.reboot/1")],
            &LemnosState::Answering,
        );
        assert_eq!(found.key, DeviceKey::node("ab:cd").unwrap());
        assert_eq!(found.lemnos, LemnosState::Answering);
        assert!(found.versions.is_empty());
        assert_eq!(found.model, None);
        assert!(capabilities(&found).grants(CapabilityKind::Slots));
        // A node with no Lemnos at all is the same key with Absent.
        let plain = id(Vec::new(), &LemnosState::Absent);
        assert_eq!(plain.key, found.key);
        assert_ne!(plain.lemnos, found.lemnos);
    }

    #[test]
    fn orion_services_are_granted_to_every_node_and_lemnos_contracts_only_when_offered() {
        let plain = capabilities(&id(Vec::new(), &LemnosState::Absent));
        for kind in ORION_OWN {
            assert!(plain.grants(kind), "{kind:?} for a plain node");
        }
        assert!(!plain.grants(CapabilityKind::Slots));
        assert!(!plain.grants(CapabilityKind::Update));
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
            let found = id(every_fact(), &state);
            assert_eq!(found.lemnos, state);
        }
    }

    #[test]
    fn every_kind_has_its_name() {
        for kind in CapabilityKind::ALL {
            let name = serde_json::to_value(kind).unwrap();
            assert_eq!(kind_named(name.as_str().unwrap()), Some(kind));
        }
    }
}
