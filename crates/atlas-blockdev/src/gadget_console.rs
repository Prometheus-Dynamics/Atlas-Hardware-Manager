//! The mass-storage gadget's USB serial console: Raspberry Pi's
//! `mass-storage-gadget64` boot image exposes the eMMC and, on the same USB
//! device (0a5c:0104), an ACM serial port with a root shell (`agetty
//! --autologin root` on ttyGS0). After a flash, Atlas types a `sync` and
//! `reboot -f` there, so the board flushes the eMMC and starts the new image without
//! anyone power-cycling it. The one-time USB boot order (the board's
//! `usb-boot`) has cleared itself, so the restart boots the eMMC, unless the
//! boot button is still held. Linux only for now (sysfs finds the port), so
//! elsewhere only the stub `restart` is used.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The gadget's USB ids (Broadcom, "Multifunction Composite Gadget").
const GADGET_VENDOR: &str = "0a5c";
const GADGET_PRODUCT: &str = "0104";

/// How long the gadget may take to go after the reboot is typed. The
/// gadget writes through its page cache: after a whole image, `sync` can
/// spend minutes putting what it still holds on the eMMC.
const GONE_TIMEOUT: Duration = Duration::from_secs(180);
/// How long the board's shell may take to print that it took the line.
const ECHO_TIMEOUT: Duration = Duration::from_secs(3);
const GONE_POLL: Duration = Duration::from_millis(250);

/// How long typing the restart may wait for the console to take it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_POLL: Duration = Duration::from_millis(100);

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
fn find_console(sys: &Path, disk_name: &str) -> Result<String, String> {
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
    let tty = found
        .into_iter()
        .next()
        .ok_or_else(|| "the gadget has no serial console (an older boot image?)".to_string())?;
    // Never this host's own console, whatever the sysfs says.
    let own = std::fs::read_to_string(sys.join("class/tty/console/active")).unwrap_or_default();
    if own.split_whitespace().any(|name| name == tty) {
        return Err(format!("{tty} is this host's own console"));
    }
    Ok(tty)
}

/// What the board's shell prints, worked out there so the console's echo of
/// the typed line can never match (a login prompt echoes too): it took the
/// line, then `sync` returned. `reboot -f` follows the sync: the gadget has
/// nothing else to stop, and its shutdown scripts once held the restart for
/// minutes.
const RESTART_LINE: &[u8] =
    b"\r\necho ATLAS-$((40+2))-TOOK; sync; echo ATLAS-$((40+2))-SYNCED; reboot -f\r\n";
const TOOK: &str = "ATLAS-42-TOOK";
const SYNCED: &str = "ATLAS-42-SYNCED";

/// The console, open for typing and reading.
#[cfg(target_os = "linux")]
struct Console {
    port: std::fs::File,
    path: PathBuf,
    seen: Vec<u8>,
}

#[cfg(target_os = "linux")]
impl Console {
    fn open(tty: &str) -> Result<Self, String> {
        use std::os::unix::fs::OpenOptionsExt;

        use nix::fcntl::OFlag;

        let path = Path::new("/dev").join(tty);
        // Only the tty itself: a regular file here (someone's stray write)
        // would "answer" with whatever was written to it.
        let meta =
            std::fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !std::os::unix::fs::FileTypeExt::is_char_device(&meta.file_type()) {
            return Err(format!("{} isn't a serial device", path.display()));
        }
        // No carrier to wait for, and not this process's controlling terminal.
        let port = std::fs::OpenOptions::new()
            .read(true)
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
        Ok(Self {
            port,
            path,
            seen: Vec::new(),
        })
    }

    /// Types `line`. The port is non-blocking: a write the line can't take
    /// yet (the gadget just came up, or another program reading the console
    /// holds it back) says EAGAIN, so try again for a while.
    fn type_line(&mut self, mut line: &[u8]) -> Result<(), String> {
        use std::io::Write;

        let deadline = std::time::Instant::now() + WRITE_TIMEOUT;
        while !line.is_empty() {
            match self.port.write(line) {
                Ok(0) => {}
                Ok(written) => {
                    line = &line[written..];
                    continue;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("{}: {error}", self.path.display())),
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!(
                    "{}: the console took no input for {} s (is another program holding it?)",
                    self.path.display(),
                    WRITE_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(WRITE_POLL);
        }
        self.port
            .flush()
            .map_err(|error| format!("{}: {error}", self.path.display()))
    }

    /// Reads what is there; false once the console is gone.
    fn read_some(&mut self) -> bool {
        use std::io::Read;

        let mut buffer = [0u8; 256];
        loop {
            match self.port.read(&mut buffer) {
                Ok(0) => return true,
                Ok(read) => self.seen.extend_from_slice(&buffer[..read]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::Interrupted =>
                {
                    return true;
                }
                Err(_) => return false,
            }
        }
    }

    fn printed(&self, marker: &str) -> bool {
        printed(&self.seen, marker)
    }
}

/// Whether the shell printed `marker` (the typed line holds only its
/// `$((40+2))` form).
fn printed(output: &[u8], marker: &str) -> bool {
    String::from_utf8_lossy(output).contains(marker)
}

/// Restarts the board behind the gadget disk `disk_name` through its serial
/// console, and waits for the gadget to go. Returns the console used and how
/// long the board's `sync` took.
#[cfg(target_os = "linux")]
pub fn restart_gadget(disk_name: &str) -> Result<String, String> {
    let sys = Path::new("/sys");
    let tty = find_console(sys, disk_name)?;
    let mut console = Console::open(&tty)?;
    console.type_line(RESTART_LINE)?;
    let entry = sys.join("class").join("tty").join(&tty);
    let started = std::time::Instant::now();
    let mut synced: Option<Duration> = None;
    loop {
        let open = console.read_some();
        if synced.is_none() && console.printed(SYNCED) {
            synced = Some(started.elapsed());
        }
        if !entry.exists() || (!open && console.printed(TOOK)) {
            return Ok(match synced {
                Some(after) => format!("{tty}, synced in {} s", after.as_secs()),
                None => tty,
            });
        }
        let took = console.printed(TOOK);
        // A console that didn't run the line: no point waiting minutes.
        let wait = if took { GONE_TIMEOUT } else { ECHO_TIMEOUT };
        if started.elapsed() >= wait {
            return Err(match (took, synced) {
                (false, _) => format!(
                    "typed the restart into {tty}, but its shell didn't run it and the board is still in USB boot; power-cycle it without holding its boot button"
                ),
                (true, None) => format!(
                    "the board took the restart on {tty} but is still putting the image on its eMMC after {} s; leave it powered until it restarts by itself (a power-cycle now can leave the image incomplete)",
                    wait.as_secs()
                ),
                (true, Some(after)) => format!(
                    "the board synced its eMMC in {} s but hadn't restarted {} s after; the image is written, so power-cycling it is safe",
                    after.as_secs(),
                    wait.as_secs()
                ),
            });
        }
        std::thread::sleep(GONE_POLL);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn restart_gadget(_disk_name: &str) -> Result<String, String> {
    Err("restarting the board from Atlas works on Linux only for now".into())
}

#[cfg(all(test, unix))]
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
    fn only_the_shells_own_output_says_it_took_the_restart() {
        // The echo of the typed line (a login prompt echoes it too).
        let echo = String::from_utf8_lossy(RESTART_LINE).into_owned();
        assert!(!printed(echo.as_bytes(), TOOK));
        assert!(!printed(echo.as_bytes(), SYNCED));
        assert!(!printed(format!("login: {echo}").as_bytes(), TOOK));
        assert!(printed(
            format!("# {echo}ATLAS-42-TOOK\r\n").as_bytes(),
            TOOK
        ));
        assert!(printed(b"ATLAS-42-TOOK\r\nATLAS-42-SYNCED\r\n", SYNCED));
    }

    #[test]
    fn the_console_on_the_gadgets_own_usb_device() {
        let root = temp("found");
        let sys = sysfs(&root, ("0a5c", "0104"), true);
        assert_eq!(find_console(&sys, "sdc").unwrap(), "ttyACM0");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn never_this_hosts_own_console() {
        let root = temp("own");
        let sys = sysfs(&root, ("0a5c", "0104"), true);
        std::fs::create_dir_all(sys.join("class/tty/console")).unwrap();
        std::fs::write(sys.join("class/tty/console/active"), "tty1 ttyACM0\n").unwrap();
        let error = find_console(&sys, "sdc").unwrap_err();
        assert!(error.contains("this host's own console"), "{error}");
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
