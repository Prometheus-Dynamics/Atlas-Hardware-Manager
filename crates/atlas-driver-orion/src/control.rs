//! Commands to the board's devices through Lemnos's Orion bridge: `set`,
//! `restore`, `release`, `power.set` and `calibration.*` on a device's
//! `lemnos.device` resource, answered
//! by request/response actions (the node holds its reply until the bridge
//! has one, so nothing polls).
//!
//! Lemnos names each Orion caller `orion:<requested_by>` and applies the
//! device's write policy to that name; it undoes a caller's writes when the
//! caller is gone, and `restore` undoes only this computer's.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use std::collections::BTreeMap;

use atlas_driver::{
    CalibrationPart, CalibrationStatus, CalibrationStep, DriverError, HardwareCapability,
    HardwareCommand, Identity,
};
use orion_control_plane::{ActionRequest, ActionState, ActionTarget, TypedConfigValue};
use orion_core::NodeId;

use crate::actions::action_id;
use crate::hardware::{DEVICE_TYPE, device_id};
use crate::transport::OrionTransport;

/// Long enough for lemnosd to write and read back; a device that hangs
/// answers `Failed` from the bridge well before.
const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct OrionHardware {
    pub(crate) transport: Arc<dyn OrionTransport>,
    pub(crate) node: NodeId,
}

impl OrionHardware {
    /// The resource of the board device `hardware` on this node.
    async fn resource(&self, hardware: &str) -> Result<ActionTarget, DriverError> {
        self.transport
            .resources()
            .await?
            .into_iter()
            .find(|(record, node)| {
                record.resource_type.as_str() == DEVICE_TYPE
                    && node.as_ref() == Some(&self.node)
                    && device_id(record) == hardware
            })
            .map(|(record, _)| ActionTarget::Resource(record.resource_id))
            .ok_or_else(|| {
                DriverError::Unsupported(format!("Orion doesn't list the board's {hardware}"))
            })
    }
}

#[async_trait]
impl HardwareCapability for OrionHardware {
    async fn control(
        &self,
        _device: &Identity,
        hardware: &str,
        command: HardwareCommand,
    ) -> Result<Option<f64>, DriverError> {
        let name = match &command {
            HardwareCommand::Set { .. } => "set".to_owned(),
            HardwareCommand::Restore { .. } => "restore".to_owned(),
            HardwareCommand::Release => "release".to_owned(),
            HardwareCommand::Power { .. } => "power.set".to_owned(),
            HardwareCommand::Calibrate { step, .. } => format!("calibration.{}", step.name()),
        };
        let mut request =
            ActionRequest::new(action_id(&name), self.resource(hardware).await?, &name);
        match &command {
            HardwareCommand::Set { control, value } => {
                request = request
                    .with_arg("control", TypedConfigValue::String(control.clone()))
                    .with_arg("value", TypedConfigValue::F64(*value));
            }
            HardwareCommand::Restore {
                control: Some(control),
            } => {
                request = request.with_arg("control", TypedConfigValue::String(control.clone()));
            }
            HardwareCommand::Power { on } => {
                request = request.with_arg("on", TypedConfigValue::Bool(*on));
            }
            HardwareCommand::Calibrate {
                step: CalibrationStep::Start,
                routine,
            } => {
                let routine = routine.clone().ok_or_else(|| {
                    DriverError::Other(format!("{hardware}: which calibration routine?"))
                })?;
                request = request.with_arg("routine", TypedConfigValue::String(routine));
            }
            HardwareCommand::Restore { control: None }
            | HardwareCommand::Release
            | HardwareCommand::Calibrate { .. } => {}
        }
        let output = self.call(hardware, request).await?;
        Ok(match output.get("applied") {
            Some(TypedConfigValue::F64(applied)) => Some(*applied),
            _ => None,
        })
    }

    async fn calibration_status(
        &self,
        _device: &Identity,
        hardware: &str,
    ) -> Result<CalibrationStatus, DriverError> {
        let name = "calibration.status";
        let request = ActionRequest::new(action_id(name), self.resource(hardware).await?, name);
        Ok(calibration_status(&self.call(hardware, request).await?))
    }
}

impl OrionHardware {
    /// Runs one action and answers its output, or why it didn't run.
    async fn call(
        &self,
        hardware: &str,
        request: ActionRequest,
    ) -> Result<BTreeMap<String, TypedConfigValue>, DriverError> {
        let result = self.transport.call_action(request, TIMEOUT).await?;
        match result.state {
            ActionState::Succeeded => Ok(result.output),
            ActionState::Rejected { reason } | ActionState::Failed { reason } => {
                Err(DriverError::Other(format!("{hardware}: {reason}")))
            }
            ActionState::TimedOut => Err(DriverError::Other(format!(
                "{hardware} didn't answer in time"
            ))),
            ActionState::Accepted | ActionState::Running { .. } => Err(DriverError::Other(
                format!("{hardware} is still working on it; check its value in a moment"),
            )),
        }
    }
}

/// `calibration.status`'s output (Lemnos `calibration_output`): `revision`,
/// `running` (`none` or the routine), `progress`, `candidate`, `failed`, and
/// per part (`accel`, `gyro`, `mag`) `samples`, `confidence`, `coverage`,
/// `residual`, `active`. A part that reads all zeros isn't the device's.
fn calibration_status(output: &BTreeMap<String, TypedConfigValue>) -> CalibrationStatus {
    let number = |key: &str| match output.get(key) {
        Some(TypedConfigValue::F64(v)) => *v,
        Some(TypedConfigValue::UInt(v)) => *v as f64,
        Some(TypedConfigValue::Int(v)) => *v as f64,
        _ => 0.0,
    };
    let count = |key: &str| match output.get(key) {
        Some(TypedConfigValue::UInt(v)) => *v,
        Some(TypedConfigValue::Int(v)) => u64::try_from(*v).unwrap_or(0),
        _ => 0,
    };
    let flag = |key: &str| matches!(output.get(key), Some(TypedConfigValue::Bool(true)));
    let parts = ["accel", "gyro", "mag"]
        .into_iter()
        .map(|name| {
            let part = CalibrationPart {
                samples: count(&format!("{name}.samples")),
                confidence: number(&format!("{name}.confidence")),
                coverage: number(&format!("{name}.coverage")),
                residual: number(&format!("{name}.residual")),
                active: flag(&format!("{name}.active")),
            };
            (name.to_owned(), part)
        })
        .filter(|(_, part)| *part != CalibrationPart::default())
        .collect();
    CalibrationStatus {
        revision: count("revision"),
        running: match output.get("running") {
            Some(TypedConfigValue::String(r)) if r != "none" => Some(r.clone()),
            _ => None,
        },
        progress: number("progress"),
        candidate: flag("candidate"),
        failed: flag("failed"),
        parts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_status_reads_lemnos_output() {
        let mut out = BTreeMap::new();
        out.insert("revision".into(), TypedConfigValue::UInt(3));
        out.insert(
            "running".into(),
            TypedConfigValue::String("accel-six".into()),
        );
        out.insert("progress".into(), TypedConfigValue::F64(0.5));
        out.insert("candidate".into(), TypedConfigValue::Bool(false));
        out.insert("failed".into(), TypedConfigValue::Bool(false));
        for part in ["accel", "gyro", "mag"] {
            let on = part != "mag";
            out.insert(
                format!("{part}.samples"),
                TypedConfigValue::UInt(if on { 120 } else { 0 }),
            );
            out.insert(
                format!("{part}.confidence"),
                TypedConfigValue::F64(if on { 0.8 } else { 0.0 }),
            );
            out.insert(
                format!("{part}.coverage"),
                TypedConfigValue::F64(if on { 0.5 } else { 0.0 }),
            );
            out.insert(format!("{part}.residual"), TypedConfigValue::F64(0.0));
            out.insert(format!("{part}.active"), TypedConfigValue::Bool(on));
        }
        let status = calibration_status(&out);
        assert_eq!(status.running.as_deref(), Some("accel-six"));
        assert_eq!((status.revision, status.progress), (3, 0.5));
        assert_eq!(
            status.parts.keys().collect::<Vec<_>>(),
            ["accel", "gyro"],
            "the IMU has no mag part"
        );
        assert!(status.parts["accel"].active);
        out.insert("running".into(), TypedConfigValue::String("none".into()));
        assert_eq!(calibration_status(&out).running, None);
    }
}
