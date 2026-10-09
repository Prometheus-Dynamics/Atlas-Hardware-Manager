//! Commands to a board's devices: set a control, undo Atlas's writes, hand a
//! fan back to the board. What the devices read comes with the status
//! ([`HardwareSnapshot`](crate::HardwareSnapshot)).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{DriverError, Identity};

/// One command to a board device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "command")]
pub enum HardwareCommand {
    /// Writes `value` (in the control's unit) to `control`.
    Set { control: String, value: f64 },
    /// Undoes this computer's writes: to `control`, or to every control.
    Restore {
        #[serde(default)]
        control: Option<String>,
    },
    /// Hands a fan back to the board's own cooling.
    Release,
}

impl HardwareCommand {
    /// What it does, for the activity log: `set duty to 0.5`.
    pub fn describe(&self) -> String {
        match self {
            Self::Set { control, value } => format!("set {control} to {value}"),
            Self::Restore {
                control: Some(control),
            } => format!("restore {control}"),
            Self::Restore { control: None } => "restore its controls".into(),
            Self::Release => "hand it back to the board".into(),
        }
    }
}

/// A board whose devices take commands.
#[async_trait]
pub trait HardwareCapability: Send + Sync {
    /// Runs `command` on the board device `hardware` (its id in the
    /// hardware snapshot). For a set, the answer is the value the device
    /// applied; otherwise `None`.
    async fn control(
        &self,
        device: &Identity,
        hardware: &str,
        command: HardwareCommand,
    ) -> Result<Option<f64>, DriverError>;
}
