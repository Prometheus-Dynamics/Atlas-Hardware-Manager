//! The catalog's data types. The build script and the library share this
//! file, so the generated catalog and the reader can't drift apart.

use serde::{Deserialize, Serialize};

/// What a USB id shows a board doing. A closed set: a file naming any other
/// state is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsbState {
    /// A boot ROM or loader's download mode (Atlas can flash from it).
    Download,
    /// A mass-storage gadget a boot ROM shows once its USB boot finished.
    Storage,
    /// The USB gadget of a board that is running.
    Gadget,
}

/// One USB id a board shows in one state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsbId {
    pub state: UsbState,
    pub vid: u16,
    pub pid: u16,
    pub name: String,
}

/// One board as the app sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    /// The model's id, as the board's description names it.
    pub model: String,
    pub display_name: String,
    /// Its USB ids, in the order its app data lists them.
    pub usb: Vec<UsbId>,
    /// A lowercase prefix of the hostname a running board of this model gets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname_prefix: Option<String>,
    /// A picture, as a path relative to the `devices/` root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The case model, as a path relative to the `devices/` root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
}

/// The whole catalog: every board found under `devices/`, sorted by model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: String,
    pub boards: Vec<Board>,
}

/// A board a USB id may belong to. One id can match several boards (the same
/// boot ROM id is shared by more than one model), so a match is a candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbMatch {
    pub model: String,
    pub display_name: String,
    pub state: UsbState,
}
