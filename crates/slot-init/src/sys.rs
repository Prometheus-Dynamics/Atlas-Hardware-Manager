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

// --- /init on a board ------------------------------------------------------

use std::ffi::CString;
use std::time::Duration;

use crate::boot::Ops;

fn c(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())
}

fn c_str(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|e| e.to_string())
}

fn sys_mount(
    source: &Path,
    target: &Path,
    fstype: Option<&str>,
    flags: libc::c_ulong,
    data: &str,
) -> Result<(), String> {
    let (src, dst, data) = (c(source)?, c(target)?, c_str(data)?);
    let fstype = fstype.map(c_str).transpose()?;
    // SAFETY: NUL-terminated strings that outlive the call; a null fstype
    // is what bind and move mounts take.
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            dst.as_ptr(),
            fstype.as_ref().map_or(std::ptr::null(), |f| f.as_ptr()),
            flags,
            data.as_ptr().cast(),
        )
    };
    if rc < 0 {
        return Err(last_error(&format!(
            "mount {} on {}",
            source.display(),
            target.display()
        )));
    }
    Ok(())
}

/// The kernel's own mounts, and the console as stdin, stdout and stderr:
/// the archive may hold no /proc, /sys or /dev, and no /dev/console, so
/// the kernel started this with no stdio.
pub fn early() {
    for dir in ["/proc", "/sys", "/dev", "/run", "/tmp"] {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = sys_mount(Path::new("proc"), Path::new("/proc"), Some("proc"), 0, "");
    let _ = sys_mount(Path::new("sysfs"), Path::new("/sys"), Some("sysfs"), 0, "");
    let _ = sys_mount(
        Path::new("devtmpfs"),
        Path::new("/dev"),
        Some("devtmpfs"),
        0,
        "",
    );
    if let Ok(console) = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/console")
    {
        for fd in 0..3 {
            // SAFETY: dup2 onto the standard descriptors; `console` stays
            // open until they are set.
            unsafe { libc::dup2(console.as_raw_fd(), fd) };
        }
    }
}

/// The real system, for /init on a board.
pub struct System;

/// The module's file and what it needs first, from modules.dep.
fn module_deps(dep: &str, name: &str) -> Option<(String, Vec<String>)> {
    let want = name.replace('_', "-");
    dep.lines().find_map(|line| {
        let (path, deps) = line.split_once(':')?;
        let base = path.rsplit('/').next()?;
        let stem = base.split(".ko").next()?.replace('_', "-");
        (stem == want).then(|| {
            (
                path.to_owned(),
                deps.split_whitespace().map(str::to_owned).collect(),
            )
        })
    })
}

fn insmod(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let params = c_str("")?;
    let name = path.to_string_lossy();
    // MODULE_INIT_COMPRESSED_FILE: the kernel decompresses .ko.xz/.zst/.gz.
    let flags = if name.ends_with(".ko") { 0 } else { 4 };
    // SAFETY: finit_module(2) with an open module file and empty params.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_finit_module,
            file.as_raw_fd(),
            params.as_ptr(),
            flags,
        )
    };
    if rc < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
        return Err(last_error(&format!("loading {}", path.display())));
    }
    Ok(())
}

impl Ops for System {
    fn mount(&self, fstype: &str, opts: &str, source: &Path, target: &Path) -> Result<(), String> {
        let mut flags = 0;
        let mut data = Vec::new();
        for opt in opts.split(',').filter(|o| !o.is_empty()) {
            match opt {
                "ro" => flags |= libc::MS_RDONLY,
                "rw" => {}
                "noatime" => flags |= libc::MS_NOATIME,
                other => data.push(other),
            }
        }
        sys_mount(source, target, Some(fstype), flags, &data.join(","))
    }

    fn bind(&self, source: &Path, target: &Path) -> Result<(), String> {
        sys_mount(source, target, None, libc::MS_BIND, "")
    }

    fn move_mount(&self, source: &Path, target: &Path) -> Result<(), String> {
        sys_mount(source, target, None, libc::MS_MOVE, "")
    }

    fn umount(&self, target: &Path) {
        if let Ok(path) = c(target) {
            // SAFETY: a NUL-terminated path.
            unsafe { libc::umount2(path.as_ptr(), 0) };
        }
    }

    fn load_module(&self, name: &str, release: &str) -> Result<(), String> {
        // /lib may be a link to /usr/lib, or not there at all.
        let base = ["/lib/modules", "/usr/lib/modules"]
            .iter()
            .map(|dir| Path::new(dir).join(release))
            .find(|dir| dir.join("modules.dep").is_file())
            .ok_or_else(|| {
                format!("no modules.dep for {release} under /lib/modules or /usr/lib/modules")
            })?;
        let dep = std::fs::read_to_string(base.join("modules.dep"))
            .map_err(|e| format!("{}/modules.dep: {e}", base.display()))?;
        let (path, deps) = module_deps(&dep, name)
            .ok_or_else(|| format!("not in {}/modules.dep (built in?)", base.display()))?;
        // modules.dep lists the deepest dependency last.
        for d in deps.iter().rev() {
            insmod(&base.join(d))?;
        }
        insmod(&base.join(path))
    }

    fn open_verity(
        &self,
        image: &Path,
        name: &str,
        verity: &Verity,
        root: &str,
    ) -> Result<String, String> {
        let root = crate::root_hash(root)?;
        let (_, dev) = open_loop(image)?;
        open_verity(name, &dev, verity, &root)
    }

    fn kernel_log(&self) -> String {
        // SAFETY: SYSLOG_ACTION_SIZE_BUFFER takes no buffer.
        let size = unsafe { libc::klogctl(10, std::ptr::null_mut(), 0) }.max(16384);
        let mut buf = vec![0u8; size as usize];
        // SAFETY: SYSLOG_ACTION_READ_ALL into a buffer of `size` bytes.
        let read = unsafe { libc::klogctl(3, buf.as_mut_ptr().cast(), size) };
        buf.truncate(read.max(0) as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn persist(&self, path: &Path, text: &str) -> Result<(), String> {
        crate::boot::persist_file(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn sync(&self) {
        // SAFETY: sync(2) takes nothing.
        unsafe { libc::sync() };
    }

    fn reboot(&self) {
        self.sync();
        // SAFETY: reboot(2) as PID 1, the filesystems synced.
        unsafe { libc::reboot(libc::RB_AUTOBOOT) };
    }

    fn reboot_tryboot(&self) {
        self.sync();
        let _ = reboot_tryboot();
    }

    fn switch_root(&self, new_root: &Path) -> Result<(), String> {
        Err(self.switch_root_inner(new_root))
    }

    fn sleep(&self, time: Duration) {
        std::thread::sleep(time);
    }
}

impl System {
    fn switch_root_inner(&self, new_root: &Path) -> String {
        use std::os::unix::process::CommandExt;
        self.umount(Path::new("/sys"));
        self.umount(Path::new("/proc"));
        if let Err(error) = self.move_mount(Path::new("/dev"), &new_root.join("dev")) {
            return error;
        }
        if let Err(error) = std::env::set_current_dir(new_root) {
            return format!("{}: {error}", new_root.display());
        }
        // Free the initramfs (its files are in RAM): everything on "/"
        // but the new root, which is another filesystem.
        if let Ok(meta) = std::fs::metadata("/") {
            remove_on_device(Path::new("/"), meta.dev());
        }
        if let Err(error) = sys_mount(Path::new("."), Path::new("/"), None, libc::MS_MOVE, "") {
            return error;
        }
        if let Err(error) = std::os::unix::fs::chroot(".") {
            return format!("chroot: {error}");
        }
        let _ = std::env::set_current_dir("/");
        let error = std::process::Command::new("/sbin/init").exec();
        format!("/sbin/init: {error}")
    }
}

/// Removes what is under `dir` on the device `dev`, never crossing into
/// another filesystem or following a link.
fn remove_on_device(dir: &Path, dev: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.dev() != dev {
            continue;
        }
        if meta.is_dir() {
            remove_on_device(&path, dev);
            let _ = std::fs::remove_dir(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_dep_lists_the_dependencies() {
        let dep = "kernel/drivers/md/dm-mod.ko:\nkernel/drivers/md/dm-bufio.ko:\nkernel/drivers/md/dm-verity.ko: kernel/drivers/md/dm-bufio.ko kernel/drivers/md/dm-mod.ko\n";
        let (path, deps) = module_deps(dep, "dm-verity").unwrap();
        assert_eq!(path, "kernel/drivers/md/dm-verity.ko");
        assert_eq!(
            deps,
            [
                "kernel/drivers/md/dm-bufio.ko",
                "kernel/drivers/md/dm-mod.ko"
            ]
        );
        assert_eq!(
            module_deps(dep, "dm_mod").unwrap().0,
            "kernel/drivers/md/dm-mod.ko"
        );
        assert!(module_deps(dep, "nope").is_none());
    }

    #[test]
    fn the_structs_have_the_kernels_sizes() {
        assert_eq!(size_of::<DmIoctl>(), 312);
        assert_eq!(size_of::<DmTargetSpec>(), 40);
        // DM_DEV_CREATE as the kernel defines it.
        assert_eq!(dm_cmd(DM_DEV_CREATE) as u32, 0xc138_fd03);
    }
}
