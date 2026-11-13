//! Atlas's own log: one file in a directory, rotated by size.
//!
//! It records three kinds of entry: UI errors, every failed call (with what
//! was asked), and breadcrumbs (what the user did just before). A support
//! bundle reads the files with [`Logger::files`] or [`Logger::read_all`].
//!
//! Guarantees:
//! - Recording never waits on the disk. Entries go to a bounded queue read
//!   by one background thread. When the queue is full the entry is dropped
//!   and the drop is counted and written as a `NOTICE` line later.
//! - Logging never fails the caller and never panics. Write errors drop the
//!   entry; [`Logger::disabled`] is a logger that records nothing.
//! - Entries are bounded. Control characters (including newlines) become
//!   spaces, and each field is cut to a fixed size.
//! - Callers pass arguments already redacted. This crate does not look for
//!   secrets, so never pass a password, token, key or account number.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The file name used by [`Config::default`].
pub const DEFAULT_FILE_NAME: &str = "atlas.log";
/// The size at which the current file rotates, by default (5 MiB).
pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The number of rotated files kept by default (`atlas.log.1`).
pub const DEFAULT_KEEP: usize = 1;
/// The smallest rotation size accepted by [`Logger::open`].
pub const MIN_MAX_BYTES: u64 = 1024;

/// Longest text kept for one argument, error or message, in bytes.
const FIELD_BYTES: usize = 1024;
/// Longest name kept for a call or a UI place, in bytes.
const NAME_BYTES: usize = 256;
/// Entries waiting for the writer before new ones are dropped.
const QUEUE_CAPACITY: usize = 1024;
/// How long [`Logger::flush`] waits for the writer.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the log lives and when it rotates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The current file's name inside the directory. Rotated files are
    /// `<name>.1`, `<name>.2`, and so on.
    pub file_name: String,
    /// The size at which the current file rotates. Rotation happens before
    /// the entry that would pass this size, so a file holds at most this many
    /// bytes unless one entry alone is larger.
    pub max_bytes: u64,
    /// How many rotated files to keep. Zero keeps only the current file.
    pub keep: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            file_name: DEFAULT_FILE_NAME.to_owned(),
            max_bytes: DEFAULT_MAX_BYTES,
            keep: DEFAULT_KEEP,
        }
    }
}

/// The kind of an entry, as written in its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An error shown or caught in the UI.
    UiError,
    /// A call that failed, with what was asked.
    FailedCall,
    /// Something the user did, for context around an error.
    Breadcrumb,
    /// Lifecycle notes, and the count of dropped entries.
    Notice,
}

impl Kind {
    /// The label written at the start of the entry's line.
    pub fn label(self) -> &'static str {
        match self {
            Kind::UiError => "UI_ERROR",
            Kind::FailedCall => "CALL_FAILED",
            Kind::Breadcrumb => "CRUMB",
            Kind::Notice => "NOTICE",
        }
    }
}

enum Message {
    Line(String),
    Flush(SyncSender<()>),
    /// Test only: tells `taken` the writer has it, then parks the writer until
    /// `gate` yields or closes.
    #[cfg(test)]
    Hold {
        taken: SyncSender<()>,
        gate: Receiver<()>,
    },
}

#[derive(Debug)]
struct Shared {
    dir: PathBuf,
    config: Config,
    sender: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

/// Records entries into the log. Cheap to clone; every clone writes to the
/// same file.
#[derive(Clone, Debug)]
pub struct Logger {
    shared: Option<Arc<Shared>>,
}

impl Logger {
    /// Opens the log in `dir`, creating the directory if needed, and starts
    /// the writer thread. Fails if the config is invalid, the directory cannot
    /// be created, or the file cannot be opened for appending. Callers may
    /// ignore the error and use [`Logger::disabled`] instead.
    pub fn open(dir: impl AsRef<Path>, config: Config) -> io::Result<Self> {
        Self::open_with_capacity(dir, config, QUEUE_CAPACITY)
    }

    fn open_with_capacity(
        dir: impl AsRef<Path>,
        config: Config,
        capacity: usize,
    ) -> io::Result<Self> {
        validate(&config)?;
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let dropped = Arc::new(AtomicU64::new(0));
        let writer = Writer::new(&dir, &config, Arc::clone(&dropped))?;
        let (sender, receiver) = mpsc::sync_channel(capacity);
        thread::Builder::new()
            .name("atlas-log".to_owned())
            .spawn(move || writer.run(receiver))?;
        Ok(Self {
            shared: Some(Arc::new(Shared {
                dir,
                config,
                sender,
                dropped,
            })),
        })
    }

    /// A logger that records nothing and has no files.
    pub fn disabled() -> Self {
        Self { shared: None }
    }

    /// The directory the log lives in, or `None` when disabled.
    pub fn dir(&self) -> Option<&Path> {
        self.shared.as_ref().map(|shared| shared.dir.as_path())
    }

    /// The current file's path, or `None` when disabled.
    pub fn path(&self) -> Option<PathBuf> {
        self.shared
            .as_ref()
            .map(|shared| shared.dir.join(&shared.config.file_name))
    }

    /// Records a UI error. `place` names the screen or control, and `message`
    /// is the error text.
    pub fn ui_error(&self, place: &str, message: &str) {
        self.record(
            Kind::UiError,
            &format!(
                "{}: {}",
                clean(place, NAME_BYTES),
                clean(message, FIELD_BYTES)
            ),
        );
    }

    /// Records a failed call. `call` names it, `asked` holds what was asked
    /// (already redacted by the caller), and `error` is why it failed.
    pub fn failed_call(&self, call: &str, asked: &str, error: &str) {
        self.record(
            Kind::FailedCall,
            &format!(
                "{} asked [{}] error: {}",
                clean(call, NAME_BYTES),
                clean(asked, FIELD_BYTES),
                clean(error, FIELD_BYTES)
            ),
        );
    }

    /// Records a breadcrumb: something the user or the app did, for context.
    pub fn breadcrumb(&self, message: &str) {
        self.record(Kind::Breadcrumb, &clean(message, FIELD_BYTES));
    }

    /// Records a lifecycle note, such as the app starting.
    pub fn notice(&self, message: &str) {
        self.record(Kind::Notice, &clean(message, FIELD_BYTES));
    }

    /// Waits until every entry recorded before this call is in the file.
    /// Returns `false` if that did not happen within ten seconds, or if the
    /// writer is gone. Returns `true` when disabled.
    pub fn flush(&self) -> bool {
        let Some(shared) = &self.shared else {
            return true;
        };
        let (ack, done) = mpsc::sync_channel(1);
        if shared.sender.send(Message::Flush(ack)).is_err() {
            return false;
        }
        done.recv_timeout(FLUSH_TIMEOUT).is_ok()
    }

    /// The files that exist now, oldest first, with the current file last.
    /// Entries still in the queue may not be in them yet; call
    /// [`Logger::flush`] first if that matters.
    pub fn files(&self) -> Vec<PathBuf> {
        let Some(shared) = &self.shared else {
            return Vec::new();
        };
        let name = &shared.config.file_name;
        let mut files: Vec<PathBuf> = (1..=shared.config.keep)
            .rev()
            .map(|index| shared.dir.join(format!("{name}.{index}")))
            .collect();
        files.push(shared.dir.join(name));
        files.retain(|path| path.is_file());
        files
    }

    /// Flushes, then returns the text of [`Logger::files`] joined oldest
    /// first. Empty when disabled.
    pub fn read_all(&self) -> io::Result<String> {
        self.flush();
        let mut text = String::new();
        for path in self.files() {
            text.push_str(&String::from_utf8_lossy(&fs::read(path)?));
        }
        Ok(text)
    }

    fn record(&self, kind: Kind, text: &str) {
        let Some(shared) = &self.shared else {
            return;
        };
        let line = format!("{} {} {}", timestamp_millis(), kind.label(), text);
        match shared.sender.try_send(Message::Line(line)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn validate(config: &Config) -> io::Result<()> {
    let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
    if config.file_name.is_empty()
        || config.file_name.contains(['/', '\\'])
        || config.file_name == "."
        || config.file_name == ".."
    {
        return Err(invalid("the log file name must be a plain name"));
    }
    if config.max_bytes < MIN_MAX_BYTES {
        return Err(invalid("the log's max_bytes is below the minimum"));
    }
    Ok(())
}

/// Replaces control characters with spaces and cuts the text to `max` bytes
/// at a character boundary, marking the cut with `...`.
fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.len() <= max {
        return flat;
    }
    let mut end = max.saturating_sub(3);
    while !flat.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &flat[..end])
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// The writer thread's state. Only this thread touches the file.
struct Writer {
    dir: PathBuf,
    file_name: String,
    max_bytes: u64,
    keep: usize,
    path: PathBuf,
    file: Option<File>,
    size: u64,
    dropped: Arc<AtomicU64>,
}

impl Writer {
    fn new(dir: &Path, config: &Config, dropped: Arc<AtomicU64>) -> io::Result<Self> {
        let path = dir.join(&config.file_name);
        let file = open_append(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            dir: dir.to_path_buf(),
            file_name: config.file_name.clone(),
            max_bytes: config.max_bytes,
            keep: config.keep,
            path,
            file: Some(file),
            size,
            dropped,
        })
    }

    fn run(mut self, receiver: Receiver<Message>) {
        while let Ok(message) = receiver.recv() {
            match message {
                Message::Line(line) => {
                    let dropped = self.dropped.swap(0, Ordering::Relaxed);
                    if dropped > 0 {
                        self.write_line(&format!(
                            "{} {} {dropped} entries dropped: the queue was full",
                            timestamp_millis(),
                            Kind::Notice.label()
                        ));
                    }
                    self.write_line(&line);
                }
                Message::Flush(ack) => {
                    let _ = ack.send(());
                }
                #[cfg(test)]
                Message::Hold { taken, gate } => {
                    let _ = taken.send(());
                    let _ = gate.recv();
                }
            }
        }
    }

    fn write_line(&mut self, line: &str) {
        let mut record = String::with_capacity(line.len() + 1);
        record.push_str(line);
        record.push('\n');
        let len = record.len() as u64;
        if self.size > 0 && self.size + len > self.max_bytes {
            self.rotate();
        }
        if self.file.is_none() {
            self.file = open_append(&self.path).ok();
            self.size = self
                .file
                .as_ref()
                .and_then(|file| file.metadata().ok())
                .map_or(0, |meta| meta.len());
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(record.as_bytes()).is_ok() {
            self.size += len;
        } else {
            self.file = None;
        }
    }

    fn rotated(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{index}", self.file_name))
    }

    fn rotate(&mut self) {
        self.file = None;
        if self.keep == 0 {
            let _ = fs::remove_file(&self.path);
        } else {
            let _ = fs::remove_file(self.rotated(self.keep));
            for index in (1..self.keep).rev() {
                let _ = fs::rename(self.rotated(index), self.rotated(index + 1));
            }
            let _ = fs::rename(&self.path, self.rotated(1));
        }
        // If a rename failed the current file is still there; empty it so the
        // size limit holds either way.
        if let Ok(file) = open_append(&self.path) {
            let _ = file.set_len(0);
            self.file = Some(file);
        }
        self.size = 0;
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_flattens_control_characters() {
        assert_eq!(clean("a\nb\tc\r", 100), "a b c ");
    }

    #[test]
    fn clean_cuts_at_a_character_boundary() {
        let cut = clean("ééééé", 6);
        assert!(cut.ends_with("..."));
        assert!(cut.len() <= 6);
    }

    #[test]
    fn invalid_configs_are_refused() {
        let plain = Config::default();
        assert!(validate(&plain).is_ok());
        let nested = Config {
            file_name: "a/b.log".to_owned(),
            ..Config::default()
        };
        assert!(validate(&nested).is_err());
        let tiny = Config {
            max_bytes: 10,
            ..Config::default()
        };
        assert!(validate(&tiny).is_err());
    }

    #[test]
    fn entries_dropped_while_the_queue_is_full_are_counted_in_the_file() {
        let dir =
            std::env::temp_dir().join(format!("atlas-log-unit-dropped-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let logger = Logger::open_with_capacity(&dir, Config::default(), 2).expect("open");
        let shared = logger.shared.as_ref().expect("enabled");

        // Park the writer, so the queue cannot drain.
        let (taken_tx, taken_rx) = mpsc::sync_channel(1);
        let (gate_tx, gate_rx) = mpsc::sync_channel::<()>(1);
        shared
            .sender
            .send(Message::Hold {
                taken: taken_tx,
                gate: gate_rx,
            })
            .expect("hold");
        taken_rx.recv().expect("writer parked");

        // Two fit in the queue; the other three are dropped and counted.
        for index in 0..5 {
            logger.breadcrumb(&format!("queued or dropped {index}"));
        }
        assert_eq!(shared.dropped.load(Ordering::Relaxed), 3);

        // Release the writer. The next line it writes is preceded by the count.
        drop(gate_tx);
        assert!(logger.flush());
        let text = fs::read_to_string(dir.join(DEFAULT_FILE_NAME)).expect("current");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(
            lines[0].contains("NOTICE 3 entries dropped"),
            "{}",
            lines[0]
        );
        assert!(lines[1].ends_with("queued or dropped 0"), "{}", lines[1]);
        assert!(lines[2].ends_with("queued or dropped 1"), "{}", lines[2]);
        assert_eq!(shared.dropped.load(Ordering::Relaxed), 0);
    }
}
