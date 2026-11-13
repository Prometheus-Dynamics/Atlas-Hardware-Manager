use serde::{Deserialize, Serialize};

/// A named set of devices that travel together. A device joins a robot by
/// its name ([`crate::DeviceRecord::robot`]); robots join a team by its name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Robot {
    pub name: String,
    /// The team the robot belongs to, as the user names it.
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}
