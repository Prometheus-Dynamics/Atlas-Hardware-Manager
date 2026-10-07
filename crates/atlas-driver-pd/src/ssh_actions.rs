//! Actions that need root on the board, run over SSH like updates (`ssh`).
//! Today that is `usb-boot`: restart straight into USB boot so Atlas can write
//! a fresh image without anyone holding the boot button. It is never offered
//! on the identity endpoint, which is unauthenticated: a board waiting in USB
//! boot stays there until it is flashed or power-cycled.

use async_trait::async_trait;
use atlas_driver::{ActionsCapability, DeviceAction, DriverError, Identity};

use crate::live::PdActions;
use crate::ssh::SshUpdate;

/// The update method a board lists when it can restart into USB boot.
pub const USB_BOOT_METHOD: &str = "usb-boot-reboot";
/// The action id, shared with the app.
pub const USB_BOOT_ACTION: &str = "usb-boot";

const USB_BOOT: &str = "/usr/lib/pd-device/usb-boot";

pub(crate) fn usb_boot_action() -> DeviceAction {
    DeviceAction {
        id: USB_BOOT_ACTION.into(),
        label: "Restart into USB boot".into(),
        destructive: true,
    }
}

/// The board's own actions (identity endpoint) plus the SSH ones.
pub(crate) struct PdDeviceActions {
    pub(crate) http: Option<PdActions>,
    pub(crate) ssh: Option<SshUpdate>,
}

#[async_trait]
impl ActionsCapability for PdDeviceActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        let mut actions = self
            .http
            .as_ref()
            .map(|http| http.actions(device))
            .unwrap_or_default();
        if self.ssh.is_some() {
            actions.retain(|action| action.id != USB_BOOT_ACTION);
            actions.push(usb_boot_action());
        }
        actions
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        match (&self.ssh, &self.http) {
            (Some(ssh), _) if action_id == USB_BOOT_ACTION => restart_into_usb_boot(ssh).await,
            (_, Some(http)) => http.run_action(device, action_id).await,
            _ => Err(DriverError::Incompatible(format!(
                "this board doesn't offer {action_id}"
            ))),
        }
    }
}

/// Checks first, so a board that can't do it says so; the restart itself
/// usually drops the connection, which counts as success.
async fn restart_into_usb_boot(ssh: &SshUpdate) -> Result<(), DriverError> {
    ssh.run(&format!("{USB_BOOT} --check"))
        .await
        .map_err(|error| match error {
            DriverError::Other(_) => DriverError::Incompatible(
                "this board can't restart into USB boot by itself; use its boot button".into(),
            ),
            other => other,
        })?;
    match ssh.run(USB_BOOT).await {
        Ok(_) | Err(DriverError::Unreachable(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usb_boot_action_needs_a_confirm() {
        let action = usb_boot_action();
        assert_eq!(action.id, "usb-boot");
        assert!(action.destructive);
    }
}
