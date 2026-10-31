//! The one ioctl a slot update needs: `UBI_IOCVOLUP` starts a volume
//! update of N bytes on `/dev/ubiX_Y`; the next N bytes written to it
//! replace the volume's contents (the kernel sets the volume's update marker
//! until the last byte lands, so a cut-short update leaves the volume marked
//! corrupt, never half old and half new).

use std::fs::File;
use std::os::fd::AsRawFd;

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
