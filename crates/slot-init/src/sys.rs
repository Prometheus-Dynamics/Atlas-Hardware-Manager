//! The ioctls: a free loop device bound read-only to the image, then a
//! device-mapper device with one verity target over it, resumed. With
//! devtmpfs mounted on /dev, both nodes appear by themselves
//! (/dev/loopN, /dev/dm-N).

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::{Verity, decode_dev, verity_table};

// The ioctl request's C type differs between glibc (unsigned long) and musl (int).
const LOOP_CTL_GET_FREE: libc::Ioctl = 0x4C82;
const LOOP_SET_FD: libc::Ioctl = 0x4C00;

/// `struct dm_ioctl` (device-mapper interface 4), 312 bytes.
#[repr(C)]
struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: i32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    dev: u64,
    name: [u8; 128],
    uuid: [u8; 129],
    data: [u8; 7],
}

/// `struct dm_target_spec`, 40 bytes, then its parameter string.
#[repr(C)]
struct DmTargetSpec {
    sector_start: u64,
    length: u64,
    status: i32,
    next: u32,
    target_type: [u8; 16],
}

const DM_READONLY_FLAG: u32 = 1;
const DM_DEV_CREATE: u8 = 3;
const DM_DEV_SUSPEND: u8 = 6;
const DM_TABLE_LOAD: u8 = 9;

const fn dm_cmd(nr: u8) -> libc::Ioctl {
    // _IOWR(0xfd, nr, struct dm_ioctl)
    ((3u32 << 30) | ((size_of::<DmIoctl>() as u32) << 16) | (0xfd << 8) | nr as u32) as libc::Ioctl
}

fn last_error(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

/// Binds a free loop device read-only to `image`; its path and `major:minor`.
pub fn open_loop(image: &Path) -> Result<(String, String), String> {
    let file = File::open(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let control = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/loop-control")
        .map_err(|e| format!("/dev/loop-control: {e}"))?;
    // SAFETY: LOOP_CTL_GET_FREE takes no argument and returns a number.
    let n = unsafe { libc::ioctl(control.as_raw_fd(), LOOP_CTL_GET_FREE) };
    if n < 0 {
        return Err(last_error("no free loop device"));
    }
    let path = format!("/dev/loop{n}");
    let device = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))?;
    // SAFETY: LOOP_SET_FD takes the backing file's descriptor; a read-only
    // descriptor makes the loop device read-only.
    if unsafe { libc::ioctl(device.as_raw_fd(), LOOP_SET_FD, file.as_raw_fd()) } < 0 {
        return Err(last_error(&path));
    }
    let rdev = device
        .metadata()
        .map_err(|e| format!("{path}: {e}"))?
        .rdev();
    Ok((path, format!("{}:{}", libc::major(rdev), libc::minor(rdev))))
}

fn header(name: &str, flags: u32, data_size: usize) -> Result<DmIoctl, String> {
    if name.len() >= 128 {
        return Err("the dm name is too long".into());
    }
    let mut io = DmIoctl {
        version: [4, 0, 0],
        data_size: data_size as u32,
        data_start: size_of::<DmIoctl>() as u32,
        target_count: 0,
        open_count: 0,
        flags,
        event_nr: 0,
        padding: 0,
        dev: 0,
        name: [0; 128],
        uuid: [0; 129],
        data: [0; 7],
    };
    io.name[..name.len()].copy_from_slice(name.as_bytes());
    Ok(io)
}

fn dm(control: &File, nr: u8, buf: &mut [u8], what: &str) -> Result<(), String> {
    // SAFETY: `buf` starts with a dm_ioctl whose data_size is buf's length.
    if unsafe { libc::ioctl(control.as_raw_fd(), dm_cmd(nr), buf.as_mut_ptr()) } < 0 {
        return Err(last_error(what));
    }
    Ok(())
}

fn bytes_of<T>(value: &T) -> &[u8] {
    // SAFETY: plain repr(C) data, read as bytes.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

/// Creates the read-only verity device `name` over `loop_dev` (`major:minor`)
/// and resumes it; its node, /dev/dm-N.
pub fn open_verity(
    name: &str,
    loop_dev: &str,
    verity: &Verity,
    root: &str,
) -> Result<String, String> {
    let control = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/mapper/control")
        .map_err(|e| format!("/dev/mapper/control: {e} (dm-mod loaded?)"))?;

    let mut buf = vec![0u8; size_of::<DmIoctl>() + 16384];
    let len = buf.len();
    let io = header(name, DM_READONLY_FLAG, len)?;
    buf[..size_of::<DmIoctl>()].copy_from_slice(bytes_of(&io));
    dm(&control, DM_DEV_CREATE, &mut buf, "creating the dm device")?;
    // SAFETY: the kernel filled in the dm_ioctl at the start of `buf`.
    let created: DmIoctl = unsafe { std::ptr::read_unaligned(buf.as_ptr().cast()) };
    let (_, minor) = decode_dev(created.dev);

    let (sectors, params) = verity_table(verity, loop_dev, root);
    let mut table = Vec::new();
    let spec_len = size_of::<DmTargetSpec>() + params.len() + 1;
    let padded = spec_len.div_ceil(8) * 8;
    let mut io = header(name, DM_READONLY_FLAG, size_of::<DmIoctl>() + padded)?;
    io.target_count = 1;
    let mut target_type = [0u8; 16];
    target_type[..6].copy_from_slice(b"verity");
    let spec = DmTargetSpec {
        sector_start: 0,
        length: sectors,
        status: 0,
        next: 0,
        target_type,
    };
    table.extend_from_slice(bytes_of(&io));
    table.extend_from_slice(bytes_of(&spec));
    table.extend_from_slice(params.as_bytes());
    table.resize(size_of::<DmIoctl>() + padded, 0);
    dm(
        &control,
        DM_TABLE_LOAD,
        &mut table,
        "loading the verity table",
    )?;

    // DM_DEV_SUSPEND without DM_SUSPEND_FLAG resumes: the table goes live.
    let io = header(name, DM_READONLY_FLAG, size_of::<DmIoctl>())?;
    let mut resume = bytes_of(&io).to_vec();
    dm(
        &control,
        DM_DEV_SUSPEND,
        &mut resume,
        "resuming the verity device",
    )?;
    Ok(format!("/dev/dm-{minor}"))
}

/// Restarts with the firmware's one-time "0 tryboot": the next boot reads
/// tryboot.txt. Doesn't return when it works.
pub fn reboot_tryboot() -> Result<(), String> {
    let arg = std::ffi::CString::new("0 tryboot").map_err(|e| e.to_string())?;
    // SAFETY: reboot(2) with LINUX_REBOOT_CMD_RESTART2 and a NUL-terminated
    // argument; the caller synced the filesystems first.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_reboot,
            libc::LINUX_REBOOT_MAGIC1,
            libc::LINUX_REBOOT_MAGIC2,
            libc::LINUX_REBOOT_CMD_RESTART2,
            arg.as_ptr(),
        )
    };
    if rc < 0 {
        return Err(last_error("reboot"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structs_have_the_kernels_sizes() {
        assert_eq!(size_of::<DmIoctl>(), 312);
        assert_eq!(size_of::<DmTargetSpec>(), 40);
        // DM_DEV_CREATE as the kernel defines it.
        assert_eq!(dm_cmd(DM_DEV_CREATE) as u32, 0xc138_fd03);
    }
}
