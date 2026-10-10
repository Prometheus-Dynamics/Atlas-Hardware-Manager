//! The mass-storage gadget's USB serial console: Raspberry Pi's
//! `mass-storage-gadget64` boot image exposes the eMMC and, on the same USB
//! device (0a5c:0104), an ACM serial port with a root shell (`agetty
//! --autologin root` on ttyGS0). After a flash, Atlas types `sync && reboot`
//! there, so the board flushes the eMMC and starts the new image without
//! anyone power-cycling it. The one-time USB boot order (the board's
//! `usb-boot`) has cleared itself, so the restart boots the eMMC, unless the
//! boot button is still held. Linux only for now (sysfs finds the port).

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The gadget's USB ids (Broadcom, "Multifunction Composite Gadget").
const GADGET_VENDOR: &str = "0a5c";
const GADGET_PRODUCT: &str = "0104";

/// How long the gadget may take to go after the reboot is typed.
const GONE_TIMEOUT: Duration = Duration::from_secs(20);
const GONE_POLL: Duration = Duration::from_millis(250);

/// The USB device directory a sysfs node hangs off: the nearest ancestor
/// with an `idVendor` file.
fn usb_device_dir(node: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(node).ok()?;
    path.ancestors()
        .find(|dir| dir.join("idVendor").is_file())
        .map(Path::to_path_buf)
}

fn attr(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name))
        .map(|text| text.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The console tty (its name, such as `ttyACM0`) on the same USB device as
/// the gadget disk `disk_name` (such as `sdc`), with sysfs at `sys`.
pub(crate) fn find_console(sys: &Path, disk_name: &str) -> Result<String, String> {
    let device = usb_device_dir(&sys.join("block").join(disk_name))
        .ok_or_else(|| format!("{disk_name} isn't a USB disk"))?;
    if attr(&device, "idVendor") != GADGET_VENDOR || attr(&device, "idProduct") != GADGET_PRODUCT {
        return Err(format!(
            "{disk_name} isn't Raspberry Pi's mass-storage gadget ({}:{})",
            attr(&device, "idVendor"),
            attr(&device, "idProduct")
        ));
    }
    let mut found: Vec<String> = std::fs::read_dir(sys.join("class").join("tty"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("ttyACM"))
        .filter(|name| {
            usb_device_dir(&sys.join("class").join("tty").join(name)).as_deref()
                == Some(device.as_path())
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .next()
        .ok_or_else(|| "the gadget has no serial console (an older boot image?)".to_string())
}

/// Types the restart into the console at `/dev/<tty>`.
#[cfg(target_os = "linux")]
fn type_restart(tty: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    use nix::fcntl::OFlag;

    let path = Path::new("/dev").join(tty);
    // No carrier to wait for, and not this process's controlling terminal.
    let mut port = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags((OFlag::O_NOCTTY | OFlag::O_NONBLOCK).bits())
        .open(&path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => format!(
                "no permission to open {} (update the USB boot rule in Settings > Host health, \
                 or add yourself to the dialout group)",
                path.display()
            ),
            _ => format!("{}: {error}", path.display()),
        })?;
    // A fresh line first, in case anything was half typed.
    port.write_all(b"\r\nsync && reboot\r\n")
        .and_then(|()| port.flush())
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Restarts the board behind the gadget disk `disk_name` through its serial
/// console, and waits for the gadget to go. Returns the console used.
#[cfg(target_os = "linux")]
pub(crate) fn restart(disk_name: &str) -> Result<String, String> {
    let sys = Path::new("/sys");
    let tty = find_console(sys, disk_name)?;
    type_restart(&tty)?;
    let entry = sys.join("class").join("tty").join(&tty);
    let deadline = std::time::Instant::now() + GONE_TIMEOUT;
    while entry.exists() {
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "typed the restart into {tty}, but the board is still in USB boot after {} s",
                GONE_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(GONE_POLL);
    }
    Ok(tty)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn restart(_disk_name: &str) -> Result<String, String> {
    Err("restarting the board from Atlas works on Linux only for now".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sysfs with one USB device (`ids`) carrying disk `sdc` and, when
    /// `console`, ttyACM0; plus an unrelated ttyACM1 on another device.
    fn sysfs(root: &Path, ids: (&str, &str), console: bool) -> PathBuf {
        let sys = root.join("sys");
        let device = sys.join("devices/usb3/3-2");
        std::fs::create_dir_all(device.join("3-2:1.0/host0/target0/0:0:0:0/block/sdc")).unwrap();
        std::fs::write(device.join("idVendor"), format!("{}\n", ids.0)).unwrap();
        std::fs::write(device.join("idProduct"), format!("{}\n", ids.1)).unwrap();
        std::fs::create_dir_all(sys.join("block")).unwrap();
        std::os::unix::fs::symlink(
            device.join("3-2:1.0/host0/target0/0:0:0:0/block/sdc"),
            sys.join("block/sdc"),
        )
        .unwrap();
        std::fs::create_dir_all(sys.join("class/tty")).unwrap();
        if console {
            std::fs::create_dir_all(device.join("3-2:1.1/tty/ttyACM0")).unwrap();
            std::os::unix::fs::symlink(
                device.join("3-2:1.1/tty/ttyACM0"),
                sys.join("class/tty/ttyACM0"),
            )
            .unwrap();
        }
        let other = sys.join("devices/usb1/1-1");
        std::fs::create_dir_all(other.join("1-1:1.0/tty/ttyACM1")).unwrap();
        std::fs::write(other.join("idVendor"), "2341\n").unwrap();
        std::os::unix::fs::symlink(
            other.join("1-1:1.0/tty/ttyACM1"),
            sys.join("class/tty/ttyACM1"),
        )
        .unwrap();
        sys
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas-console-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_console_on_the_gadgets_own_usb_device() {
        let root = temp("found");
        let sys = sysfs(&root, ("0a5c", "0104"), true);
        assert_eq!(find_console(&sys, "sdc").unwrap(), "ttyACM0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_raspberry_pi_gadget_and_never_another_serial_port() {
        let root = temp("other");
        let sys = sysfs(&root, ("0781", "5581"), true);
        let error = find_console(&sys, "sdc").unwrap_err();
        assert!(
            error.contains("isn't Raspberry Pi's mass-storage gadget"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);

        let root = temp("none");
        let sys = sysfs(&root, ("0a5c", "0104"), false);
        let error = find_console(&sys, "sdc").unwrap_err();
        assert!(error.contains("no serial console"), "{error}");
        assert!(find_console(&sys, "sdz").is_err(), "no such disk");
        let _ = std::fs::remove_dir_all(root);
    }
}
