//! Writing through link drops, with no hardware: a disk in memory whose link
//! drops at chosen points and loses what it had not flushed. It never gives
//! up on its own, so each test that leaves a disk away ends the write with
//! the cancel, as a person would.

#![cfg(unix)]

mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use atlas_disk::{
    DiskError, ResumePolicy, Target, WriteOptions, WriteProgress, WriteReport, journal_path,
    write_image,
};
use common::{sha256_hex, temp};

const SEGMENT: usize = 64 << 10;

/// A disk whose link drops after given byte counts of writes. Writes sit in a
/// cache until flushed, and a drop loses the cache, as a USB disk does.
struct Dropping {
    id: String,
    medium: Vec<u8>,
    cache: Vec<(u64, Vec<u8>)>,
    written: u64,
    /// Bytes written at which the link drops, in order.
    drop_at: Vec<u64>,
    connected: bool,
    /// Reattaches that fail before one works; `None` never comes back.
    absent_for: Option<u32>,
    /// Reads that return wrong data once.
    garble_reads: u32,
    reattaches: u32,
    writes: Vec<u64>,
    pauses: Vec<Duration>,
}

impl Dropping {
    fn new(size: usize, drop_at: &[u64]) -> Self {
        Self {
            id: "disk-1".into(),
            medium: vec![0xAA; size],
            cache: Vec::new(),
            written: 0,
            drop_at: drop_at.to_vec(),
            connected: true,
            absent_for: Some(0),
            garble_reads: 0,
            reattaches: 0,
            writes: Vec::new(),
            pauses: Vec::new(),
        }
    }

    fn link(&self) -> io::Result<()> {
        if self.connected {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "USB disconnect"))
        }
    }
}

impl Target for Dropping {
    fn capacity(&self) -> u64 {
        self.medium.len() as u64
    }

    fn identity(&self) -> String {
        self.id.clone()
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.link()?;
        self.writes.push(offset);
        self.written += data.len() as u64;
        if self.drop_at.first().is_some_and(|at| self.written >= *at) {
            self.drop_at.remove(0);
            self.connected = false;
            self.cache.clear();
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "USB disconnect"));
        }
        self.cache.push((offset, data.to_vec()));
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.link()?;
        for (offset, data) in self.cache.drain(..) {
            let start = offset as usize;
            self.medium[start..start + data.len()].copy_from_slice(&data);
        }
        Ok(())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.link()?;
        let start = offset as usize;
        buf.copy_from_slice(&self.medium[start..start + buf.len()]);
        if self.garble_reads > 0 {
            self.garble_reads -= 1;
            buf[0] ^= 0xFF;
        }
        Ok(())
    }

    fn reattach(&mut self) -> Result<(), String> {
        self.reattaches += 1;
        match &mut self.absent_for {
            None => Err("the board never showed again".into()),
            Some(0) => {
                self.connected = true;
                Ok(())
            }
            Some(left) => {
                *left -= 1;
                Err("not yet".into())
            }
        }
    }

    fn pause(&mut self, how_long: Duration) {
        self.pauses.push(how_long);
    }
}

fn policy() -> ResumePolicy {
    ResumePolicy {
        segment_bytes: SEGMENT,
        max_backoff: Duration::from_secs(5),
    }
}

fn verified() -> WriteOptions {
    WriteOptions {
        verify: true,
        image_sha256: None,
    }
}

/// An image of a size that ends mid-segment, as a file.
fn image(len: usize) -> (PathBuf, Vec<u8>) {
    let bytes: Vec<u8> = (0..len as u32).map(|i| (i % 251) as u8).collect();
    let path = temp("image.img");
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn count(events: &[WriteProgress], dropped: bool) -> usize {
    events
        .iter()
        .filter(|event| match event {
            WriteProgress::LinkDropped { .. } => dropped,
            WriteProgress::Resuming { .. } => !dropped,
            _ => false,
        })
        .count()
}

fn run(
    image: &Path,
    target: &mut Dropping,
    policy: &ResumePolicy,
    journal: Option<&Path>,
    cancel: &AtomicBool,
) -> (Result<WriteReport, DiskError>, Vec<WriteProgress>) {
    let mut events = Vec::new();
    let result = write_image(
        image,
        target,
        &verified(),
        policy,
        journal,
        &mut |event| events.push(event),
        cancel,
    );
    (result, events)
}

/// Cancels the write once the target has failed to come back `attempts` times.
fn cancel_after_attempts(attempts: u32, cancel: &AtomicBool) -> impl FnMut(WriteProgress) + '_ {
    move |event| {
        if let WriteProgress::Reattaching { attempt, .. } = event
            && attempt >= attempts
        {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[test]
fn drops_mid_write_resume_and_the_disk_ends_up_right() {
    let (path, bytes) = image(10 * SEGMENT + 1000);
    let mut disk = Dropping::new(16 * SEGMENT, &[SEGMENT as u64 / 2, 5 * SEGMENT as u64]);

    let (result, events) = run(&path, &mut disk, &policy(), None, &AtomicBool::new(false));

    let report = result.unwrap();
    assert_eq!(report.bytes_written, bytes.len() as u64);
    assert_eq!(report.sha256, sha256_hex(&bytes));
    assert!(report.verified);
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    assert_eq!(disk.reattaches, 2);
    assert_eq!((count(&events, true), count(&events, false)), (2, 2));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_drop_costs_one_segment_at_most() {
    let (path, _) = image(8 * SEGMENT);
    let mut disk = Dropping::new(8 * SEGMENT, &[3 * SEGMENT as u64 + 10]);

    let (result, events) = run(&path, &mut disk, &policy(), None, &AtomicBool::new(false));

    result.unwrap();
    // Nine segment writes for eight segments: the dropped one went out twice.
    assert_eq!(disk.writes.len(), 9);
    let resumed_at = events.iter().find_map(|event| match event {
        WriteProgress::Resuming { written, .. } => Some(*written),
        _ => None,
    });
    assert_eq!(resumed_at, Some(3 * SEGMENT as u64));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_read_back_that_differs_is_written_again() {
    let (path, bytes) = image(3 * SEGMENT);
    let mut disk = Dropping::new(4 * SEGMENT, &[]);
    disk.garble_reads = 1;

    let (result, events) = run(&path, &mut disk, &policy(), None, &AtomicBool::new(false));

    result.unwrap();
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    assert!(events.iter().any(|event| matches!(
        event,
        WriteProgress::LinkDropped { reason, .. } if reason.contains("read-back differed")
    )));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_disk_that_never_comes_back_is_retried_with_capped_backoff_until_cancelled() {
    let (path, _) = image(4 * SEGMENT);
    let mut disk = Dropping::new(4 * SEGMENT, &[SEGMENT as u64 * 2]);
    disk.absent_for = None;
    let cancel = AtomicBool::new(false);
    let mut events = Vec::new();
    let mut stop = cancel_after_attempts(6, &cancel);

    let result = write_image(
        &path,
        &mut disk,
        &verified(),
        &policy(),
        None,
        &mut |event| {
            stop(event.clone());
            events.push(event);
        },
        &cancel,
    );

    assert!(matches!(result, Err(DiskError::Cancelled)), "{result:?}");
    assert!(disk.reattaches >= 6, "gave up after {}", disk.reattaches);
    assert!(
        disk.pauses
            .iter()
            .all(|pause| *pause <= Duration::from_secs(5)),
        "backoff past the cap: {:?}",
        disk.pauses
    );
    assert!(events.iter().any(|event| matches!(
        event,
        WriteProgress::Reattaching { reason, .. } if reason.contains("never showed again")
    )));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_later_run_resumes_from_the_journal_without_rewriting() {
    let (path, bytes) = image(10 * SEGMENT + 77);
    let journal = journal_path(&path);
    let mut disk = Dropping::new(12 * SEGMENT, &[6 * SEGMENT as u64 + 5]);
    disk.absent_for = None;
    let cancel = AtomicBool::new(false);

    let first = write_image(
        &path,
        &mut disk,
        &verified(),
        &policy(),
        Some(&journal),
        &mut cancel_after_attempts(2, &cancel),
        &cancel,
    );
    assert!(matches!(first, Err(DiskError::Cancelled)));
    assert!(journal.exists(), "an unfinished write keeps its journal");

    // The board is back for a new run.
    disk.connected = true;
    disk.absent_for = Some(0);
    disk.writes.clear();
    cancel.store(false, Ordering::Relaxed);
    let (second, events) = run(&path, &mut disk, &policy(), Some(&journal), &cancel);

    second.unwrap();
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    assert_eq!(disk.writes.first(), Some(&(6 * SEGMENT as u64)));
    assert_eq!(disk.writes.len(), 5);
    assert_eq!(count(&events, false), 1);
    assert!(!journal.exists(), "a proven write leaves no journal");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_journal_for_another_target_is_not_trusted() {
    let (path, bytes) = image(4 * SEGMENT);
    let journal = journal_path(&path);
    let mut first_disk = Dropping::new(4 * SEGMENT, &[2 * SEGMENT as u64 + 1]);
    first_disk.absent_for = None;
    let cancel = AtomicBool::new(false);
    let _ = write_image(
        &path,
        &mut first_disk,
        &verified(),
        &policy(),
        Some(&journal),
        &mut cancel_after_attempts(1, &cancel),
        &cancel,
    );
    assert!(journal.exists());

    let mut other = Dropping::new(4 * SEGMENT, &[]);
    other.id = "disk-2".into();
    let (result, _) = run(
        &path,
        &mut other,
        &policy(),
        Some(&journal),
        &AtomicBool::new(false),
    );

    result.unwrap();
    assert_eq!(other.writes.len(), 4, "another disk's journal is ignored");
    assert_eq!(&other.medium[..bytes.len()], &bytes[..]);
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(journal);
}

#[test]
fn a_changed_disk_is_rewritten_from_the_first_changed_segment() {
    let (path, bytes) = image(6 * SEGMENT);
    let journal = journal_path(&path);
    let mut disk = Dropping::new(6 * SEGMENT, &[4 * SEGMENT as u64 + 10]);
    disk.absent_for = None;
    let cancel = AtomicBool::new(false);
    let _ = write_image(
        &path,
        &mut disk,
        &verified(),
        &policy(),
        Some(&journal),
        &mut cancel_after_attempts(1, &cancel),
        &cancel,
    );
    assert!(journal.exists());

    // The journal claims four segments; the disk changed in the third since.
    disk.medium[2 * SEGMENT + 9] ^= 1;
    disk.connected = true;
    disk.absent_for = Some(0);
    disk.writes.clear();
    cancel.store(false, Ordering::Relaxed);
    let (result, _) = run(&path, &mut disk, &policy(), Some(&journal), &cancel);

    result.unwrap();
    assert_eq!(
        disk.writes,
        vec![2, 3, 4, 5]
            .into_iter()
            .map(|n| (n * SEGMENT) as u64)
            .collect::<Vec<_>>()
    );
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_compressed_image_resumes_too() {
    let original: Vec<u8> = (0..(1 << 20) + 5u32).map(|i| (i % 241) as u8).collect();
    let compressed = common::xz_bytes(&original);
    let path = temp("image.img.xz");
    std::fs::write(&path, &compressed).unwrap();
    let mut disk = Dropping::new(2 << 20, &[256 << 10, 512 << 10]);
    let policy = ResumePolicy {
        segment_bytes: 128 << 10,
        ..policy()
    };

    let (result, _) = run(&path, &mut disk, &policy, None, &AtomicBool::new(false));

    assert_eq!(result.unwrap().bytes_written, original.len() as u64);
    assert_eq!(&disk.medium[..original.len()], &original[..]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn cancelling_between_segments_stops_the_write() {
    let (path, _) = image(4 * SEGMENT);
    let mut disk = Dropping::new(4 * SEGMENT, &[]);

    let (result, _) = run(&path, &mut disk, &policy(), None, &AtomicBool::new(true));

    assert!(matches!(result, Err(DiskError::Cancelled)));
    assert!(disk.writes.is_empty());
    let _ = std::fs::remove_file(path);
}
