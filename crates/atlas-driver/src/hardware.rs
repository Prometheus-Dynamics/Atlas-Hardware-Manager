//! Commands to a board's devices: set a control, undo Atlas's writes, hand a
//! fan back to the board. What the devices read comes with the status
//! ([`HardwareSnapshot`](crate::HardwareSnapshot)), and live, at device rate,
//! from a board that streams it ([`HardwareFrame`]).

use std::collections::BTreeMap;
use std::sync::Arc;

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
    /// Switches a power switch (a USB port's power) on or off.
    Power { on: bool },
    /// A step of the device's calibration; `routine` names what `start` runs
    /// (Lemnos: `accel-six`, `gyro-hold`, `mag-rotate`).
    Calibrate {
        step: CalibrationStep,
        #[serde(default)]
        routine: Option<String>,
    },
}

/// What a calibration command does (Lemnos's `calibration.<step>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CalibrationStep {
    /// Starts a routine.
    Start,
    /// Ends the running routine; a finished result is kept as a candidate.
    Stop,
    /// Makes the candidate the device's calibration (the board keeps it).
    Apply,
    /// Drops the candidate.
    Discard,
    /// Back to the factory calibration.
    Reset,
}

impl CalibrationStep {
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Apply => "apply",
            Self::Discard => "discard",
            Self::Reset => "reset",
        }
    }
}

/// One part of a device's calibration (`accel`, `gyro`, `mag`); ratios 0..1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPart {
    pub samples: u64,
    pub confidence: f64,
    pub coverage: f64,
    pub residual: f64,
    /// Calibrated values are in use for it.
    pub active: bool,
}

/// A device's calibration state, read now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CalibrationStatus {
    pub revision: u64,
    /// The routine running, if any.
    pub running: Option<String>,
    /// Its progress, 0..1.
    pub progress: f64,
    /// A finished result waits to be applied or discarded.
    pub candidate: bool,
    /// The last routine failed.
    pub failed: bool,
    /// By part; a part the device doesn't have isn't listed.
    pub parts: BTreeMap<String, CalibrationPart>,
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
            Self::Power { on } => format!("switch it {}", if *on { "on" } else { "off" }),
            Self::Calibrate {
                step: CalibrationStep::Start,
                routine,
            } => format!("start calibrating ({})", routine.as_deref().unwrap_or("?")),
            Self::Calibrate { step, .. } => format!("calibration: {}", step.name()),
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

    /// Runs the board device `hardware`'s action `name` with `args`, and
    /// answers its output: the actions that aren't a control (Lemnos's
    /// `looks.*` and `light.brightness` on a light, `power.reset` on a power
    /// switch). Numbers go as numbers, text as text.
    async fn device_action(
        &self,
        _device: &Identity,
        hardware: &str,
        name: &str,
        _args: BTreeMap<String, serde_json::Value>,
    ) -> Result<BTreeMap<String, serde_json::Value>, DriverError> {
        Err(DriverError::Unsupported(format!(
            "{hardware}: {name} isn't reachable on this board"
        )))
    }

    /// The calibration state of the board device `hardware`.
    async fn calibration_status(
        &self,
        _device: &Identity,
        hardware: &str,
    ) -> Result<CalibrationStatus, DriverError> {
        Err(DriverError::Unsupported(format!(
            "{hardware}: calibration isn't reachable on this board"
        )))
    }
}

/// One channel of a live device: its name and unit (empty: none).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveChannel {
    pub name: String,
    #[serde(default)]
    pub unit: String,
}

/// A device of a live stream, as the board describes it; `missing` when the
/// board has no such device.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LiveDevice {
    pub id: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub period_ms: u32,
    #[serde(default)]
    pub channels: Vec<LiveChannel>,
    #[serde(default)]
    pub missing: bool,
    /// Why the board won't stream it (a fan or a light has no readings).
    #[serde(default)]
    pub refused: Option<String>,
}

/// What a live hardware stream delivers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum HardwareFrame {
    /// The devices asked for, first.
    Devices { devices: Vec<LiveDevice> },
    /// Readings of one device since the last batch: each row is the board's
    /// monotonic time (µs) and a value per channel, in `Devices` order
    /// (`None`: not read).
    Samples {
        device: String,
        samples: Vec<(u64, Vec<Option<f64>>)>,
    },
    /// The stream ended: why.
    Gone { reason: String },
}

/// Where a live hardware stream's frames go.
pub type FrameSink = Arc<dyn Fn(HardwareFrame) + Send + Sync>;
