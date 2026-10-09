//! Commands to the board's devices through Lemnos's Orion bridge: `set`,
//! `restore` and `release` on a device's `lemnos.device` resource, answered
//! by request/response actions (the node holds its reply until the bridge
//! has one, so nothing polls).
//!
//! Lemnos names each Orion caller `orion:<requested_by>` and applies the
//! device's write policy to that name; it undoes a caller's writes when the
//! caller is gone, and `restore` undoes only this computer's.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{DriverError, HardwareCapability, HardwareCommand, Identity};
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
        let target = self.resource(hardware).await?;
        let name = match &command {
            HardwareCommand::Set { .. } => "set",
            HardwareCommand::Restore { .. } => "restore",
            HardwareCommand::Release => "release",
        };
        let mut request = ActionRequest::new(action_id(name), target, name);
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
            HardwareCommand::Restore { control: None } | HardwareCommand::Release => {}
        }
        let result = self.transport.call_action(request, TIMEOUT).await?;
        match result.state {
            ActionState::Succeeded => Ok(match result.output.get("applied") {
                Some(TypedConfigValue::F64(applied)) => Some(*applied),
                _ => None,
            }),
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
