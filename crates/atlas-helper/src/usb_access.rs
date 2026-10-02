//! `atlas-helper install-usb-access`: lets this computer open Raspberry Pi
//! boards in USB boot mode without root. One elevated run, then never again.
//!
//! - Linux: installs the udev rule in `/etc/udev/rules.d` and reloads udev.
//! - Windows: binds WinUSB to the Pi boot ROM ids with `wdi-simple` (libwdi),
//!   the same way Raspberry Pi's rpiboot installer does.
//! - macOS: nothing is needed.

use std::path::PathBuf;

use atlas_blockdev::HelperMessage;

use crate::Reporter;

pub(crate) fn parse(args: &[String]) -> Result<Option<PathBuf>, String> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--progress" => Ok(Some(PathBuf::from(path))),
        _ => Err("usage: atlas-helper install-usb-access [--progress <file>]".into()),
    }
}

pub(crate) fn run(progress: Option<PathBuf>) -> Result<(), String> {
    let mut reporter = Reporter::open(progress.as_deref())?;
    match install() {
        Ok(message) => {
            reporter.send(&HelperMessage::Fixed { message });
            Ok(())
        }
        Err(message) => {
            reporter.send(&HelperMessage::Error {
                message: message.clone(),
            });
            Err(message)
        }
    }
}

#[cfg(target_os = "linux")]
const RULES_PATH: &str = "/etc/udev/rules.d/60-atlas-usbboot.rules";

#[cfg(target_os = "linux")]
fn install() -> Result<String, String> {
    use std::process::Command;

    std::fs::write(RULES_PATH, atlas_usbboot::LINUX_UDEV_RULES_FILE)
        .map_err(|error| format!("could not write {RULES_PATH}: {error}"))?;
    let reload = Command::new("udevadm")
        .args(["control", "--reload-rules"])
        .status()
        .map_err(|error| format!("udevadm: {error}"))?;
    if !reload.success() {
        return Err("udevadm could not reload the rules".into());
    }
    // Applies the rule to a board that is already plugged in.
    let _ = Command::new("udevadm")
        .args([
            "trigger",
            "--action=add",
            "--subsystem-match=usb",
            "--attr-match=idVendor=0a5c",
        ])
        .status();
    Ok(format!(
        "Installed {RULES_PATH}. Pi boards in USB boot mode can now be opened without root."
    ))
}

#[cfg(target_os = "windows")]
fn wdi_simple() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()
        .map(std::path::Path::to_path_buf)?;
    std::env::var_os("ATLAS_WDI_SIMPLE")
        .map(PathBuf::from)
        .into_iter()
        .chain([
            exe_dir.join("drivers").join("wdi-simple.exe"),
            exe_dir.join("wdi-simple.exe"),
        ])
        .find(|path| path.is_file())
}

#[cfg(target_os = "windows")]
fn install() -> Result<String, String> {
    use std::process::Command;

    let tool = wdi_simple().ok_or_else(|| {
        "the WinUSB installer (wdi-simple.exe) is missing from this installation".to_string()
    })?;
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for pid in atlas_usbboot::BOOT_PRODUCT_IDS {
        let status = Command::new(&tool)
            .args([
                "-n",
                "Raspberry Pi USB boot",
                "-v",
                "0x0a5c",
                "-p",
                &format!("0x{pid:04x}"),
                "-t",
                "0",
            ])
            .status();
        match status {
            Ok(status) if status.success() => installed.push(format!("{pid:04x}")),
            Ok(status) => failed.push(format!("{pid:04x} (exit {status})")),
            Err(error) => failed.push(format!("{pid:04x} ({error})")),
        }
    }
    if installed.is_empty() {
        return Err(format!(
            "WinUSB could not be installed: {}",
            failed.join(", ")
        ));
    }
    let mut message = format!(
        "WinUSB is bound to the Pi boot devices 0a5c:{}.",
        installed.join(", 0a5c:")
    );
    if !failed.is_empty() {
        message.push_str(&format!(" Not installed: {}.", failed.join(", ")));
    }
    Ok(message)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn install() -> Result<String, String> {
    Ok("No setup is needed on this system to open Pi boot devices.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_parse() {
        assert_eq!(parse(&[]).unwrap(), None);
        assert_eq!(
            parse(&["--progress".into(), "/tmp/p".into()]).unwrap(),
            Some(PathBuf::from("/tmp/p"))
        );
        assert!(parse(&["--bogus".into()]).is_err());
    }

    #[test]
    fn the_rules_file_carries_the_rule() {
        assert!(atlas_usbboot::LINUX_UDEV_RULES_FILE.contains(atlas_usbboot::LINUX_UDEV_RULE));
    }
}
