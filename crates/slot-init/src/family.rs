//! What the boot chip decides, for the BCM2712 (devices/families/rpi-bcm2712):
//! the Pi firmware boots the slot one-line files name, and takes a one-time
//! "0 tryboot" restart into the trial one.

use crate::boot::Env;
use crate::cmdline_arg;

/// On the boot FAT: the file naming the slot a trial restart boots.
pub const TRIAL_FILE: &str = "trial.txt";

/// The trial file's content for `slot`.
pub fn trial_entry(slot: &str) -> String {
    format!("os_prefix=slots/{slot}/\n")
}

/// reboot(2)'s argument for a restart into the trial slot.
pub const TRIAL_RESTART_ARG: &str = "0 tryboot";

/// On p1, a runner's "0 tryboot": the reboot argument doesn't reach it, so
/// a restart into tryboot.txt leaves this file, which the runner deletes
/// as it starts that one trial boot (with `atlas.tryboot=1`).
pub const TRYBOOT_REQUEST: &str = "tryboot.req";

/// Whether this boot is a trial: the firmware's chosen/bootloader/tryboot
/// (a big-endian 1), else a runner's `atlas.tryboot=1`.
pub fn is_trial(env: &Env, cmdline: &str) -> bool {
    match std::fs::read(env.at("proc/device-tree/chosen/bootloader/tryboot")) {
        Ok(b) => b.len() >= 4 && b[..4] == [0, 0, 0, 1],
        Err(_) => cmdline_arg(cmdline, "atlas.tryboot") == Some("1"),
    }
}
