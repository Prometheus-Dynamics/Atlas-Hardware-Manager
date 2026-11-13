//! Writing an image to a target and proving it landed.
//!
//! The image goes out in segments. Each segment is written, flushed and read
//! back before it counts. A drop (a write, flush or read that fails) makes the
//! writer reattach the target and send the segment again from memory. It
//! keeps trying, backing off up to `max_backoff`, until the target is back or
//! the write is cancelled: a write is never given up on by itself.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::image::{decode_into, decoded_at_least, detect_format};
use crate::journal::{Journal, image_identity};
use crate::{DiskError, Target};

const CHUNK: usize = 4 << 20;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const FIRST_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOptions {
    /// Read each segment back after writing and compare it.
    pub verify: bool,
    /// Refuse the image unless its file has this SHA-256.
    pub image_sha256: Option<String>,
}

/// How a write is cut up and how long a drop may take to recover from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumePolicy {
    /// Bytes proven at a time: what one drop costs at most.
    pub segment_bytes: usize,
    /// The longest wait between reattach attempts. Ten seconds at most.
    pub max_backoff: Duration,
}

impl Default for ResumePolicy {
    fn default() -> Self {
        Self {
            // About a second of writes behind a USB 2 hub.
            segment_bytes: 16 << 20,
            max_backoff: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "phase")]
pub enum WriteProgress {
    CheckingImage {
        done: u64,
        total: u64,
    },
    Preparing,
    /// `input_*` count bytes of the (possibly compressed) file; `written`
    /// counts bytes proven on the target.
    Writing {
        input_done: u64,
        input_total: u64,
        written: u64,
    },
    /// The target dropped while `written` bytes were proven.
    LinkDropped {
        written: u64,
        input_done: u64,
        input_total: u64,
        drops: u32,
        reason: String,
    },
    /// The target did not come back on this attempt; the next one follows.
    Reattaching {
        written: u64,
        input_done: u64,
        input_total: u64,
        attempt: u32,
        reason: String,
    },
    /// The target is back: writing goes on from `written`.
    Resuming {
        written: u64,
        input_done: u64,
        input_total: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteReport {
    pub bytes_written: u64,
    /// SHA-256 of the decoded image as written.
    pub sha256: String,
    pub verified: bool,
    pub seconds: u64,
}

/// Marks a cancelled write. Not `ErrorKind::Interrupted`: `io::copy`
/// retries those forever.
#[derive(Debug)]
struct CancelledWrite;

impl std::fmt::Display for CancelledWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for CancelledWrite {}

fn cancelled_error() -> io::Error {
    io::Error::other(CancelledWrite)
}

fn is_cancelled(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<CancelledWrite>())
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

/// How long to wait before reattach attempt `attempt` (1 for the first).
fn backoff(attempt: u32, max: Duration) -> Duration {
    FIRST_BACKOFF.saturating_mul(1 << attempt.min(5)).min(max)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes into segments, proves each one on the target, and counts it.
struct Sink<'a> {
    target: &'a mut dyn Target,
    policy: &'a ResumePolicy,
    verify: bool,
    segment: Vec<u8>,
    readback: Vec<u8>,
    hasher: Sha256,
    written: u64,
    /// Bytes a journal claims; checked by reading, not trusted.
    claimed: u64,
    drops: u32,
    journal: Option<(PathBuf, Journal)>,
    cancel: &'a AtomicBool,
    input_read: Arc<AtomicU64>,
    input_total: u64,
    progress: &'a mut dyn FnMut(WriteProgress),
    last_report: Instant,
    /// Set when the write stops for a reason of its own.
    failed: Option<DiskError>,
}

impl Sink<'_> {
    fn input_done(&self) -> u64 {
        self.input_read.load(Ordering::Relaxed)
    }

    fn stop(&mut self, error: DiskError) -> io::Error {
        self.failed = Some(error);
        cancelled_error()
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Writes, flushes and reads back the first `length` bytes of the segment
    /// at `written`.
    fn land(&mut self, length: usize) -> Result<(), Missed> {
        let offset = self.written;
        self.target
            .write_at(offset, &self.segment[..length])
            .map_err(Missed::Io)?;
        self.target.flush().map_err(Missed::Io)?;
        if self.verify {
            self.target
                .read_at(offset, &mut self.readback[..length])
                .map_err(Missed::Io)?;
            if self.readback[..length] != self.segment[..length] {
                return Err(Missed::Differed);
            }
        }
        Ok(())
    }

    /// Whether the target already holds the segment, as a journal claims.
    fn already_there(&mut self, length: usize) -> bool {
        self.target
            .read_at(self.written, &mut self.readback[..length])
            .is_ok()
            && self.readback[..length] == self.segment[..length]
    }

    fn commit_segment(&mut self) -> io::Result<()> {
        if self.cancelled() {
            return Err(cancelled_error());
        }
        let length = self.segment.len();
        let end = self.written + length as u64;
        if end > self.target.capacity() {
            let disk = self.target.capacity();
            return Err(self.stop(DiskError::TooLarge { image: end, disk }));
        }
        self.hasher.update(&self.segment);
        if self.readback.len() < length {
            self.readback.resize(length, 0);
        }
        let skip = end <= self.claimed && self.already_there(length);
        if !skip && self.claimed > self.written {
            // The journal's claim ends here: the disk changed since.
            self.claimed = self.written;
        }
        if !skip {
            if self.claimed > 0 && self.claimed == self.written {
                self.claimed = 0;
                self.emit(WriteProgress::Resuming {
                    written: self.written,
                    input_done: self.input_done(),
                    input_total: self.input_total,
                });
            }
            while let Err(missed) = self.land(length) {
                self.survive(missed)?;
            }
        }
        self.written = end;
        self.segment.clear();
        if let Some((path, journal)) = &mut self.journal {
            journal.proven = end;
            // A journal that cannot be saved costs time on a rerun, not data.
            let _ = journal.save(path);
        }
        self.report(false);
        Ok(())
    }

    /// Counts a drop, then reattaches the target until it is back or the
    /// write is cancelled.
    fn survive(&mut self, missed: Missed) -> io::Result<()> {
        if self.cancelled() {
            return Err(cancelled_error());
        }
        self.drops = self.drops.saturating_add(1);
        self.emit(WriteProgress::LinkDropped {
            written: self.written,
            input_done: self.input_done(),
            input_total: self.input_total,
            drops: self.drops,
            reason: missed.to_string(),
        });
        let mut attempt = 0u32;
        loop {
            if self.cancelled() {
                return Err(cancelled_error());
            }
            attempt = attempt.saturating_add(1);
            match self.target.reattach() {
                Ok(()) => break,
                Err(reason) => {
                    self.emit(WriteProgress::Reattaching {
                        written: self.written,
                        input_done: self.input_done(),
                        input_total: self.input_total,
                        attempt,
                        reason,
                    });
                    self.target.pause(backoff(attempt, self.policy.max_backoff));
                }
            }
        }
        self.emit(WriteProgress::Resuming {
            written: self.written,
            input_done: self.input_done(),
            input_total: self.input_total,
        });
        Ok(())
    }

    fn emit(&mut self, event: WriteProgress) {
        (self.progress)(event);
    }

    fn report(&mut self, force: bool) {
        if force || self.last_report.elapsed() >= PROGRESS_EVERY {
            self.last_report = Instant::now();
            let event = WriteProgress::Writing {
                input_done: self.input_done(),
                input_total: self.input_total,
                written: self.written,
            };
            self.emit(event);
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

pub(crate) fn hash_file(
    path: &Path,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<String, DiskError> {
    let mut file = File::open(path).map_err(|error| DiskError::io(path, error))?;
    let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DiskError::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| DiskError::io(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        done += read as u64;
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            progress(WriteProgress::CheckingImage { done, total });
        }
    }
    Ok(hex(&hasher.finalize()))
}

/// Writes the image at `image` to `target` in proven segments, through drops.
///
/// With `journal`, a run picks up where an earlier one with the same image,
/// target and segment size left off. The journal is removed once the write
/// is complete. Without `verify` nothing is proven, so no journal is used.
///
/// The caller must have checked the target with [`check_target`](crate::check_target)
/// on a fresh listing.
pub fn write_image(
    image: &Path,
    target: &mut dyn Target,
    options: &WriteOptions,
    policy: &ResumePolicy,
    journal: Option<&Path>,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<WriteReport, DiskError> {
    let started = Instant::now();
    if policy.segment_bytes == 0 {
        return Err(DiskError::Image(
            "segments must hold at least one byte".into(),
        ));
    }
    if let Some(expected) = &options.image_sha256 {
        let actual = hash_file(image, progress, cancel)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(DiskError::ImageHashMismatch {
                expected: expected.clone(),
                actual,
            });
        }
    }
    let format = detect_format(image)?;
    let input_total = std::fs::metadata(image)
        .map_err(|error| DiskError::io(image, error))?
        .len();
    // Refused before anything is written: a raw image by its length, a
    // compressed one by what its header says it decodes to (at least).
    if let Some(size) = decoded_at_least(image, format)
        && size > target.capacity()
    {
        return Err(DiskError::TooLarge {
            image: size,
            disk: target.capacity(),
        });
    }

    let journal = journal.filter(|_| options.verify).map(|path| {
        let fresh = Journal {
            image: image_identity(image, options.image_sha256.as_deref()).unwrap_or_default(),
            target: target.identity(),
            segment_bytes: policy.segment_bytes as u64,
            proven: 0,
        };
        let earlier = Journal::load(path).filter(|earlier| {
            earlier.image == fresh.image
                && earlier.target == fresh.target
                && earlier.segment_bytes == fresh.segment_bytes
        });
        (path.to_path_buf(), earlier.unwrap_or(fresh))
    });
    let claimed = journal.as_ref().map_or(0, |(_, journal)| journal.proven);

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
        claimed,
        drops: 0,
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
        if is_cancelled(&error) {
            return Err(DiskError::Cancelled);
        }
        return Err(match error.kind() {
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
                DiskError::Image(error.to_string())
            }
            _ => DiskError::io(image, error),
        });
    }
    sink.report(true);
    let Sink {
        written,
        hasher,
        journal,
        ..
    } = sink;
    if let Some((path, _)) = &journal {
        Journal::remove(path);
    }
    Ok(WriteReport {
        bytes_written: written,
        sha256: hex(&hasher.finalize()),
        verified: options.verify,
        seconds: started.elapsed().as_secs(),
    })
}
