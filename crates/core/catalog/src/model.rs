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

/// One USB id a board shows in one state. With no `pid`, the id is vendor-wide:
/// it matches any product of that vendor in that state. An exact `vid:pid`
/// match always ranks above a vendor-wide one (see [`crate::Catalog::recognise_usb`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsbId {
    pub state: UsbState,
    pub vid: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u16>,
    pub name: String,
}

/// A named port on a board: what kind of port it is and what to call it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// The port's name on the board, `[a-z0-9-]`, unique within the board.
    pub name: String,
    /// What kind of port it is, `[a-z0-9-]` (for example `usb-c`, `ethernet`, `csi`).
    pub kind: String,
    /// What the app shows for it.
    pub label: String,
}

/// Which way a case sits on the board. Each field is optional. The values are
/// the case model's own frame, as the board's case description measured them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Orientation {
    /// The rotation taking case vectors to IMU vectors, as a unit quaternion
    /// `[w, x, y, z]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_in_imu: Option<[f64; 4]>,
    /// The angle of LED 0 in the case's frame, from +x toward +y, in degrees
    /// `0..360`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_led0_deg: Option<f64>,
    /// Which way the rest of the ring runs from LED 0: `1` or `-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring_direction: Option<i8>,
}

/// What a variant changes. Each field, when present, replaces the board's own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pictures: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<Port>>,
}

/// One option of a board: a second variant of the same board, such as a lens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    /// The variant's id within the board, `[a-z0-9-]`.
    pub key: String,
    pub display_name: String,
    /// `false` when the variant's facts are not yet measured or confirmed.
    pub confirmed: bool,
    #[serde(default, skip_serializing_if = "Overrides::is_empty")]
    pub overrides: Overrides,
}

impl Overrides {
    fn is_empty(&self) -> bool {
        self.pictures.is_none() && self.case.is_none() && self.ports.is_none()
    }
}

/// One board as the app sees it. The `Option` and `Vec` fields are empty for a
/// board whose app data does not give them, and are left out of the JSON then.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// How a device with no USB id of its own is recognised: the driver
    /// family that finds it, free text (for example `robot`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_by: Option<String>,
    /// The named ports, sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Every picture, as paths relative to the `devices/` root, in the order
    /// the app data lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
    /// The first of [`Board::pictures`], for readers that show one picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// The case model, as a path relative to the `devices/` root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    /// How the case sits on the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    /// Options of this board, sorted by key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
}

/// The whole catalog: every board found under `devices/`, sorted by model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// `true` when the board lists this exact `vid:pid`; `false` when it
    /// matched on a vendor-wide id alone.
    pub exact: bool,
}
