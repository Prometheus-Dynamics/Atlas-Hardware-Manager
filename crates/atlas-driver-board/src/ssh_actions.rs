//! Work that needs root on the board, run over SSH like updates (`ssh`):
//!
//! - `usb-boot`: restart straight into USB boot so Atlas can write a fresh
//!   image without anyone holding the boot button. It is never offered on
//!   the identity endpoint, which is unauthenticated: a board waiting in USB
//!   boot stays there until it is flashed or power-cycled.
//! - `set-clock`: set the board's clock (no RTC battery, often no NTP) from
//!   this computer's.
//! - `reboot` and `power-off` (`systemctl`), and for A/B boards
//!   `update.cancel` and `update.rollback` (the writer's `cancel` and
//!   `rollback`). The ids are Orion's, so the app shows one control whichever
//!   way the board is reached.
//! - the self-test (`/usr/lib/board/selftest --json`), for boards whose
//!   identity lists `selftest` in `diagnostics`.
//!
//! Every command runs with `BOARD_EVENT_SOURCE=atlas` (`ssh`), so the
//! board's event log says Atlas asked.

use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{
    ActionsCapability, DeviceAction, DriverError, Identity, SelfTestCapability, SelfTestReport,
    SelfTestStep,
};

use crate::live::BoardActions;
use crate::ssh::SshUpdate;

/// The update method a board lists when it can restart into USB boot.
pub const USB_BOOT_METHOD: &str = "usb-boot-reboot";
/// The action id, shared with the app.
pub const USB_BOOT_ACTION: &str = "usb-boot";
/// Sets the board's clock from this computer's.
pub const SET_CLOCK_ACTION: &str = "set-clock";
/// Restarts the board.
pub const REBOOT_ACTION: &str = "reboot";
/// Shuts the board down; it stays off until its power is cycled.
pub const POWER_OFF_ACTION: &str = "power-off";
/// Stops a stage, or forgets a staged update (Orion's `update.cancel`).
pub const UPDATE_CANCEL_ACTION: &str = "update.cancel";
/// Restarts into the previous confirmed version (Orion's `update.rollback`).
pub const UPDATE_ROLLBACK_ACTION: &str = "update.rollback";

const USB_BOOT: &str = "/usr/lib/board/usb-boot";
const WRITER: &str = "/usr/lib/board/update";
/// Logs an event first when the image has the package's event log.
const EVENT: &str = "e=/usr/lib/board/event; [ ! -x \"$e\" ] || \"$e\"";

/// What a board lists in `diagnostics` when it has the self-test.
pub const SELFTEST_DIAGNOSTIC: &str = "selftest";
const SELFTEST: &str = "/usr/lib/board/selftest --json";
/// The same, telling each check as it goes (package 1.0.7's later builds).
const SELFTEST_LIVE: &str = "/usr/lib/board/selftest --json --progress";
/// Stepping the fan through its states takes the longest, about 10 s.
const SELFTEST_TIMEOUT: Duration = Duration::from_secs(3 * 60);

fn action(id: &str, label: &str, destructive: bool) -> DeviceAction {
    DeviceAction {
        id: id.into(),
        label: label.into(),
        destructive,
    }
}

pub(crate) fn usb_boot_action() -> DeviceAction {
    action(USB_BOOT_ACTION, "Restart into USB boot", true)
}

/// The SSH actions a board gets, by what it supports.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SshOffer {
    /// Restart into USB boot (the board supports it).
    pub(crate) usb_boot: bool,
    /// Set the clock (it is off).
    pub(crate) set_clock: bool,
    /// Restart and power off (a device-package board Atlas reaches as root).
    pub(crate) power: bool,
    /// Cancel and roll back updates (the A/B writer).
    pub(crate) updates: bool,
}

impl SshOffer {
    fn actions(self) -> Vec<DeviceAction> {
        let mut list = Vec::new();
        if self.power {
            list.push(action(REBOOT_ACTION, "Restart", false));
        }
        if self.set_clock {
            list.push(action(
                SET_CLOCK_ACTION,
                "Set clock from this computer",
                false,
            ));
        }
        if self.updates {
            list.push(action(UPDATE_CANCEL_ACTION, "Cancel update", false));
            list.push(action(
                UPDATE_ROLLBACK_ACTION,
                "Go back to the previous version",
                true,
            ));
        }
        if self.power {
            list.push(action(POWER_OFF_ACTION, "Power off", true));
        }
        if self.usb_boot {
            list.push(usb_boot_action());
        }
        list
    }
}

/// The board's own actions (identity endpoint) plus the SSH ones; an SSH
/// action replaces the endpoint's action of the same id.
pub(crate) struct BoardDeviceActions {
    pub(crate) http: Option<BoardActions>,
    pub(crate) ssh: Option<SshUpdate>,
    pub(crate) offer: SshOffer,
}

#[async_trait]
impl ActionsCapability for BoardDeviceActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        let mut actions = self
            .http
            .as_ref()
            .map(|http| http.actions(device))
            .unwrap_or_default();
        if self.ssh.is_some() {
            let ssh = self.offer.actions();
            actions.retain(|action| !ssh.iter().any(|own| own.id == action.id));
            actions.extend(ssh);
        }
        actions
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        let ours = self.offer.actions().iter().any(|a| a.id == action_id);
        match (&self.ssh, &self.http) {
            (Some(ssh), _) if ours => run_ssh_action(ssh, action_id).await,
            (_, Some(http)) => http.run_action(device, action_id).await,
            _ => Err(DriverError::Incompatible(format!(
                "this board doesn't offer {action_id}"
            ))),
        }
    }
}

async fn run_ssh_action(ssh: &SshUpdate, action_id: &str) -> Result<(), DriverError> {
    match action_id {
        USB_BOOT_ACTION => restart_into_usb_boot(ssh).await,
        SET_CLOCK_ACTION => set_clock(ssh).await,
        REBOOT_ACTION => {
            restart(
                ssh,
                &format!("{EVENT} reboot 'restart requested from Atlas'; systemctl reboot"),
            )
            .await
        }
        POWER_OFF_ACTION => {
            restart(
                ssh,
                &format!("{EVENT} power-off 'power-off requested from Atlas'; systemctl poweroff"),
            )
            .await
        }
        UPDATE_CANCEL_ACTION => writer(ssh, &format!("{WRITER} cancel")).await,
        // The writer switches, then restarts the board, which drops the
        // connection.
        UPDATE_ROLLBACK_ACTION => match writer(ssh, &format!("{WRITER} rollback")).await {
            Err(DriverError::Unreachable(_)) => Ok(()),
            other => other,
        },
        other => Err(DriverError::Unsupported(format!("no action {other}"))),
    }
}

/// A restart or power-off usually drops the connection, which counts as
/// success.
async fn restart(ssh: &SshUpdate, script: &str) -> Result<(), DriverError> {
    match ssh.run(script).await {
        Ok(_) | Err(DriverError::Unreachable(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// The writer's refusal (exit 3, "nothing changed") reads as its reason.
async fn writer(ssh: &SshUpdate, script: &str) -> Result<(), DriverError> {
    ssh.run(script)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            DriverError::Other(reason) => {
                DriverError::Incompatible(reason.trim_start_matches("update: ").to_string())
            }
            other => other,
        })
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
/// and coreutils `date -s` accept, and logs the change. A board without an
/// RTC keeps it until power-off (or until NTP takes over).
async fn set_clock(ssh: &SshUpdate) -> Result<(), DriverError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| DriverError::Other(error.to_string()))?;
    let stamp = utc_stamp(now.as_secs());
    ssh.run(&set_clock_script(&stamp)).await.map(|_| ())
}

fn set_clock_script(stamp: &str) -> String {
    format!(
        "old=$(date +%s); date -u -s '{stamp}' >/dev/null && {{ {EVENT} clock.set \"clock set from a computer over SSH\" \"old=$old\" \"new=$(date +%s)\"; true; }}"
    )
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
pub(crate) struct BoardSelfTest {
    pub(crate) ssh: SshUpdate,
}

#[async_trait]
impl SelfTestCapability for BoardSelfTest {
    async fn run_selftest(&self, _device: &Identity) -> Result<SelfTestReport, DriverError> {
        let output = tokio::time::timeout(SELFTEST_TIMEOUT, self.ssh.run(SELFTEST))
            .await
            .map_err(|_| DriverError::Other("the self-test took too long".into()))??;
        SelfTestReport::parse(&output).map_err(DriverError::Other)
    }

    async fn run_selftest_live(
        &self,
        device: &Identity,
        progress: &(dyn Fn(SelfTestStep) + Send + Sync),
    ) -> Result<SelfTestReport, DriverError> {
        let on_line = |line: &str| {
            if let Some(step) = SelfTestStep::parse(line) {
                progress(step);
            }
        };
        let run = tokio::time::timeout(
            SELFTEST_TIMEOUT,
            self.ssh.run_watching(SELFTEST_LIVE, &on_line),
        )
        .await
        .map_err(|_| DriverError::Other("the self-test took too long".into()))?;
        match run {
            Ok(output) => SelfTestReport::parse(&output).map_err(DriverError::Other),
            // An older board doesn't know --progress (exit 2): run it plain.
            Err(error) if error.to_string().contains("unknown argument") => {
                self.run_selftest(device).await
            }
            Err(error) => Err(error),
        }
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

    #[test]
    fn the_offer_lists_what_the_board_supports() {
        let ids = |offer: SshOffer| -> Vec<(String, bool)> {
            offer
                .actions()
                .into_iter()
                .map(|a| (a.id, a.destructive))
                .collect()
        };
        assert!(ids(SshOffer::default()).is_empty());
        let all = ids(SshOffer {
            usb_boot: true,
            set_clock: true,
            power: true,
            updates: true,
        });
        let expect = [
            ("reboot", false),
            ("set-clock", false),
            ("update.cancel", false),
            ("update.rollback", true),
            ("power-off", true),
            ("usb-boot", true),
        ];
        assert_eq!(
            all,
            expect
                .iter()
                .map(|(id, d)| (id.to_string(), *d))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn set_clock_logs_on_boards_with_the_event_log() {
        let script = set_clock_script("2026-10-07 00:00:00");
        assert!(
            script.starts_with("old=$(date +%s); date -u -s '2026-10-07 00:00:00' >/dev/null && ")
        );
        assert!(script.contains("[ ! -x \"$e\" ] || \"$e\" clock.set"));
        assert!(script.ends_with("true; }"));
    }
}
