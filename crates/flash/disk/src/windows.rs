//! Windows: physical drives and volumes through `DeviceIoControl`, every volume
//! of a disk locked and dismounted before the raw write, the raw write through
//! std's file API with write-through and whole-sector I/O, then a flush, a
//! re-read of the disk and an eject.
//!
//! UNPROVEN: nothing here has run on Windows. The write path is refused on
//! Windows until the host test in `docs/host-tests/disk-writer.md` has passed
//! and `WINDOWS_PROOF` is flipped in `platform.rs`. This file is type-checked
//! with `cargo check --target x86_64-pc-windows-gnu`; it is not compiled on
//! Windows by this change.

use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::{
    BusTypeUsb, CreateFileW, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumePathNamesForVolumeNameW,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DISK_EXTENT, DISK_GEOMETRY_EX, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_UPDATE_PROPERTIES, IOCTL_STORAGE_EJECT_MEDIA,
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, StorageDeviceProperty, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::platform::{Guard, Platform, Proof, RawDisk, WINDOWS_PROOF};
use crate::sector::{Positional, Sectored};
use crate::{Disk, DiskError, DiskSource};

/// Physical drive numbers looked at. A PC has a few; the rest are absent.
const MAX_DRIVES: u32 = 64;
const DRIVE_PREFIX: &str = r"\\.\PhysicalDrive";

/// The platform of Windows: `\\.\PhysicalDriveN`, `FSCTL_LOCK_VOLUME` and
/// `FSCTL_DISMOUNT_VOLUME` for each volume, administrator rights to write.
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsPlatform;

/// Closes a handle when dropped.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateFileW or OpenProcessToken and is
        // closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(units: &[u16]) -> String {
    let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Opens a disk or volume with no access rights: enough to query it, and
/// listing needs no administrator.
fn open_query(path: &str) -> Option<Handle> {
    let name = wide(path);
    // SAFETY: `name` is NUL-terminated and lives across the call. The security
    // attributes and template handle are optional and passed as null.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
}

/// Sends one control code. The output is 8-byte-aligned storage, so the
/// structures read from it are aligned. Returns the bytes the device filled in.
fn control(handle: HANDLE, code: u32, input: &[u8], output: &mut [u64]) -> io::Result<usize> {
    let mut returned = 0u32;
    // SAFETY: each buffer is valid for the length passed for it. No overlapped
    // I/O is requested, so the last argument is null.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            output.as_mut_ptr().cast(),
            size_of_val(output) as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(returned as usize)
    }
}

fn bytes_of<T>(value: &T) -> &[u8] {
    // SAFETY: the bytes of a value of type T, for the value's whole size, while
    // the reference lives.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn as_bytes(words: &[u64]) -> &[u8] {
    // SAFETY: a u64 slice viewed as its bytes, for its whole length.
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), size_of_val(words)) }
}

fn read_struct<T: Copy>(words: &[u64]) -> T {
    assert!(
        size_of::<T>() <= size_of_val(words),
        "the output buffer is too small"
    );
    // SAFETY: the buffer holds at least size_of::<T>() bytes (checked above), and
    // read_unaligned accepts any alignment. T is a Windows structure: plain data.
    unsafe { std::ptr::read_unaligned(words.as_ptr().cast::<T>()) }
}

/// A NUL-terminated ASCII string at `offset` in a descriptor, if there is one.
fn string_at(bytes: &[u8], offset: u32) -> Option<String> {
    let start = offset as usize;
    if start == 0 || start >= bytes.len() {
        return None;
    }
    let end = bytes[start..]
        .iter()
        .position(|b| *b == 0)
        .map_or(bytes.len(), |p| start + p);
    let text = String::from_utf8_lossy(&bytes[start..end])
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

fn disk_number(disk: &Disk) -> Result<u32, DiskError> {
    disk.path
        .display()
        .to_string()
        .strip_prefix(DRIVE_PREFIX)
        .and_then(|digits| digits.parse().ok())
        .ok_or_else(|| {
            DiskError::Unsafe(format!(
                "{} is not a physical drive path ({DRIVE_PREFIX}N)",
                disk.path.display()
            ))
        })
}

fn drive_path(number: u32) -> String {
    format!("{DRIVE_PREFIX}{number}")
}

/// Fields of a disk the write needs.
struct Geometry {
    size_bytes: u64,
    sector: u32,
}

fn geometry(handle: HANDLE) -> io::Result<Geometry> {
    let mut out = [0u64; 16];
    control(handle, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut out)?;
    let info: DISK_GEOMETRY_EX = read_struct(&out);
    Ok(Geometry {
        size_bytes: u64::try_from(info.DiskSize).unwrap_or(0),
        sector: info.Geometry.BytesPerSector,
    })
}

/// The descriptor of a disk: removable flag, bus, and its strings.
fn descriptor(handle: HANDLE) -> io::Result<(STORAGE_DEVICE_DESCRIPTOR, Vec<u8>)> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut out = [0u64; 128];
    let filled = control(
        handle,
        IOCTL_STORAGE_QUERY_PROPERTY,
        bytes_of(&query),
        &mut out,
    )?;
    let desc: STORAGE_DEVICE_DESCRIPTOR = read_struct(&out);
    Ok((desc, as_bytes(&out)[..filled.min(1024)].to_vec()))
}

/// A volume, its mount paths, and the physical disks its extents are on.
struct Volume {
    /// `\\?\Volume{GUID}\`, with the trailing backslash.
    guid: String,
    mounts: Vec<String>,
    disks: Vec<u32>,
}

fn mount_paths(guid: &str) -> Vec<String> {
    let name = wide(guid);
    let mut buffer = vec![0u16; 1024];
    let mut returned = 0u32;
    // SAFETY: `name` is NUL-terminated; `buffer` and `returned` are valid for
    // the lengths passed.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            name.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return Vec::new();
    }
    // The answer is a list of NUL-terminated paths, ended by an empty one.
    buffer
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

fn volume_disks(guid: &str) -> Vec<u32> {
    let Some(handle) = open_query(guid.trim_end_matches('\\')) else {
        return Vec::new();
    };
    let mut out = [0u64; 64];
    let Ok(filled) = control(
        handle.0,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        &mut out,
    ) else {
        return Vec::new();
    };
    let bytes = &as_bytes(&out)[..filled.min(out.len() * 8)];
    let count_end = size_of::<u32>();
    if bytes.len() < count_end {
        return Vec::new();
    }
    let count = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let first = offset_of!(VOLUME_DISK_EXTENTS, Extents);
    let mut disks = Vec::new();
    for index in 0..count {
        let at = first + index * size_of::<DISK_EXTENT>();
        if at + size_of::<DISK_EXTENT>() > bytes.len() {
            break;
        }
        let extent: DISK_EXTENT = read_at(bytes, at);
        disks.push(extent.DiskNumber);
    }
    disks
}

/// Reads one `T` at byte `at` of `bytes`, unaligned. The caller checks the
/// range first.
fn read_at<T: Copy>(bytes: &[u8], at: usize) -> T {
    assert!(
        at + size_of::<T>() <= bytes.len(),
        "the read is past the output"
    );
    // SAFETY: the range is inside `bytes` (asserted above), and read_unaligned
    // accepts any alignment. T is a Windows structure: plain data.
    unsafe { std::ptr::read_unaligned(bytes[at..].as_ptr().cast::<T>()) }
}

/// Every volume of this computer, with its mounts and disks.
fn volumes() -> Result<Vec<Volume>, DiskError> {
    let mut found = Vec::new();
    let mut name = [0u16; 512];
    // SAFETY: `name` holds the 512 UTF-16 units the length passed names.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return Err(DiskError::List(format!(
            "volumes: {}",
            io::Error::last_os_error()
        )));
    }
    loop {
        let guid = from_wide(&name);
        let mounts = mount_paths(&guid);
        let disks = volume_disks(&guid);
        found.push(Volume {
            guid,
            mounts,
            disks,
        });
        // SAFETY: as for FindFirstVolumeW above; the search handle is still open.
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    // SAFETY: `find` is the handle FindFirstVolumeW returned, closed once.
    unsafe { FindVolumeClose(find) };
    Ok(found)
}

fn list_physical() -> Result<Vec<Disk>, DiskError> {
    let volumes = volumes()?;
    let system_drive = std::env::var("SystemDrive")
        .ok()
        .map(|drive| format!("{}\\", drive.trim_end_matches('\\')));
    let mut disks = Vec::new();
    for number in 0..MAX_DRIVES {
        let path = drive_path(number);
        let Some(handle) = open_query(&path) else {
            continue;
        };
        let (desc, strings) =
            descriptor(handle.0).map_err(|error| DiskError::List(format!("{path}: {error}")))?;
        let info =
            geometry(handle.0).map_err(|error| DiskError::List(format!("{path}: {error}")))?;
        let mut mount_points: Vec<String> = volumes
            .iter()
            .filter(|volume| volume.disks.contains(&number))
            .flat_map(|volume| volume.mounts.iter().cloned())
            .collect();
        mount_points.sort();
        mount_points.dedup();
        let system = system_drive.as_ref().is_some_and(|drive| {
            mount_points
                .iter()
                .any(|point| point.eq_ignore_ascii_case(drive))
        });
        disks.push(Disk {
            path: PathBuf::from(&path),
            name: format!("PhysicalDrive{number}"),
            size_bytes: info.size_bytes,
            vendor: string_at(&strings, desc.VendorIdOffset),
            model: string_at(&strings, desc.ProductIdOffset),
            serial: string_at(&strings, desc.SerialNumberOffset),
            removable: desc.RemovableMedia,
            usb: desc.BusType == BusTypeUsb,
            usb_port: None,
            system,
            mount_points,
        });
    }
    Ok(disks)
}

/// The volumes of one disk, locked and dismounted for as long as this lives.
/// Closing their handles releases the locks.
struct Locked {
    _held: Vec<File>,
}

fn lock_volumes(number: u32) -> Result<Locked, DiskError> {
    let mut held = Vec::new();
    for volume in volumes()?
        .into_iter()
        .filter(|volume| volume.disks.contains(&number))
    {
        let path = volume.guid.trim_end_matches('\\').to_string();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .map_err(|error| DiskError::Busy(path.clone(), error.to_string()))?;
        let handle = file.as_raw_handle() as HANDLE;
        control(handle, FSCTL_LOCK_VOLUME, &[], &mut [])
            .map_err(|error| DiskError::Busy(path.clone(), format!("lock: {error}")))?;
        control(handle, FSCTL_DISMOUNT_VOLUME, &[], &mut [])
            .map_err(|error| DiskError::Busy(path.clone(), format!("dismount: {error}")))?;
        held.push(file);
    }
    Ok(Locked { _held: held })
}

/// Whole-sector positioned I/O on the physical drive. `seek_write` and
/// `seek_read` may move fewer bytes than asked, so both loop.
impl Positional for File {
    fn pwrite(&mut self, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let moved = self.seek_write(data, offset)?;
            if moved == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            offset += moved as u64;
            data = &data[moved..];
        }
        Ok(())
    }

    fn pread(&mut self, mut offset: u64, mut buf: &mut [u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let moved = self.seek_read(buf, offset)?;
            if moved == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            offset += moved as u64;
            buf = &mut buf[moved..];
        }
        Ok(())
    }

    fn fsync(&mut self) -> io::Result<()> {
        // FlushFileBuffers on the physical drive handle.
        self.sync_all()
    }
}

fn open_raw(number: u32) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_WRITE_THROUGH)
        .open(drive_path(number))
}

/// Whether this process holds an elevated token (administrator rights).
fn elevated() -> io::Result<bool> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the process pseudo-handle needs no closing; `token` receives a
    // handle that `Handle` closes below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

impl DiskSource for WindowsPlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        list_physical()
    }
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "Windows"
    }

    fn proof(&self) -> Proof {
        WINDOWS_PROOF
    }

    fn may_write(&self) -> Result<(), DiskError> {
        match elevated() {
            Ok(true) => Ok(()),
            Ok(false) => Err(DiskError::Unsafe(
                "writing a disk on Windows needs administrator rights: the helper must run elevated (approve the User Account Control prompt)".into(),
            )),
            Err(error) => Err(DiskError::Unsafe(format!(
                "could not check for administrator rights: {error}"
            ))),
        }
    }

    fn prepare(&self, disk: &Disk) -> Result<Guard, DiskError> {
        let number = disk_number(disk)?;
        Ok(Box::new(lock_volumes(number)?))
    }

    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        let number = disk_number(disk)?;
        let handle = open_query(&drive_path(number))
            .ok_or_else(|| DiskError::NotFound(disk.path.display().to_string()))?;
        let sector = geometry(handle.0)
            .map_err(|error| DiskError::io(&disk.path, error))?
            .sector;
        drop(handle);
        let file = open_raw(number).map_err(|error| DiskError::io(&disk.path, error))?;
        let sectored = Sectored::new(file, sector as usize)
            .map_err(|error| DiskError::io(&disk.path, error))?;
        Ok(Box::new(sectored))
    }

    fn finish(&self, disk: &Disk, mut raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        raw.sync()
            .map_err(|error| DiskError::io(&disk.path, error))?;
        drop(raw);
        let number = disk_number(disk)?;
        let handle = open_query(&drive_path(number))
            .ok_or_else(|| DiskError::NotFound(disk.path.display().to_string()))?;
        control(handle.0, IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut [])
            .map_err(|error| DiskError::io(&disk.path, error))?;
        control(handle.0, IOCTL_STORAGE_EJECT_MEDIA, &[], &mut [])
            .map_err(|error| DiskError::io(&disk.path, error))?;
        Ok(())
    }
}
