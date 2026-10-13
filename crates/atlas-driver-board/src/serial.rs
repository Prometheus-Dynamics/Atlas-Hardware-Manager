//! The board's USB serial console, found next to its network identity.
//!
//! A board's USB gadget includes an ACM serial port with a console on
//! it (device package 1.0.7+). It works without networking, SSH, or any
//! agent, so Atlas points at it whenever the board is plugged in: the port
//! whose USB serial matches the serial the identity endpoint reported.

/// The host's tty for the board with this USB serial, such as
/// `/dev/ttyACM0`: its first console, the one for people. Linux only for now.
pub fn serial_console(serial: &str) -> Option<String> {
    serial_consoles(serial).into_iter().next()
}

/// Every console tty of the board with this USB serial, by USB interface
/// (ttyGS0's first). Linux only for now.
pub(crate) fn serial_consoles(serial: &str) -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        linux::find_all(std::path::Path::new("/sys/class/tty"), serial)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = serial;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    /// The ttyACM* under `class_dir` whose USB device has `serial`, by
    /// interface number.
    pub(super) fn find_all(class_dir: &Path, serial: &str) -> Vec<String> {
        let serial = serial.trim();
        if serial.is_empty() {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(class_dir) else {
            return Vec::new();
        };
        let mut found: Vec<(u32, String)> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("ttyACM"))
            .filter_map(|name| {
                // device -> the USB interface; its parent is the USB device.
                let interface = std::fs::canonicalize(class_dir.join(&name).join("device")).ok()?;
                let usb_serial =
                    std::fs::read_to_string(interface.parent()?.join("serial")).ok()?;
                if !usb_serial.trim().eq_ignore_ascii_case(serial) {
                    return None;
                }
                let number = std::fs::read_to_string(interface.join("bInterfaceNumber"))
                    .ok()
                    .and_then(|text| u32::from_str_radix(text.trim(), 16).ok())
                    .unwrap_or(u32::MAX);
                Some((number, format!("/dev/{name}")))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, path)| path).collect()
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

            std::fs::write(usb.join("1-4:1.2/bInterfaceNumber"), "04\n").unwrap();
            std::fs::create_dir_all(usb.join("1-4:1.0")).unwrap();
            std::fs::write(usb.join("1-4:1.0/bInterfaceNumber"), "02\n").unwrap();
            std::fs::create_dir_all(class.join("ttyACM7")).unwrap();
            std::os::unix::fs::symlink(usb.join("1-4:1.0"), class.join("ttyACM7/device")).unwrap();

            assert_eq!(
                find_all(&class, "A317BCBEE5226D57"),
                vec!["/dev/ttyACM7".to_owned(), "/dev/ttyACM0".to_owned()],
                "by interface: ttyGS0's first"
            );
            assert!(find_all(&class, "someone-else").is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
