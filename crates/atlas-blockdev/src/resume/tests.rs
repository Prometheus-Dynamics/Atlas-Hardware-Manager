//! Fault injection without hardware: a disk in memory whose link drops at
//! chosen points and loses what it had not flushed.

use std::path::PathBuf;

use super::*;

/// A disk whose link drops after given byte counts of writes. Writes sit in
/// a cache until flushed, and a drop loses the cache, as a USB disk does.
struct Dropping {
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
}

impl Dropping {
    fn new(size: usize, drop_at: &[u64]) -> Self {
        Self {
            medium: vec![0xAA; size],
            cache: Vec::new(),
            written: 0,
            drop_at: drop_at.to_vec(),
            connected: true,
            absent_for: Some(0),
            garble_reads: 0,
            reattaches: 0,
            writes: Vec::new(),
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

    fn reattach(&mut self, _deadline: Instant) -> Result<(), String> {
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
}

const SEGMENT: usize = 64 << 10;

fn policy() -> ResumePolicy {
    ResumePolicy {
        segment_bytes: SEGMENT,
        max_drops: 5,
        deadline: Duration::from_secs(60),
    }
}

fn temp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "atlas-blockdev-resume-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ))
}

/// An image of a size that ends mid-segment, as a file.
fn image(len: usize) -> (PathBuf, Vec<u8>) {
    let bytes: Vec<u8> = (0..len as u32).map(|i| (i % 251) as u8).collect();
    let path = temp("image.img");
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn verified() -> WriteOptions {
    WriteOptions {
        verify: true,
        image_sha256: None,
    }
}

fn write(
    image: &Path,
    target: &mut Dropping,
    policy: &ResumePolicy,
    journal: Option<(&Path, &str)>,
) -> (Result<WriteReport, BlockError>, Vec<WriteProgress>) {
    let mut events = Vec::new();
    let result = write_resumable(
        image,
        target,
        &verified(),
        policy,
        journal,
        &mut |event| events.push(event),
        &AtomicBool::new(false),
    );
    (result, events)
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

#[test]
fn drops_mid_write_resume_and_the_disk_ends_up_right() {
    let (path, bytes) = image(10 * SEGMENT + 1000);
    let mut disk = Dropping::new(16 * SEGMENT, &[SEGMENT as u64 / 2, 5 * SEGMENT as u64]);

    let (result, events) = write(&path, &mut disk, &policy(), None);

    let report = result.unwrap();
    assert_eq!(report.bytes_written, bytes.len() as u64);
    assert_eq!(report.sha256, hex(&Sha256::digest(&bytes)));
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

    let (result, events) = write(&path, &mut disk, &policy(), None);

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

    let (result, events) = write(&path, &mut disk, &policy(), None);

    result.unwrap();
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    assert!(events.iter().any(|event| matches!(
        event,
        WriteProgress::LinkDropped { reason, .. } if reason.contains("read-back differed")
    )));
    let _ = std::fs::remove_file(path);
}

#[test]
fn too_many_drops_give_up_with_the_reason() {
    let (path, _) = image(8 * SEGMENT);
    let drops: Vec<u64> = (1..=10).map(|n| n * 1000).collect();
    let mut disk = Dropping::new(8 * SEGMENT, &drops);

    let (result, _) = write(&path, &mut disk, &policy(), None);

    let Err(BlockError::LinkLost(reason)) = result else {
        panic!("expected LinkLost, got {result:?}");
    };
    assert!(
        reason.contains("6 times, more than the 5 allowed"),
        "{reason}"
    );
    assert!(reason.contains("USB disconnect"), "{reason}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_disk_that_never_comes_back_gives_up_with_why() {
    let (path, _) = image(4 * SEGMENT);
    let mut disk = Dropping::new(4 * SEGMENT, &[SEGMENT as u64 * 2]);
    disk.absent_for = None;

    let (result, _) = write(&path, &mut disk, &policy(), None);

    let Err(BlockError::LinkLost(reason)) = result else {
        panic!("expected LinkLost, got {result:?}");
    };
    assert!(reason.contains("never showed again"), "{reason}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_later_run_resumes_from_the_journal_without_rewriting() {
    let (path, bytes) = image(10 * SEGMENT + 77);
    let journal = journal_path(&path);
    let mut disk = Dropping::new(12 * SEGMENT, &[6 * SEGMENT as u64 + 5]);
    disk.absent_for = None;

    let (first, _) = write(&path, &mut disk, &policy(), Some((&journal, "board-1")));
    assert!(matches!(first, Err(BlockError::LinkLost(_))));
    assert_eq!(Journal::load(&journal).unwrap().proven, 6 * SEGMENT as u64);

    // The board is back for a new run.
    disk.connected = true;
    disk.writes.clear();
    let (second, events) = write(&path, &mut disk, &policy(), Some((&journal, "board-1")));

    second.unwrap();
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    assert_eq!(disk.writes.first(), Some(&(6 * SEGMENT as u64)));
    assert_eq!(disk.writes.len(), 5);
    assert_eq!(count(&events, false), 1);
    assert!(!journal.exists(), "a proven write leaves no journal");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_journal_for_another_board_or_a_changed_disk_is_not_trusted() {
    let (path, bytes) = image(4 * SEGMENT);
    let journal = journal_path(&path);
    let mut disk = Dropping::new(4 * SEGMENT, &[]);
    let claim = |target: &str, proven: u64| Journal {
        image: image_identity(&path, None),
        target: target.into(),
        segment_bytes: SEGMENT as u64,
        proven,
    };

    claim("board-2", 4 * SEGMENT as u64).save(&journal);
    write(&path, &mut disk, &policy(), Some((&journal, "board-1")))
        .0
        .unwrap();
    assert_eq!(disk.writes.len(), 4, "another board's journal is ignored");

    // The journal claims it all, but the disk changed in segment 2 since.
    disk.medium[2 * SEGMENT + 9] ^= 1;
    disk.writes.clear();
    claim("board-1", 4 * SEGMENT as u64).save(&journal);
    write(&path, &mut disk, &policy(), Some((&journal, "board-1")))
        .0
        .unwrap();
    assert_eq!(disk.writes, vec![2 * SEGMENT as u64, 3 * SEGMENT as u64]);
    assert_eq!(&disk.medium[..bytes.len()], &bytes[..]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_compressed_image_resumes_too() {
    let original: Vec<u8> = (0..(3 << 20) + 5u32).map(|i| (i % 241) as u8).collect();
    let mut compressed = Vec::new();
    lzma_rs::xz_compress(&mut &original[..], &mut compressed).unwrap();
    let path = temp("image.xz");
    std::fs::write(&path, &compressed).unwrap();
    let mut disk = Dropping::new(4 << 20, &[1 << 20, 2 << 20]);
    let policy = ResumePolicy {
        segment_bytes: 256 << 10,
        ..policy()
    };

    let (result, _) = write(&path, &mut disk, &policy, None);

    assert_eq!(result.unwrap().bytes_written, original.len() as u64);
    assert_eq!(&disk.medium[..original.len()], &original[..]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn cancelling_stops_between_segments() {
    let (path, _) = image(4 * SEGMENT);
    let mut disk = Dropping::new(4 * SEGMENT, &[]);

    let result = write_resumable(
        &path,
        &mut disk,
        &verified(),
        &policy(),
        None,
        &mut |_| {},
        &AtomicBool::new(true),
    );

    assert!(matches!(result, Err(BlockError::Cancelled)));
    assert!(disk.writes.is_empty());
    let _ = std::fs::remove_file(path);
}
