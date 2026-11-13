//! macOS: `diskutil` lists disks, unmounts a disk's volumes and ejects it. The
//! raw device `/dev/rdiskN` cannot be opened by an unprivileged app, so the
//! system's authorisation helper, `authopen`, opens it with the user's consent
//! and hands the open file back over a socket pair (SCM_RIGHTS). Writes use
//! `F_NOCACHE` and whole-sector I/O; the flush is `F_FULLFSYNC`.
//!
//! UNPROVEN: nothing here has run on macOS. The write path is refused on macOS
//! until the host test in `docs/host-tests/disk-writer.md` has passed and
//! `MACOS_PROOF` is flipped in `platform.rs`. This file is type-checked with
//! `cargo check --target x86_64-apple-darwin`; it is not run on macOS by this
//! change. The `authopen` arguments (`-stdoutpipe -o 2`) follow its man page as
//! read by the author; the host test confirms them.

use std::fs::File;
use std::io::{self, IoSliceMut};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use nix::sys::socket::{ControlMessageOwned, MsgFlags, recvmsg};
use plist::{Dictionary, Value};

use crate::platform::{Guard, MACOS_PROOF, Platform, Proof, RawDisk};
use crate::sector::{Positional, Sectored};
use crate::{Disk, DiskError, DiskSource, is_system_mount};

const DISKUTIL: &str = "/usr/sbin/diskutil";
const AUTHOPEN: &str = "/usr/libexec/authopen";

/// The platform of macOS: `/dev/diskN` to list and unmount, `/dev/rdiskN` to
/// write, `authopen` for the privilege.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacPlatform;

fn diskutil(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(DISKUTIL)
        .args(args)
        .output()
        .map_err(|error| format!("diskutil: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn parse(bytes: &[u8]) -> Result<Value, DiskError> {
    plist::from_bytes(bytes).map_err(|error| DiskError::List(error.to_string()))
}

/// `diskutil info -plist` for one device.
fn info(device: &str) -> Result<Dictionary, DiskError> {
    let bytes = diskutil(&["info", "-plist", device]).map_err(DiskError::List)?;
    parse(&bytes)?
        .as_dictionary()
        .cloned()
        .ok_or_else(|| DiskError::List(format!("diskutil info {device} is not a dictionary")))
}

fn text(dict: &Dictionary, key: &str) -> Option<String> {
    dict.get(key)?.as_string().map(str::to_string)
}

fn flag(dict: &Dictionary, key: &str) -> bool {
    dict.get(key).and_then(Value::as_boolean).unwrap_or(false)
}

fn number(dict: &Dictionary, key: &str) -> Option<u64> {
    dict.get(key).and_then(Value::as_unsigned_integer)
}

/// Every `MountPoint` under an entry of `diskutil list`, partitions and APFS
/// volumes included.
fn collect_mounts(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Dictionary(dict) => {
            for (key, child) in dict.iter() {
                if key == "MountPoint" {
                    if let Some(point) = child.as_string().filter(|point| !point.is_empty()) {
                        out.push(point.to_string());
                    }
                } else {
                    collect_mounts(child, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_mounts(item, out);
            }
        }
        _ => {}
    }
}

/// `diskN`, a whole disk: never a slice (`diskNsM`) or a raw node.
fn whole_name(name: &str) -> bool {
    name.strip_prefix("disk")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The `/dev/diskN` node of a disk, checked again before any command runs.
fn device_of(disk: &Disk) -> Result<String, DiskError> {
    let path = disk.path.display().to_string();
    match path.strip_prefix("/dev/") {
        Some(name) if whole_name(name) => Ok(path),
        _ => Err(DiskError::Unsafe(format!(
            "{path} is not a whole disk node (/dev/diskN)"
        ))),
    }
}

fn list_macos() -> Result<Vec<Disk>, DiskError> {
    let bytes = diskutil(&["list", "-plist"]).map_err(DiskError::List)?;
    let root = parse(&bytes)?;
    let all = root
        .as_dictionary()
        .and_then(|dict| dict.get("AllDisksAndPartitions"))
        .and_then(Value::as_array)
        .ok_or_else(|| DiskError::List("diskutil list has no disk list".into()))?;
    let mut disks = Vec::new();
    for entry in all {
        let Some(name) = entry
            .as_dictionary()
            .and_then(|dict| text(dict, "DeviceIdentifier"))
        else {
            continue;
        };
        if !whole_name(&name) {
            continue;
        }
        let props = info(&format!("/dev/{name}"))?;
        // Whole physical disks only: a container, a virtual disk or a slice is not one.
        if !flag(&props, "WholeDisk")
            || text(&props, "VirtualOrPhysical").as_deref() != Some("Physical")
        {
            continue;
        }
        let mut mount_points = Vec::new();
        collect_mounts(entry, &mut mount_points);
        mount_points.sort();
        mount_points.dedup();
        let system = mount_points
            .iter()
            .any(|point| is_system_mount(point) || point.starts_with("/System/Volumes/"));
        disks.push(Disk {
            path: PathBuf::from(format!("/dev/{name}")),
            size_bytes: number(&props, "TotalSize").unwrap_or(0),
            vendor: None,
            model: text(&props, "MediaName").or_else(|| text(&props, "IORegistryEntryName")),
            serial: None,
            removable: flag(&props, "RemovableMedia"),
            usb: text(&props, "BusProtocol").as_deref() == Some("USB"),
            usb_port: None,
            system,
            mount_points,
            name,
        });
    }
    disks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(disks)
}

/// Asks `authopen` to open `path` read-write, and receives the open file over
/// a socket pair. `authopen` shows the system's authorisation prompt.
fn authopen(path: &str) -> io::Result<File> {
    let (parent, child) = UnixStream::pair()?;
    let mut helper = Command::new(AUTHOPEN)
        .args(["-stdoutpipe", "-o", "2", path])
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(child)))
        .stderr(Stdio::null())
        .spawn()?;
    let mut control = nix::cmsg_space!([RawFd; 1]);
    let mut byte = [0u8; 1];
    let received = {
        let mut iov = [IoSliceMut::new(&mut byte)];
        let message = recvmsg::<()>(
            parent.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::empty(),
        )
        .map_err(io::Error::from)?;
        let mut fd = None;
        for part in message.cmsgs().map_err(io::Error::from)? {
            if let ControlMessageOwned::ScmRights(fds) = part {
                fd = fds.first().copied();
            }
        }
        fd
    };
    let status = helper.wait()?;
    let fd = received
        .ok_or_else(|| io::Error::other(format!("authopen did not open {path} (exit {status})")))?;
    // SAFETY: `fd` was received over SCM_RIGHTS just now; nothing else owns it,
    // so the File takes sole ownership and closes it once.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Sets a flag on the open raw device with `fcntl`.
fn fcntl_on(file: &File, command: i32, argument: i32) -> io::Result<()> {
    // SAFETY: the descriptor is the open file's own, valid while `file` lives.
    // These commands take an integer argument (or none) and touch no memory.
    let result = unsafe { nix::libc::fcntl(file.as_raw_fd(), command, argument) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Positional for File {
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.write_all_at(data, offset)
    }

    fn pread(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.read_exact_at(buf, offset)
    }

    fn fsync(&mut self) -> io::Result<()> {
        // F_FULLFSYNC asks the drive to flush its own buffers too. It is
        // strict: a failure is an error, not a fallback to fsync.
        fcntl_on(self, nix::libc::F_FULLFSYNC, 0)
    }
}

impl DiskSource for MacPlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        list_macos()
    }
}

impl Platform for MacPlatform {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn proof(&self) -> Proof {
        MACOS_PROOF
    }

    fn prepare(&self, disk: &Disk) -> Result<Guard, DiskError> {
        let device = device_of(disk)?;
        diskutil(&["unmountDisk", &device])
            .map_err(|error| DiskError::Busy(disk.name.clone(), error))?;
        Ok(Box::new(()))
    }

    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        let device = device_of(disk)?;
        let raw = device.replacen("/dev/disk", "/dev/rdisk", 1);
        let props = info(&device)?;
        let sector = number(&props, "DeviceBlockSize")
            .filter(|size| *size > 0)
            .unwrap_or(512);
        let file = authopen(&raw).map_err(|error| DiskError::io(&raw, error))?;
        fcntl_on(&file, nix::libc::F_NOCACHE, 1).map_err(|error| DiskError::io(&raw, error))?;
        let sectored =
            Sectored::new(file, sector as usize).map_err(|error| DiskError::io(&raw, error))?;
        Ok(Box::new(sectored))
    }

    fn finish(&self, disk: &Disk, mut raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        let device = device_of(disk)?;
        raw.sync()
            .map_err(|error| DiskError::io(&disk.path, error))?;
        drop(raw);
        diskutil(&["eject", &device]).map_err(|error| DiskError::Io {
            path: disk.path.clone(),
            message: error,
        })?;
        Ok(())
    }
}
