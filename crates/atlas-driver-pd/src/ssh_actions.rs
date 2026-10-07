//! Work that needs root on the board, run over SSH like updates (`ssh`):
//!
//! - `usb-boot`: restart straight into USB boot so Atlas can write a fresh
//!   image without anyone holding the boot button. It is never offered on
//!   the identity endpoint, which is unauthenticated: a board waiting in USB
//!   boot stays there until it is flashed or power-cycled.
//! - `set-clock`: set the board's clock (no RTC battery, often no NTP) from
//!   this computer's.
//! - the self-test (`/usr/lib/pd-device/selftest --json`), for boards whose
//!   identity lists `selftest` in `diagnostics`.

use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    ActionsCapability, DeviceAction, DriverError, Identity, SelfTestCapability, SelfTestReport,
};

use crate::live::PdActions;
use crate::ssh::SshUpdate;

/// The update method a board lists when it can restart into USB boot.
pub const USB_BOOT_METHOD: &str = "usb-boot-reboot";
/// The action id, shared with the app.
pub const USB_BOOT_ACTION: &str = "usb-boot";
/// Sets the board's clock from this computer's.
pub const SET_CLOCK_ACTION: &str = "set-clock";

const USB_BOOT: &str = "/usr/lib/pd-device/usb-boot";

/// What a board lists in `diagnostics` when it has the self-test.
pub const SELFTEST_DIAGNOSTIC: &str = "selftest";
const SELFTEST: &str = "/usr/lib/pd-device/selftest --json";
/// Stepping the fan through its states takes the longest, about 10 s.
const SELFTEST_TIMEOUT: Duration = Duration::from_secs(3 * 60);

pub(crate) fn usb_boot_action() -> DeviceAction {
    DeviceAction {
        id: USB_BOOT_ACTION.into(),
        label: "Restart into USB boot".into(),
        destructive: true,
    }
}

fn set_clock_action() -> DeviceAction {
    DeviceAction {
        id: SET_CLOCK_ACTION.into(),
        label: "Set clock from this computer".into(),
        destructive: false,
    }
}

/// The board's own actions (identity endpoint) plus the SSH ones.
pub(crate) struct PdDeviceActions {
    pub(crate) http: Option<PdActions>,
    pub(crate) ssh: Option<SshUpdate>,
    /// Offer restarting into USB boot (the board supports it).
    pub(crate) usb_boot: bool,
    /// Offer setting the clock (it is off).
    pub(crate) set_clock: bool,
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
            actions.retain(|action| action.id != USB_BOOT_ACTION && action.id != SET_CLOCK_ACTION);
            if self.set_clock {
                actions.push(set_clock_action());
            }
            if self.usb_boot {
                actions.push(usb_boot_action());
            }
        }
        actions
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        match (&self.ssh, &self.http) {
            (Some(ssh), _) if action_id == USB_BOOT_ACTION => restart_into_usb_boot(ssh).await,
            (Some(ssh), _) if action_id == SET_CLOCK_ACTION => set_clock(ssh).await,
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

/// Sets the board's clock to this computer's, in UTC, a form both busybox
/// and coreutils `date -s` accept. A board without an RTC keeps it until
/// power-off (or until NTP takes over).
async fn set_clock(ssh: &SshUpdate) -> Result<(), DriverError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| DriverError::Other(error.to_string()))?;
    let stamp = utc_stamp(now.as_secs());
    ssh.run(&format!("date -u -s '{stamp}' >/dev/null"))
        .await
        .map(|_| ())
}

/// `YYYY-MM-DD hh:mm:ss` in UTC for Unix seconds (proleptic Gregorian).
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The board's self-test over SSH. The board puts the fan and LEDs back
/// itself, also when the connection drops.
pub(crate) struct PdSelfTest {
    pub(crate) ssh: SshUpdate,
}

#[async_trait]
impl SelfTestCapability for PdSelfTest {
    async fn run_selftest(&self, _device: &Identity) -> Result<SelfTestReport, DriverError> {
        let output = tokio::time::timeout(SELFTEST_TIMEOUT, self.ssh.run(SELFTEST))
            .await
            .map_err(|_| DriverError::Other("the self-test took too long".into()))??;
        SelfTestReport::parse(&output).map_err(DriverError::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamps_are_calendar_dates() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc_stamp(1_791_331_200), "2026-10-07 00:00:00");
        assert_eq!(utc_stamp(1_791_331_200 + 3_661), "2026-10-07 01:01:01");
    }

    #[test]
    fn the_usb_boot_action_needs_a_confirm() {
        let action = usb_boot_action();
        assert_eq!(action.id, "usb-boot");
        assert!(action.destructive);
    }
}
