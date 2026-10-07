//! The board's USB serial console, found next to its network identity.
//!
//! A board's USB gadget includes an ACM serial port with a console on
//! it (device package 1.0.7+). It works without networking, SSH, or any
//! agent, so Atlas points at it whenever the board is plugged in: the port
//! whose USB serial matches the serial the identity endpoint reported.

/// The host's tty for the board with this USB serial, such as
/// `/dev/ttyACM0`. Linux only for now.
pub fn serial_console(serial: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::find(std::path::Path::new("/sys/class/tty"), serial)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = serial;
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    /// The first ttyACM* under `class_dir` whose USB device has `serial`.
    pub(super) fn find(class_dir: &Path, serial: &str) -> Option<String> {
        let serial = serial.trim();
        if serial.is_empty() {
            return None;
        }
        let mut names: Vec<String> = std::fs::read_dir(class_dir)
            .ok()?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("ttyACM"))
            .collect();
        names.sort();
        names.into_iter().find_map(|name| {
            // device -> the USB interface; its parent is the USB device.
            let interface = std::fs::canonicalize(class_dir.join(&name).join("device")).ok()?;
            let usb_serial = std::fs::read_to_string(interface.parent()?.join("serial")).ok()?;
            usb_serial
                .trim()
                .eq_ignore_ascii_case(serial)
                .then(|| format!("/dev/{name}"))
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_tty_with_the_boards_serial_is_found() {
            let root = std::env::temp_dir().join(format!("atlas-tty-{}", std::process::id()));
            let usb = root.join("devices/usb1/1-4");
            std::fs::create_dir_all(usb.join("1-4:1.2")).unwrap();
            std::fs::write(usb.join("serial"), "a317bcbee5226d57\n").unwrap();
            let class = root.join("class/tty");
            std::fs::create_dir_all(class.join("ttyACM0")).unwrap();
            std::os::unix::fs::symlink(usb.join("1-4:1.2"), class.join("ttyACM0/device")).unwrap();

            assert_eq!(
                find(&class, "A317BCBEE5226D57").as_deref(),
                Some("/dev/ttyACM0")
            );
            assert_eq!(find(&class, "someone-else"), None);
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
