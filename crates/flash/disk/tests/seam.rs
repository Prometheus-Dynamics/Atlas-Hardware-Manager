//! The write pipeline against a fake platform. Every "disk" is a regular file in
//! the temporary directory, and the fake platform records the order of every
//! call, so the tests can see what the pipeline did and in what sequence.
//! Nothing here opens a block device.

mod common;

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use atlas_disk::{
    Acknowledgement, Disk, DiskError, DiskSource, Guard, Platform, Proof, RawDisk, ResumePolicy,
    WriteOptions, WriteProgress, WriteReport, WriteRequest, write_disk,
};
use common::{sha256_hex, temp};

type Log = Arc<Mutex<Vec<String>>>;

/// The same shape `DiskError::io` makes inside the crate.
fn io_error(path: impl Into<PathBuf>, error: io::Error) -> DiskError {
    DiskError::Io {
        path: path.into(),
        message: error.to_string(),
    }
}

fn note(log: &Log, event: impl Into<String>) {
    log.lock().unwrap().push(event.into());
}

/// Records "drop-guard" when the volumes come back.
struct GuardLog(Log);

impl Drop for GuardLog {
    fn drop(&mut self) {
        note(&self.0, "drop-guard");
    }
}

/// A raw disk backed by a file. Its plan can drop it once (a write fails) or
/// cancel the write at a given write.
struct FakeRaw {
    file: File,
    log: Log,
    writes: usize,
    drop_at: Option<usize>,
    cancel: Option<(usize, Arc<AtomicBool>)>,
}

impl RawDisk for FakeRaw {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.writes += 1;
        if self.drop_at == Some(self.writes) {
            self.drop_at = None;
            note(&self.log, "dropped");
            return Err(io::Error::other("the link dropped"));
        }
        note(&self.log, format!("write@{offset}+{}", data.len()));
        if let Some((at, cancel)) = &self.cancel
            && self.writes == *at
        {
            cancel.store(true, Ordering::Relaxed);
        }
        common::write_all_at(&self.file, data, offset)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        note(&self.log, format!("read@{offset}+{}", buf.len()));
        common::read_exact_at(&self.file, buf, offset)
    }

    fn sync(&mut self) -> io::Result<()> {
        note(&self.log, "sync");
        self.file.sync_data()
    }
}

impl Drop for FakeRaw {
    fn drop(&mut self) {
        note(&self.log, "drop-raw");
    }
}

/// The fake system. `backing` is the file that stands in for the disk.
struct FakePlatform {
    log: Log,
    backing: PathBuf,
    proof: Proof,
    listing: Mutex<Vec<Disk>>,
    listing_calls: AtomicUsize,
    /// From this listing call on, the disk is gone (a reattach finds nothing).
    vanish_from_listing: Option<usize>,
    /// At this listing call, the write is cancelled.
    cancel_at_listing: Option<usize>,
    cancel: Arc<AtomicBool>,
    refuse_elevation: bool,
    fail_open: bool,
    /// Drops the first raw device at this write; later devices are whole.
    drop_at_write: Mutex<Option<usize>>,
    /// Cancels the write at this write number, on the first raw device.
    cancel_at_write: Option<usize>,
    fail_finish: bool,
}

impl FakePlatform {
    fn new(backing: PathBuf, disk: Disk) -> Self {
        Self {
            log: Arc::new(Mutex::new(Vec::new())),
            backing,
            proof: Proof::Proven,
            listing: Mutex::new(vec![disk]),
            listing_calls: AtomicUsize::new(0),
            vanish_from_listing: None,
            cancel_at_listing: None,
            cancel: Arc::new(AtomicBool::new(false)),
            refuse_elevation: false,
            fail_open: false,
            drop_at_write: Mutex::new(None),
            cancel_at_write: None,
            fail_finish: false,
        }
    }

    fn events(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<(u64, usize)> {
        self.events()
            .iter()
            .filter_map(|event| {
                let rest = event.strip_prefix("write@")?;
                let (offset, len) = rest.split_once('+')?;
                Some((offset.parse().ok()?, len.parse().ok()?))
            })
            .collect()
    }
}

impl DiskSource for FakePlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        let call = self.listing_calls.fetch_add(1, Ordering::Relaxed) + 1;
        note(&self.log, "list");
        if self.cancel_at_listing == Some(call) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        if self.vanish_from_listing.is_some_and(|from| call >= from) {
            return Ok(Vec::new());
        }
        Ok(self.listing.lock().unwrap().clone())
    }
}

impl Platform for FakePlatform {
    fn name(&self) -> &'static str {
        "Fake"
    }

    fn proof(&self) -> Proof {
        self.proof
    }

    fn may_write(&self) -> Result<(), DiskError> {
        note(&self.log, "may_write");
        if self.refuse_elevation {
            return Err(DiskError::Unsafe("needs administrator rights".into()));
        }
        Ok(())
    }

    fn prepare(&self, _disk: &Disk) -> Result<Guard, DiskError> {
        note(&self.log, "prepare");
        Ok(Box::new(GuardLog(self.log.clone())))
    }

    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        note(&self.log, "open");
        if self.fail_open {
            return Err(io_error(&disk.path, io::Error::other("busy")));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.backing)
            .map_err(|error| io_error(&self.backing, error))?;
        let drop_at = self.drop_at_write.lock().unwrap().take();
        let cancel = self
            .cancel_at_write
            .filter(|_| drop_at.is_none())
            .map(|at| (at, self.cancel.clone()));
        Ok(Box::new(FakeRaw {
            file,
            log: self.log.clone(),
            writes: 0,
            drop_at,
            cancel,
        }))
    }

    fn finish(&self, disk: &Disk, mut raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        note(&self.log, "finish");
        raw.sync().map_err(|error| io_error(&disk.path, error))?;
        drop(raw);
        if self.fail_finish {
            return Err(io_error(&disk.path, io::Error::other("eject refused")));
        }
        Ok(())
    }
}

fn card(size: u64) -> Disk {
    Disk {
        path: PathBuf::from("/dev/fake9"),
        name: "fake9".into(),
        size_bytes: size,
        vendor: Some("Vendor".into()),
        model: Some("Card".into()),
        serial: Some("S1".into()),
        removable: true,
        usb: true,
        usb_port: Some("1-4".into()),
        system: false,
        mount_points: Vec::new(),
    }
}

/// A raw image of `len` bytes: a pattern that never repeats within a sector.
fn raw_image(len: usize) -> Vec<u8> {
    (0..len).map(|n| (n * 7 % 251) as u8 + 1).collect()
}

/// A target file of `size` bytes filled with 0xEE, standing in for a disk.
fn target_file(size: usize) -> PathBuf {
    let path = temp("seam-target");
    fs::write(&path, vec![0xEE; size]).unwrap();
    path
}

fn image_file(bytes: &[u8]) -> PathBuf {
    let path = temp("seam-image.img");
    fs::write(&path, bytes).unwrap();
    path
}

fn policy() -> ResumePolicy {
    ResumePolicy {
        segment_bytes: 4096,
        max_backoff: std::time::Duration::from_millis(1),
    }
}

fn run(
    platform: &Arc<FakePlatform>,
    chosen: &Disk,
    ack: Acknowledgement,
    image: &Path,
    allow_large: bool,
    progress: &mut dyn FnMut(WriteProgress),
) -> Result<WriteReport, DiskError> {
    let options = WriteOptions {
        verify: true,
        image_sha256: None,
    };
    let policy = policy();
    let request = WriteRequest {
        image,
        options: &options,
        policy: &policy,
        journal: None,
        allow_large,
    };
    let cancel = platform.cancel.clone();
    write_disk(platform.clone(), chosen, ack, &request, progress, &cancel)
}

fn position(events: &[String], wanted: &str) -> usize {
    events
        .iter()
        .position(|event| event == wanted)
        .unwrap_or_else(|| panic!("{wanted} is missing from {events:?}"))
}

#[test]
fn the_pipeline_runs_in_order_and_finishes_after_the_last_flush() {
    let raw = raw_image(10_000);
    let image = image_file(&raw);
    let target = target_file(64 * 1024);
    let platform = Arc::new(FakePlatform::new(target.clone(), card(64 * 1024)));
    let report = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.bytes_written, 10_000);
    assert!(report.verified);

    let events = platform.events();
    assert_eq!(&events[..4], ["may_write", "list", "prepare", "open"]);
    assert!(position(&events, "prepare") < position(&events, "open"));
    assert!(
        events.iter().any(|e| e.starts_with("read@")),
        "verify reads back"
    );
    let finish = position(&events, "finish");
    let last_write = events
        .iter()
        .rposition(|e| e.starts_with("write@"))
        .unwrap();
    assert!(
        events[last_write..finish].iter().any(|e| e == "sync"),
        "the last write is flushed before finish"
    );
    assert_eq!(
        &events[finish..],
        ["finish", "sync", "drop-raw", "drop-guard"],
        "finish flushes, drops the raw device, then the volumes come back"
    );
    let on_disk = fs::read(&target).unwrap();
    assert_eq!(&on_disk[..10_000], &raw[..]);
    assert_eq!(sha256_hex(&on_disk[..10_000]), sha256_hex(&raw));
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn an_unproven_platform_refuses_before_anything_is_listed_or_touched() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.proof = Proof::Unproven;
    let platform = Arc::new(platform);
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Withheld,
        &image,
        false,
        &mut |_| {},
    );
    assert!(
        matches!(&result, Err(DiskError::Unsupported(text)) if text.contains("not been proven on this system yet")),
        "{result:?}"
    );
    assert!(platform.events().is_empty(), "{:?}", platform.events());
    assert!(fs::read(&target).unwrap().iter().all(|b| *b == 0xEE));
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn an_unproven_platform_writes_once_the_acknowledgement_is_given() {
    let raw = raw_image(5000);
    let image = image_file(&raw);
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.proof = Proof::Unproven;
    let platform = Arc::new(platform);
    run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(&fs::read(&target).unwrap()[..5000], &raw[..]);
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn without_elevation_nothing_is_listed_or_prepared() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.refuse_elevation = true;
    let platform = Arc::new(platform);
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(matches!(result, Err(DiskError::Unsafe(_))));
    assert_eq!(platform.events(), ["may_write"]);
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_system_disk_is_refused_before_anything_is_prepared() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);
    let mut system = card(64 * 1024);
    system.system = true;
    let platform = Arc::new(FakePlatform::new(target.clone(), system.clone()));
    let result = run(
        &platform,
        &system,
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(matches!(result, Err(DiskError::Unsafe(text)) if text.contains("running system")));
    assert_eq!(platform.events(), ["may_write", "list"]);
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_disk_that_changed_or_vanished_ends_the_write_with_its_own_message() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);

    // Same path, another serial: someone swapped the card.
    let mut swapped = card(64 * 1024);
    swapped.serial = Some("OTHER".into());
    let platform = Arc::new(FakePlatform::new(target.clone(), swapped));
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(
        matches!(result, Err(DiskError::Unsafe(text)) if text.contains("not the disk that was chosen"))
    );
    assert!(!platform.events().iter().any(|e| e == "prepare"));

    // Gone from the listing: a NotFound, nothing prepared.
    let mut gone = FakePlatform::new(target.clone(), card(64 * 1024));
    gone.listing = Mutex::new(Vec::new());
    let gone = Arc::new(gone);
    let result = run(
        &gone,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(matches!(result, Err(DiskError::NotFound(_))));
    assert!(!gone.events().iter().any(|e| e == "prepare"));

    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_disk_above_the_large_bound_needs_the_second_choice() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);
    let big = card(atlas_disk::LARGE_DISK_BYTES + 512);
    let platform = Arc::new(FakePlatform::new(target.clone(), big.clone()));
    let refused = run(
        &platform,
        &big,
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(
        matches!(refused, Err(DiskError::Unsafe(text)) if text.contains("second, explicit choice"))
    );
    assert!(!platform.events().iter().any(|e| e == "prepare"));
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_failed_open_takes_nothing_and_writes_nothing() {
    let image = image_file(&raw_image(4096));
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.fail_open = true;
    let platform = Arc::new(platform);
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(result.is_err());
    let events = platform.events();
    assert_eq!(
        events,
        ["may_write", "list", "prepare", "open", "drop-guard"]
    );
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_cancelled_write_is_not_finished_and_writes_nothing_after_the_cancel() {
    let image = image_file(&raw_image(20_000));
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.cancel_at_write = Some(2);
    let platform = Arc::new(platform);
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(matches!(result, Err(DiskError::Cancelled)), "{result:?}");
    let events = platform.events();
    assert!(
        !events.iter().any(|e| e == "finish"),
        "a cancelled write is not finished"
    );
    assert_eq!(platform.writes().len(), 2, "no write after the cancel");
    assert_eq!(&events[events.len() - 2..], ["drop-raw", "drop-guard"]);
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

/// Needs the `xz` program (unix only, see `common::xz_bytes`).
#[test]
#[cfg(unix)]
fn nothing_is_written_past_the_image_and_the_signature_record_never_is() {
    let raw = raw_image(10_000);
    let record = vec![0x5A; 108];
    let mut two_streams = common::xz_bytes(&raw);
    two_streams.extend_from_slice(&common::xz_bytes(&record));
    let image = image_file(&two_streams);
    let target = target_file(64 * 1024);
    let platform = Arc::new(FakePlatform::new(target.clone(), card(64 * 1024)));
    run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    )
    .unwrap();
    let furthest = platform
        .writes()
        .iter()
        .map(|(offset, len)| offset + *len as u64)
        .max()
        .unwrap();
    assert_eq!(furthest, 10_000, "nothing is written past the image's size");
    let on_disk = fs::read(&target).unwrap();
    assert_eq!(&on_disk[..10_000], &raw[..]);
    assert!(
        on_disk[10_000..].iter().all(|b| *b == 0xEE),
        "the record and everything after the image is untouched"
    );
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_drop_reattaches_and_the_write_goes_on_on_the_same_disk() {
    let raw = raw_image(10_000);
    let image = image_file(&raw);
    let target = target_file(64 * 1024);
    let platform = FakePlatform::new(target.clone(), card(64 * 1024));
    *platform.drop_at_write.lock().unwrap() = Some(2);
    let platform = Arc::new(platform);
    let mut resumed = false;
    run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |event| {
            resumed |= matches!(event, WriteProgress::Resuming { .. });
        },
    )
    .unwrap();
    assert!(resumed, "the write reports that it resumed");
    let events = platform.events();
    assert!(events.iter().any(|e| e == "dropped"));
    assert_eq!(
        events.iter().filter(|e| *e == "open").count(),
        2,
        "the disk is opened again after the drop"
    );
    assert_eq!(&fs::read(&target).unwrap()[..10_000], &raw[..]);
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_disk_that_vanishes_during_a_drop_is_reported_by_name_until_cancelled() {
    let image = image_file(&raw_image(10_000));
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    *platform.drop_at_write.lock().unwrap() = Some(2);
    platform.vanish_from_listing = Some(2);
    platform.cancel_at_listing = Some(4);
    let platform = Arc::new(platform);
    let mut reasons = Vec::new();
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |event| {
            if let WriteProgress::Reattaching { reason, .. } = event {
                reasons.push(reason);
            }
        },
    );
    assert!(matches!(result, Err(DiskError::Cancelled)), "{result:?}");
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("fake9 is not there")),
        "{reasons:?}"
    );
    assert!(!platform.events().iter().any(|e| e == "finish"));
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}

#[test]
fn a_failed_finish_says_the_image_was_written_and_verified() {
    let raw = raw_image(4096);
    let image = image_file(&raw);
    let target = target_file(64 * 1024);
    let mut platform = FakePlatform::new(target.clone(), card(64 * 1024));
    platform.fail_finish = true;
    let platform = Arc::new(platform);
    let result = run(
        &platform,
        &card(64 * 1024),
        Acknowledgement::Given,
        &image,
        false,
        &mut |_| {},
    );
    assert!(
        matches!(&result, Err(DiskError::Finish(text)) if text.contains("eject refused")),
        "{result:?}"
    );
    let _ = fs::remove_file(target);
    let _ = fs::remove_file(image);
}
