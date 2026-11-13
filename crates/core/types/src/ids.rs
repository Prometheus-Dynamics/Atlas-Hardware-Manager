use std::fmt;

use serde::{Deserialize, Serialize};

/// A device family, such as `sample-soc`. One driver owns one family.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Family(String);

impl Family {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A hardware serial number as reported by the device.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Serial(String);

impl Serial {
    pub fn new(serial: impl Into<String>) -> Self {
        Self(serial.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Serial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The stable key for a device. Never an address or a port, which change as a device moves
/// between links.
///
/// A key has one of two kinds, and its kind says what the device can be asked to do:
///
/// - [`DeviceKey::Board`]: a board that publishes its hardware facts, keyed `family:serial`.
///   Only this kind has a board identity (see [`DeviceKey::board_identity`]), which flashing,
///   slot installs and matching a board description need.
/// - [`DeviceKey::Node`]: an Orion node that publishes no hardware facts (a PC, someone else's
///   board, a controller before our package), keyed by its Orion key fingerprint and written
///   `@<fingerprint>`. The `@` is not a character a family name may use, so the two forms can
///   never be confused: a family name is `[a-z0-9-]` in a board file, and the parser refuses a
///   family that starts with `@` or contains `:`.
///
/// The text form ([`fmt::Display`]) round-trips through [`DeviceKey::parse`] for every key the
/// constructors accept. An old `family:serial` string parses as a board key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKey {
    /// A board's own identity: its family and its serial as the board reports them.
    Board { family: Family, serial: Serial },
    /// An Orion node's key fingerprint, for a node without hardware facts.
    Node { fingerprint: String },
}

/// The prefix of a node key's text form. Not usable in a family name.
const NODE_PREFIX: char = '@';

/// Why a key's parts cannot make a key. The text form could not carry them back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyError {
    /// What was refused, in words.
    pub reason: String,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for KeyError {}

impl DeviceKey {
    /// A board key from its family and serial, as the board itself reports them.
    ///
    /// # Errors
    ///
    /// If the family is blank, contains `:`, or starts with `@`, or if the serial is blank.
    /// Those are the parts the text form cannot carry back.
    pub fn board(family: impl Into<String>, serial: impl Into<String>) -> Result<Self, KeyError> {
        let (family, serial) = (family.into(), serial.into());
        if family.trim().is_empty() || family.contains(':') || family.starts_with(NODE_PREFIX) {
            return Err(KeyError {
                reason: format!("{family:?} is not a family name a key can carry"),
            });
        }
        if serial.trim().is_empty() {
            return Err(KeyError {
                reason: format!("the serial of a {family} board is blank"),
            });
        }
        Ok(Self::Board {
            family: Family::new(family),
            serial: Serial::new(serial),
        })
    }

    /// A node key from an Orion key fingerprint.
    ///
    /// # Errors
    ///
    /// If the fingerprint is blank.
    pub fn node(fingerprint: impl Into<String>) -> Result<Self, KeyError> {
        let fingerprint = fingerprint.into();
        if fingerprint.trim().is_empty() {
            return Err(KeyError {
                reason: "the node's key fingerprint is blank".into(),
            });
        }
        Ok(Self::Node { fingerprint })
    }

    /// A board key for values that are known to be valid, such as a driver's own family and a
    /// serial it read from the hardware.
    ///
    /// # Panics
    ///
    /// If [`DeviceKey::board`] would refuse the parts. Use `board` for values that come from
    /// outside the program.
    pub fn new(family: impl Into<String>, serial: impl Into<String>) -> Self {
        match Self::board(family, serial) {
            Ok(key) => key,
            Err(error) => panic!("{error}"),
        }
    }

    /// The family of a board key. `None` for a node key.
    pub const fn family(&self) -> Option<&Family> {
        match self {
            Self::Board { family, .. } => Some(family),
            Self::Node { .. } => None,
        }
    }

    /// The serial of a board key. `None` for a node key.
    pub const fn serial(&self) -> Option<&Serial> {
        match self {
            Self::Board { serial, .. } => Some(serial),
            Self::Node { .. } => None,
        }
    }

    /// Whether this key is a board's own identity.
    pub const fn is_board(&self) -> bool {
        matches!(self, Self::Board { .. })
    }

    /// The board identity of this key, for a job that needs one (flashing, version installs,
    /// matching a board description). A node key is refused, and the error names what is
    /// missing.
    ///
    /// # Errors
    ///
    /// [`NoBoardIdentity`] for a node key: the node publishes no family and serial.
    pub fn board_identity(&self) -> Result<(&Family, &Serial), NoBoardIdentity> {
        match self {
            Self::Board { family, serial } => Ok((family, serial)),
            Self::Node { fingerprint } => Err(NoBoardIdentity {
                fingerprint: fingerprint.clone(),
            }),
        }
    }

    /// Parses the text form written by [`fmt::Display`]: `@<fingerprint>` for a node key, else
    /// `family:serial` for a board key.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(fingerprint) = text.strip_prefix(NODE_PREFIX) {
            return Self::node(fingerprint).ok();
        }
        let (family, serial) = text.split_once(':')?;
        Self::board(family, serial).ok()
    }
}

impl fmt::Display for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Board { family, serial } => write!(f, "{family}:{serial}"),
            Self::Node { fingerprint } => write!(f, "{NODE_PREFIX}{fingerprint}"),
        }
    }
}

/// A key that is a node's, asked for a job that needs a board's own identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoBoardIdentity {
    /// The node's Orion key fingerprint: all Atlas knows of it.
    pub fingerprint: String,
}

impl fmt::Display for NoBoardIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the device has no board identity: it publishes no family and serial, so only its Orion key fingerprint {} is known",
            self.fingerprint
        )
    }
}

impl std::error::Error for NoBoardIdentity {}

/// The text form is the key's serde form. A file written before keys had kinds stored the
/// board key as an object with `family` and `serial`; that still loads, as a board key.
impl Serialize for DeviceKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DeviceKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Text(String),
            Legacy { family: String, serial: String },
        }
        match Stored::deserialize(deserializer)? {
            Stored::Text(text) => Self::parse(&text)
                .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not a device key"))),
            Stored::Legacy { family, serial } => {
                Self::board(family, serial).map_err(|error| serde::de::Error::custom(error.reason))
            }
        }
    }
}

/// Identifies one link for the lifetime of a scan, for example `usbnet:enp0s20u1`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkId(String);

impl LinkId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_key_round_trips_through_its_text_form() {
        let key = DeviceKey::new("sample-soc", "1000abcd");
        assert_eq!(key.to_string(), "sample-soc:1000abcd");
        assert_eq!(DeviceKey::parse("sample-soc:1000abcd"), Some(key));
    }

    #[test]
    fn a_node_key_round_trips_and_is_not_a_board_key() {
        let key = DeviceKey::node("ab:cd:01").unwrap();
        assert_eq!(key.to_string(), "@ab:cd:01");
        assert_eq!(DeviceKey::parse("@ab:cd:01"), Some(key.clone()));
        assert!(!key.is_board());
        assert_eq!(key.family(), None);
        assert_eq!(key.serial(), None);
    }

    #[test]
    fn every_key_the_constructors_accept_parses_back_to_itself() {
        let keys = [
            DeviceKey::new("sample-soc", "1000abcd"),
            DeviceKey::new("sample-soc", "serial:with:colons"),
            DeviceKey::new("sample-soc", " padded "),
            DeviceKey::node("ab:cd").unwrap(),
            DeviceKey::node("@at-sign").unwrap(),
        ];
        for key in keys {
            assert_eq!(
                DeviceKey::parse(&key.to_string()),
                Some(key.clone()),
                "{key}"
            );
        }
    }

    #[test]
    fn parse_refuses_what_is_missing() {
        assert_eq!(DeviceKey::parse("sample-soc"), None);
        assert_eq!(DeviceKey::parse(":abc"), None);
        assert_eq!(DeviceKey::parse("sample-soc: "), None);
        assert_eq!(DeviceKey::parse("@"), None);
        assert_eq!(DeviceKey::parse(""), None);
    }

    #[test]
    fn a_family_that_would_read_back_as_another_kind_is_refused() {
        assert!(DeviceKey::board("@node", "1").is_err());
        assert!(DeviceKey::board("a:b", "1").is_err());
        assert!(DeviceKey::board("", "1").is_err());
        assert!(DeviceKey::board("fam", " ").is_err());
        assert!(DeviceKey::node(" ").is_err());
    }

    #[test]
    fn a_node_key_is_refused_for_a_job_that_needs_a_board_identity() {
        let node = DeviceKey::node("ab:cd").unwrap();
        let refused = node.board_identity().unwrap_err();
        assert_eq!(refused.fingerprint, "ab:cd");
        assert!(refused.to_string().contains("no board identity"));
        assert!(refused.to_string().contains("ab:cd"));

        let board = DeviceKey::new("sample-soc", "1000abcd");
        let (family, serial) = board.board_identity().unwrap();
        assert_eq!(
            (family.as_str(), serial.as_str()),
            ("sample-soc", "1000abcd")
        );
    }

    #[test]
    fn a_board_key_as_an_object_from_before_kinds_still_loads_as_a_board_key() {
        let legacy = r#"{"family":"sample-soc","serial":"1000abcd"}"#;
        let key: DeviceKey = serde_json::from_str(legacy).unwrap();
        assert_eq!(key, DeviceKey::new("sample-soc", "1000abcd"));
    }

    #[test]
    fn the_stored_form_is_the_text_form_and_round_trips() {
        for key in [
            DeviceKey::new("sample-soc", "1000abcd"),
            DeviceKey::node("ab:cd").unwrap(),
        ] {
            let stored = serde_json::to_string(&key).unwrap();
            assert_eq!(stored, format!("\"{key}\""));
            assert_eq!(serde_json::from_str::<DeviceKey>(&stored).unwrap(), key);
        }
        assert!(serde_json::from_str::<DeviceKey>("\"no-colon\"").is_err());
    }
}
