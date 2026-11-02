//! Writing an image over a link that drops: a board's eMMC behind USB hubs
//! that reset. The image goes out in segments. Each is written, flushed,
//! and read back before it counts, and a journal beside the image records
//! how far the disk is proven. When the link drops, the target is reattached
//! (the board found again, booted again if it fell back to its ROM) and the
//! segment goes out again from memory. A later run with the same journal
//! re-reads the proven part instead of writing it, then goes on from there.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use atlas_driver::util::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::BlockError;
use crate::image::{decode_into, detect_format};
use crate::writer::{WriteOptions, WriteProgress, WriteReport, cancelled_error, hash_file};

#[cfg(target_os = "linux")]
mod disk_target;
#[cfg(target_os = "linux")]
pub use disk_target::DiskTarget;

/// Sector size every segment is padded to.
const SECTOR: usize = 4096;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Where a resumable write lands: a disk that can go away and come back.
pub trait Target {
    /// Bytes the target holds.
    fn capacity(&self) -> u64;
    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()>;
    /// Makes what was written durable on the medium.
    fn flush(&mut self) -> io::Result<()>;
    /// Reads from the medium, not from a cache.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    /// After a drop: waits for the target to come back and reopens it, by
    /// `deadline` at the latest. `Err` says why it will not.
    fn reattach(&mut self, deadline: Instant) -> Result<(), String>;
}

/// How a resumable write behaves and when it gives up.
#[derive(Clone, Debug)]
pub struct ResumePolicy {
    /// Bytes proven at a time: what one drop costs at most.
    pub segment_bytes: usize,
    /// Drops survived before giving up.
    pub max_drops: u32,
    /// The whole write, drops and reattaching included.
    pub deadline: Duration,
}

impl Default for ResumePolicy {
    fn default() -> Self {
        Self {
            // About a second of eMMC writes behind a USB 2 hub.
            segment_bytes: 16 << 20,
            max_drops: 25,
            deadline: Duration::from_secs(45 * 60),
        }
    }
}

/// What makes a journal apply: the same image, target and segment size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    image: String,
    target: String,
    segment_bytes: u64,
    /// Bytes from the start of the disk known to hold the image.
    proven: u64,
}

impl Journal {
    fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// Replaces the file whole, so a crash leaves the old or the new one.
    fn save(&self, path: &Path) {
        let partial = path.with_extension("journal-new");
        let saved = serde_json::to_vec(self)
            .ok()
            .and_then(|bytes| std::fs::write(&partial, bytes).ok())
            .and_then(|()| std::fs::rename(&partial, path).ok());
        if saved.is_none() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

/// The journal for `image`: beside it, `<image>.journal`.
pub fn journal_path(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(".journal");
    PathBuf::from(name)
}

/// Names the image a journal belongs to: its hash when known, else its size
/// and modification time.
fn image_identity(image: &Path, sha256: Option<&str>) -> String {
    if let Some(sha) = sha256 {
        return format!("sha256:{}", sha.trim().to_ascii_lowercase());
    }
    let meta = std::fs::metadata(image).ok();
    let modified = meta
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    format!("{}:{modified}", meta.map_or(0, |meta| meta.len()))
}

/// Why a segment did not land.
enum Missed {
    Io(io::Error),
    Differed,
}

impl std::fmt::Display for Missed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Differed => f.write_str("the read-back differed from what was written"),
        }
    }
}

struct Sink<'a> {
    target: &'a mut dyn Target,
    policy: &'a ResumePolicy,
    verify: bool,
    segment: Vec<u8>,
    readback: Vec<u8>,
    hasher: Sha256,
    written: u64,
    /// Bytes a journal claims; re-read rather than written.
    claimed: u64,
    drops: u32,
    deadline: Instant,
    journal: Option<(PathBuf, Journal)>,
    cancel: &'a AtomicBool,
    input_read: Arc<AtomicU64>,
    input_total: u64,
    progress: &'a mut dyn FnMut(WriteProgress),
    last_report: Instant,
    /// Set when the write stops for a reason of its own.
    failed: Option<BlockError>,
}

impl Sink<'_> {
    fn input_done(&self) -> u64 {
        self.input_read.load(Ordering::Relaxed)
    }

    fn stop(&mut self, error: BlockError) -> io::Error {
        self.failed = Some(error);
        cancelled_error()
    }

    /// Writes, flushes and reads back the segment buffer at `written`.
    fn land(&mut self, padded: usize) -> Result<(), Missed> {
        let offset = self.written;
        self.target
            .write_at(offset, &self.segment[..padded])
            .map_err(Missed::Io)?;
        self.target.flush().map_err(Missed::Io)?;
        if self.verify {
            self.target
                .read_at(offset, &mut self.readback[..padded])
                .map_err(Missed::Io)?;
            if self.readback[..padded] != self.segment[..padded] {
                return Err(Missed::Differed);
            }
        }
        Ok(())
    }

    /// Whether the disk already holds the segment, as a journal claims.
    fn already_there(&mut self, padded: usize) -> bool {
        let offset = self.written;
        self.target
            .read_at(offset, &mut self.readback[..padded])
            .is_ok()
            && self.readback[..padded] == self.segment[..padded]
    }

    fn commit_segment(&mut self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(cancelled_error());
        }
        let length = self.segment.len();
        self.hasher.update(&self.segment);
        if self.written + length as u64 > self.target.capacity() {
            return Err(self.stop(BlockError::TooLarge {
                image: self.written + length as u64,
                disk: self.target.capacity(),
            }));
        }
        let padded = length.div_ceil(SECTOR) * SECTOR;
        self.segment.resize(padded, 0);
        if self.readback.len() < padded {
            self.readback.resize(padded, 0);
        }
        let end = self.written + length as u64;
        let skip = end <= self.claimed && self.already_there(padded);
        if !skip && self.claimed > self.written {
            // The journal's claim ends here: the disk changed since.
            self.claimed = self.written;
        }
        if !skip {
            if self.claimed > 0 && self.claimed == self.written {
                self.claimed = 0;
                let event = WriteProgress::Resuming {
                    written: self.written,
                    input_done: self.input_done(),
                    input_total: self.input_total,
                };
                (self.progress)(event);
            }
            while let Err(missed) = self.land(padded) {
                self.survive(missed)?;
            }
        }
        self.written = end;
        self.segment.clear();
        if let Some((path, journal)) = &mut self.journal {
            journal.proven = end;
            journal.save(path);
        }
        self.report(false);
        Ok(())
    }

    /// Counts a drop, gives up past the limits, else reattaches.
    fn survive(&mut self, missed: Missed) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(cancelled_error());
        }
        self.drops += 1;
        let reason = missed.to_string();
        let event = WriteProgress::LinkDropped {
            written: self.written,
            input_done: self.input_done(),
            input_total: self.input_total,
            drops: self.drops,
            max_drops: self.policy.max_drops,
            reason: reason.clone(),
        };
        (self.progress)(event);
        if self.drops > self.policy.max_drops {
            return Err(self.stop(BlockError::LinkLost(format!(
                "the link to the disk dropped {} times, more than the {} allowed; the last: {reason}",
                self.drops, self.policy.max_drops
            ))));
        }
        if Instant::now() >= self.deadline {
            return Err(self.stop(BlockError::LinkLost(format!(
                "the write ran past its {} minute limit; the last drop: {reason}",
                self.policy.deadline.as_secs() / 60
            ))));
        }
        if let Err(why) = self.target.reattach(self.deadline) {
            return Err(self.stop(BlockError::LinkLost(format!(
                "the link to the disk dropped ({reason}) and it did not come back: {why}"
            ))));
        }
        let event = WriteProgress::Resuming {
            written: self.written,
            input_done: self.input_done(),
            input_total: self.input_total,
        };
        (self.progress)(event);
        Ok(())
    }

    fn report(&mut self, force: bool) {
        if force || self.last_report.elapsed() >= PROGRESS_EVERY {
            self.last_report = Instant::now();
            let event = WriteProgress::Writing {
                input_done: self.input_done(),
                input_total: self.input_total,
                written: self.written,
            };
            (self.progress)(event);
        }
    }
}

impl Write for Sink<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut rest = data;
        while !rest.is_empty() {
            let room = self.policy.segment_bytes - self.segment.len();
            let take = room.min(rest.len());
            self.segment.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.segment.len() == self.policy.segment_bytes {
                self.commit_segment()?;
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Writes `image` to `target` in proven segments, through link drops.
/// With a `journal`, a run picks up where an earlier one with the same
/// image and `target_id` left off; it is removed once the write is proven.
pub fn write_resumable(
    image: &Path,
    target: &mut dyn Target,
    options: &WriteOptions,
    policy: &ResumePolicy,
    journal: Option<(&Path, &str)>,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<WriteReport, BlockError> {
    let started = Instant::now();
    if policy.segment_bytes == 0 || !policy.segment_bytes.is_multiple_of(SECTOR) {
        return Err(BlockError::Image(format!(
            "segments must be whole {SECTOR} byte sectors"
        )));
    }
    if let Some(expected) = &options.image_sha256 {
        let actual = hash_file(image, progress, cancel)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(BlockError::ImageHashMismatch {
                expected: expected.clone(),
                actual,
            });
        }
    }
    let format = detect_format(image)?;
    let input_total = std::fs::metadata(image)
        .map_err(|error| BlockError::io(image, error))?
        .len();
    if format == crate::ImageFormat::Raw && input_total > target.capacity() {
        return Err(BlockError::TooLarge {
            image: input_total,
            disk: target.capacity(),
        });
    }
    // Without a read-back nothing is proven, so there is no journal.
    let journal = journal.filter(|_| options.verify).map(|(path, target_id)| {
        let fresh = Journal {
            image: image_identity(image, options.image_sha256.as_deref()),
            target: target_id.to_string(),
            segment_bytes: policy.segment_bytes as u64,
            proven: 0,
        };
        let earlier = Journal::load(path).filter(|earlier| {
            Journal {
                proven: 0,
                ..earlier.clone()
            } == fresh
        });
        (path.to_path_buf(), earlier.unwrap_or(fresh))
    });

    progress(WriteProgress::Preparing);
    let input_read = Arc::new(AtomicU64::new(0));
    let mut sink = Sink {
        target,
        policy,
        verify: options.verify,
        segment: Vec::with_capacity(policy.segment_bytes),
        readback: Vec::new(),
        hasher: Sha256::new(),
        written: 0,
        claimed: journal.as_ref().map_or(0, |(_, journal)| journal.proven),
        drops: 0,
        deadline: started + policy.deadline,
        journal,
        cancel,
        input_read: input_read.clone(),
        input_total,
        progress,
        last_report: Instant::now(),
        failed: None,
    };
    let decoded = decode_into(image, format, &mut sink, input_read).and_then(|()| {
        if sink.segment.is_empty() {
            Ok(())
        } else {
            sink.commit_segment()
        }
    });
    if let Some(failed) = sink.failed.take() {
        return Err(failed);
    }
    if let Err(error) = decoded {
        if crate::writer::is_cancelled(&error) {
            return Err(BlockError::Cancelled);
        }
        return Err(match error.kind() {
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
                BlockError::Image(error.to_string())
            }
            _ => BlockError::io(image, error),
        });
    }
    sink.report(true);
    if let Some((path, _)) = &sink.journal {
        let _ = std::fs::remove_file(path);
    }
    Ok(WriteReport {
        bytes_written: sink.written,
        sha256: hex(&sink.hasher.finalize()),
        verified: options.verify,
        seconds: started.elapsed().as_secs(),
    })
}

#[cfg(test)]
mod tests;
