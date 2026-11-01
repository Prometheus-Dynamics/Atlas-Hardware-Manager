//! The kernel's ioctls: raw flash partitions (`/dev/mtdN`), a trial's
//! watchdog, and `UBI_IOCVOLUP`, which starts a volume
//! update of N bytes on `/dev/ubiX_Y`; the next N bytes written to it
//! replace the volume's contents (the kernel sets the volume's update marker
//! until the last byte lands, so a cut-short update leaves the volume marked
//! corrupt, never half old and half new).

use std::fs::File;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::mtd::Mtd;

// The ioctl request's C type differs between glibc (unsigned long) and musl (int).
/// `_IOW('O', 0, __s64)` (include/uapi/mtd/ubi-user.h).
const UBI_IOCVOLUP: libc::Ioctl = 0x4008_4f00u32 as libc::Ioctl;

pub fn start_update(volume: &File, bytes: u64) -> Result<(), String> {
    let bytes = i64::try_from(bytes).map_err(|_| "the image is too large".to_string())?;
    // SAFETY: the fd is open for the call's duration and the kernel reads one
    // __s64 through the pointer, which points at a live i64.
    let result = unsafe { libc::ioctl(volume.as_raw_fd(), UBI_IOCVOLUP, &bytes as *const i64) };
    if result < 0 {
        return Err(format!(
            "starting the volume update: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// `_IOW('M', 2, struct erase_info_user)`, include/uapi/mtd/mtd-abi.h.
const MEMERASE: libc::Ioctl = 0x4008_4d02u32 as libc::Ioctl;
/// `_IOW('M', 11, __kernel_loff_t)`.
const MEMGETBADBLOCK: libc::Ioctl = 0x4008_4d0bu32 as libc::Ioctl;
/// `_IOWR('W', 6, int)`, include/uapi/linux/watchdog.h.
const WDIOC_SETTIMEOUT: libc::Ioctl = 0xc004_5706u32 as libc::Ioctl;

/// `struct erase_info_user`.
#[repr(C)]
struct EraseInfo {
    start: u32,
    length: u32,
}

fn last_error(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn sysfs_number(dir: &Path, name: &str) -> Result<u64, String> {
    let path = dir.join(name);
    std::fs::read_to_string(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .trim()
        .parse()
        .map_err(|_| format!("{}: not a number", path.display()))
}

/// A partition's `/dev/mtdN`.
pub struct LinuxMtd {
    file: File,
    node: String,
    size: u64,
    erase_size: u32,
    page_size: u32,
}

impl LinuxMtd {
    /// Opens `node` (`mtd4`), its geometry from `sysfs` (`/sys/class/mtd`).
    pub fn open(sysfs: &Path, dev: &Path, node: &str) -> Result<Self, String> {
        let dir = sysfs.join(node);
        let path = dev.join(node);
        Ok(Self {
            file: std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?,
            node: node.to_string(),
            size: sysfs_number(&dir, "size")?,
            erase_size: sysfs_number(&dir, "erasesize")? as u32,
            page_size: sysfs_number(&dir, "writesize")? as u32,
        })
    }
}

impl Mtd for LinuxMtd {
    fn size(&self) -> u64 {
        self.size
    }
    fn erase_size(&self) -> u32 {
        self.erase_size
    }
    fn page_size(&self) -> u32 {
        self.page_size
    }
    fn is_bad(&mut self, offset: u64) -> Result<bool, String> {
        let offset = offset as i64;
        // SAFETY: the fd is open for the call and the kernel reads one loff_t
        // through the pointer, which points at a live i64.
        let result =
            unsafe { libc::ioctl(self.file.as_raw_fd(), MEMGETBADBLOCK, &offset as *const i64) };
        if result < 0 {
            return Err(last_error(&format!("{}: bad block check", self.node)));
        }
        Ok(result > 0)
    }
    fn read(&mut self, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
        self.file
            .read_exact_at(buffer, offset)
            .map_err(|error| format!("{} at {offset:#x}: {error}", self.node))
    }
    fn erase(&mut self, offset: u64) -> Result<(), String> {
        let info = EraseInfo {
            start: u32::try_from(offset).map_err(|_| "offset past 4 GiB".to_string())?,
            length: self.erase_size,
        };
        // SAFETY: the fd is open for the call and the kernel reads one
        // erase_info_user through the pointer, which points at a live one.
        let result =
            unsafe { libc::ioctl(self.file.as_raw_fd(), MEMERASE, &info as *const EraseInfo) };
        if result < 0 {
            return Err(last_error(&format!("{}: erase at {offset:#x}", self.node)));
        }
        Ok(())
    }
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), String> {
        self.file
            .write_all_at(data, offset)
            .map_err(|error| format!("{} at {offset:#x}: {error}", self.node))
    }
}

/// `/dev/watchdog`, armed from when it's opened.
pub struct Watchdog {
    file: File,
    timeout_s: Option<u32>,
}

impl Watchdog {
    /// Opens it and asks for `timeout_s`. A driver that won't set it keeps
    /// its own: that's no reason to stop, with the watchdog already armed.
    pub fn open(path: &Path, timeout_s: u32) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut timeout = timeout_s as libc::c_int;
        // SAFETY: the fd is open for the call and the kernel reads and writes
        // one int through the pointer, which points at a live, mutable one.
        let result = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                WDIOC_SETTIMEOUT,
                &mut timeout as *mut libc::c_int,
            )
        };
        Ok(Self {
            file,
            timeout_s: (result >= 0 && timeout > 0).then_some(timeout as u32),
        })
    }

    /// How often to feed it: a third of what the driver took, else every
    /// second, whatever its own timeout is.
    pub fn feed_every(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.timeout_s
                .map_or(1, |timeout| u64::from((timeout / 3).max(1))),
        )
    }

    /// The longest it can take to fire.
    pub fn fires_within(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.timeout_s.unwrap_or(120)) + 30)
    }

    pub fn feed(&mut self) -> Result<(), String> {
        self.file
            .write_all(b"\0")
            .map_err(|error| format!("feeding the watchdog: {error}"))
    }

    /// Stands it down: the magic close (a kernel built with
    /// `WATCHDOG_NOWAYOUT` won't, and needs feeding for good).
    pub fn disarm(mut self) -> Result<(), String> {
        self.file
            .write_all(b"V")
            .map_err(|error| format!("disarming the watchdog: {error}"))
    }
}
