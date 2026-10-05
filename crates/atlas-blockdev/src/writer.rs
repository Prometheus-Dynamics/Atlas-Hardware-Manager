//! Writing an image to a disk and proving it landed.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::image::decode_into;
use crate::platform::{SECTOR, open_for_verify, open_for_write, prepare_for_write};
use crate::{BlockError, Disk, detect_format};

const CHUNK: usize = 4 << 20;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOptions {
    /// Read the disk back after writing and compare.
    pub verify: bool,
    /// Refuse the image unless its file has this SHA-256.
    pub image_sha256: Option<String>,
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
    /// counts bytes landed on the disk.
    Writing {
        input_done: u64,
        input_total: u64,
        written: u64,
    },
    Syncing,
    Verifying {
        done: u64,
        total: u64,
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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

/// Buffers decoded bytes into sector-aligned chunks and writes them.
struct DiskSink<'a> {
    file: File,
    buffer: Vec<u8>,
    hasher: Sha256,
    written: u64,
    capacity: u64,
    cancel: &'a AtomicBool,
    input_read: Arc<AtomicU64>,
    input_total: u64,
    progress: &'a mut dyn FnMut(WriteProgress),
    last_report: Instant,
}

impl DiskSink<'_> {
    fn flush_full_chunks(&mut self, force: bool) -> io::Result<()> {
        while self.buffer.len() >= CHUNK || (force && !self.buffer.is_empty()) {
            let take = self.buffer.len().min(CHUNK);
            let mut chunk: Vec<u8> = self.buffer.drain(..take).collect();
            self.hasher.update(&chunk);
            let data_len = chunk.len() as u64;
            if self.written + data_len > self.capacity {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "image larger than disk",
                ));
            }
            // Raw nodes on Windows and macOS only accept whole sectors.
            chunk.resize(chunk.len().div_ceil(SECTOR) * SECTOR, 0);
            self.file.write_all(&chunk)?;
            self.written += data_len;
            self.report(false);
        }
        Ok(())
    }

    fn report(&mut self, force: bool) {
        if force || self.last_report.elapsed() >= PROGRESS_EVERY {
            self.last_report = Instant::now();
            (self.progress)(WriteProgress::Writing {
                input_done: self.input_read.load(Ordering::Relaxed),
                input_total: self.input_total,
                written: self.written,
            });
        }
    }
}

impl Write for DiskSink<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(cancelled_error());
        }
        self.buffer.extend_from_slice(data);
        self.flush_full_chunks(false)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn hash_file(
    path: &Path,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<String, BlockError> {
    let mut file = File::open(path).map_err(|error| BlockError::io(path, error))?;
    let total = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(BlockError::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| BlockError::io(path, error))?;
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

fn verify(
    disk: &Disk,
    length: u64,
    expected: &str,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<(), BlockError> {
    let mut file = open_for_verify(disk)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut last = Instant::now();
    while done < length {
        if cancel.load(Ordering::Relaxed) {
            return Err(BlockError::Cancelled);
        }
        // Reads stay sector-aligned; only the image's bytes are hashed.
        let want = CHUNK.min((length - done).div_ceil(SECTOR as u64) as usize * SECTOR);
        file.read_exact(&mut buffer[..want])
            .map_err(|error| BlockError::io(&disk.path, error))?;
        let useful = want.min((length - done) as usize);
        hasher.update(&buffer[..useful]);
        done += useful as u64;
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            progress(WriteProgress::Verifying {
                done,
                total: length,
            });
        }
    }
    let actual = hex(&hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(BlockError::VerifyMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}

/// Writes `image` to `disk`. The caller must have checked the disk with
/// [`check_target`](crate::check_target) on a fresh listing.
pub fn write_image(
    image: &Path,
    disk: &Disk,
    options: &WriteOptions,
    progress: &mut dyn FnMut(WriteProgress),
    cancel: &AtomicBool,
) -> Result<WriteReport, BlockError> {
    let started = Instant::now();
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
    if format == crate::ImageFormat::Raw && input_total > disk.size_bytes {
        return Err(BlockError::TooLarge {
            image: input_total,
            disk: disk.size_bytes,
        });
    }

    progress(WriteProgress::Preparing);
    let restore = prepare_for_write(disk)?;
    let input_read = Arc::new(AtomicU64::new(0));
    let mut sink = DiskSink {
        file: open_for_write(disk)?,
        buffer: Vec::with_capacity(CHUNK * 2),
        hasher: Sha256::new(),
        written: 0,
        capacity: disk.size_bytes,
        cancel,
        input_read: input_read.clone(),
        input_total,
        progress,
        last_report: Instant::now(),
    };
    let decoded = decode_into(image, format, &mut sink, input_read)
        .and_then(|()| sink.flush_full_chunks(true));
    if let Err(error) = decoded {
        if is_cancelled(&error) {
            return Err(BlockError::Cancelled);
        }
        return Err(match error.kind() {
            io::ErrorKind::WriteZero => BlockError::TooLarge {
                image: sink.written + sink.buffer.len() as u64,
                disk: disk.size_bytes,
            },
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
                BlockError::Image(error.to_string())
            }
            _ => BlockError::io(&disk.path, error),
        });
    }
    sink.report(true);
    let DiskSink {
        file,
        hasher,
        written,
        progress,
        ..
    } = sink;
    progress(WriteProgress::Syncing);
    file.sync_all()
        .map_err(|error| BlockError::io(&disk.path, error))?;
    drop(file);
    let sha256 = hex(&hasher.finalize());

    if options.verify {
        verify(disk, written, &sha256, progress, cancel)?;
    }
    drop(restore);
    Ok(WriteReport {
        bytes_written: written,
        sha256,
        verified: options.verify,
        seconds: started.elapsed().as_secs(),
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::disk::tests::disk;

    fn temp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atlas-blockdev-writer-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ))
    }

    /// A regular file stands in for the disk: same open, write, sync, and
    /// read-back path, no privileges needed.
    fn fake_disk(size: u64) -> (Disk, PathBuf) {
        let path = temp("disk");
        std::fs::write(&path, vec![0xAAu8; size as usize]).unwrap();
        let mut target = disk();
        target.path = path.clone();
        target.size_bytes = size;
        (target, path)
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_compressed_image_is_written_and_verified() {
        let original: Vec<u8> = (0..(5 << 20) + 123u32).map(|i| (i % 253) as u8).collect();
        let mut compressed = Vec::new();
        lzma_rs::xz_compress(&mut &original[..], &mut compressed).unwrap();
        let image = temp("image.xz");
        std::fs::write(&image, &compressed).unwrap();
        let (target, disk_path) = fake_disk(16 << 20);
        let mut phases = Vec::new();

        let report = write_image(
            &image,
            &target,
            &WriteOptions {
                verify: true,
                image_sha256: Some(sha(&compressed)),
            },
            &mut |progress| phases.push(progress),
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(report.bytes_written, original.len() as u64);
        assert_eq!(report.sha256, sha(&original));
        assert!(report.verified);
        let on_disk = std::fs::read(&disk_path).unwrap();
        assert_eq!(&on_disk[..original.len()], &original[..]);
        assert!(phases.contains(&WriteProgress::Syncing));
        let _ = std::fs::remove_file(image);
        let _ = std::fs::remove_file(disk_path);
    }

    #[test]
    fn a_wrong_image_hash_stops_before_touching_the_disk() {
        let image = temp("image.img");
        std::fs::write(&image, b"image").unwrap();
        let (target, disk_path) = fake_disk(1 << 20);

        let result = write_image(
            &image,
            &target,
            &WriteOptions {
                verify: true,
                image_sha256: Some("00".repeat(32)),
            },
            &mut |_| {},
            &AtomicBool::new(false),
        );

        assert!(matches!(result, Err(BlockError::ImageHashMismatch { .. })));
        assert!(
            std::fs::read(&disk_path)
                .unwrap()
                .iter()
                .all(|b| *b == 0xAA)
        );
        let _ = std::fs::remove_file(image);
        let _ = std::fs::remove_file(disk_path);
    }

    #[test]
    fn an_image_larger_than_the_disk_is_refused() {
        let image = temp("big.img");
        std::fs::write(&image, vec![1u8; 2 << 20]).unwrap();
        let (target, disk_path) = fake_disk(1 << 20);

        let result = write_image(
            &image,
            &target,
            &WriteOptions::default(),
            &mut |_| {},
            &AtomicBool::new(false),
        );

        assert!(matches!(result, Err(BlockError::TooLarge { .. })));
        let _ = std::fs::remove_file(image);
        let _ = std::fs::remove_file(disk_path);
    }

    #[test]
    fn cancelling_stops_the_write() {
        let image = temp("cancel.img");
        std::fs::write(&image, vec![1u8; 8 << 20]).unwrap();
        let (target, disk_path) = fake_disk(16 << 20);

        let result = write_image(
            &image,
            &target,
            &WriteOptions::default(),
            &mut |_| {},
            &AtomicBool::new(true),
        );

        assert!(matches!(result, Err(BlockError::Cancelled)));
        let _ = std::fs::remove_file(image);
        let _ = std::fs::remove_file(disk_path);
    }
}
