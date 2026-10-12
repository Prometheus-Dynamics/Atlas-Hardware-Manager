//! One connection with Atlas: requests in, replies out, over any byte pipe
//! (the FunctionFS endpoints on the board, a socket pair in tests).
//!
//! Three threads keep the link from ever waiting on itself: one reads
//! frames, one writes replies, and a write job decompresses and writes the
//! image while this one keeps taking its data. So a progress reply never
//! holds up the data behind it.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use atlas_flasher_proto::{
    Decoder, Format, Frame, Kind, PROTOCOL, Phase, RebootInto, Reply, Request, json_frame,
};
use sha2::{Digest, Sha256};

use crate::board::{self, Board};
use crate::hex;
use crate::ring::{Emmc, Look, Show};

/// How often progress is reported.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const IO_CHUNK: usize = 4 << 20;

pub struct Context {
    /// The eMMC (`/dev/mmcblk0`).
    pub disk: PathBuf,
    pub board: Board,
    /// The daemon's version, for `hello`.
    pub version: String,
    /// Drop the page cache before verifying, so it reads the eMMC and not
    /// what was just written (on the board; tests write to a file).
    pub drop_caches: bool,
    /// The status ring, showing each phase.
    pub ring: Arc<dyn Show>,
    /// What the eMMC holds now, for whether rebooting to it is safe.
    pub emmc: Arc<Mutex<Emmc>>,
}

impl Context {
    fn set_emmc(&self, state: Emmc) {
        *self
            .emmc
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Atlas went away (unplugged, or the link reset).
    Closed,
    /// Atlas asked for a reboot; the reply is sent.
    Reboot(RebootInto),
}

type Frames = Receiver<Result<Frame, String>>;

/// Serves one connection until it closes or Atlas asks for a reboot.
pub fn serve<R, W>(read: R, write: W, context: &Context) -> Outcome
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let frames = spawn_reader(read);
    let (replies, writer) = spawn_writer(write);
    let outcome = Session {
        context,
        frames,
        replies: replies.clone(),
        last: None,
    }
    .run();
    drop(replies);
    let _ = writer.join();
    outcome
}

fn spawn_reader<R: Read + Send + 'static>(mut read: R) -> Frames {
    let (tx, rx) = mpsc::sync_channel(16);
    thread::spawn(move || {
        let mut decoder = Decoder::default();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = match read.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            decoder.push(&buffer[..n]);
            while let Some(frame) = decoder.next_frame() {
                let failed = frame.is_err();
                if tx.send(frame.map_err(|error| error.to_string())).is_err() || failed {
                    return;
                }
            }
        }
    });
    rx
}

fn spawn_writer<W: Write + Send + 'static>(
    mut write: W,
) -> (Sender<Reply>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Reply>();
    let handle = thread::spawn(move || {
        for reply in rx {
            if write
                .write_all(&json_frame(&reply))
                .and_then(|()| write.flush())
                .is_err()
            {
                return;
            }
        }
    });
    (tx, handle)
}

struct Session<'a> {
    context: &'a Context,
    frames: Frames,
    replies: Sender<Reply>,
    /// What the last write wrote: image bytes and their SHA-256.
    last: Option<(u64, String)>,
}

fn error(message: impl Into<String>) -> Reply {
    Reply::Error {
        message: message.into(),
    }
}

impl Session<'_> {
    fn reply(&self, reply: Reply) {
        let _ = self.replies.send(reply);
    }

    fn run(mut self) -> Outcome {
        loop {
            let frame = match self.frames.recv() {
                Ok(Ok(frame)) => frame,
                Ok(Err(message)) => {
                    self.reply(error(message));
                    return Outcome::Closed;
                }
                Err(_) => return Outcome::Closed,
            };
            if frame.kind == Kind::Data {
                // An empty one may trail an aborted write; it ends nothing.
                if !frame.payload.is_empty() {
                    self.reply(error("image data without a write request"));
                }
                continue;
            }
            let request = match serde_json::from_slice::<Request>(&frame.payload) {
                Ok(request) => request,
                Err(problem) => {
                    self.reply(error(format!("not a request: {problem}")));
                    continue;
                }
            };
            match request {
                Request::Hello { .. } => self.reply(Reply::Hello {
                    protocol: PROTOCOL,
                    daemon: self.context.version.clone(),
                    serial: self.context.board.serial.clone(),
                    model: self.context.board.model.clone(),
                    revision: self.context.board.revision.clone(),
                }),
                Request::Info => self.reply(Reply::Info {
                    serial: self.context.board.serial.clone(),
                    emmc: board::emmc(&self.context.disk),
                }),
                Request::Write {
                    size,
                    sha256,
                    format,
                    keep_data,
                } => {
                    if let Some(outcome) = self.write(size, sha256, format, keep_data) {
                        return outcome;
                    }
                }
                Request::Verify => {
                    let reply = self
                        .verify()
                        .unwrap_or_else(|problem| error(format!("verify: {problem}")));
                    self.reply(reply);
                }
                Request::Reboot {
                    into: RebootInto::Emmc,
                } => {
                    self.reply(Reply::Ok);
                    return Outcome::Reboot(RebootInto::Emmc);
                }
                Request::Reboot {
                    into: RebootInto::UsbBoot,
                } => self.reply(error(
                    "rebooting into USB boot isn't supported; power-cycle with the button held",
                )),
            }
        }
    }

    /// Takes the image and writes it. Returns an outcome only when the link
    /// closed partway.
    fn write(
        &mut self,
        size: u64,
        sha256: Option<String>,
        format: Format,
        keep_data: bool,
    ) -> Option<Outcome> {
        // Atlas sends the image only after `ready`, so a refusal is all.
        let refuse = |this: &mut Self, message: String| {
            this.reply(error(message));
            None
        };
        if keep_data {
            return refuse(
                self,
                "keeping /data across a flash isn't supported by this flasher yet".into(),
            );
        }
        let capacity = board::emmc(&self.context.disk).size;
        if format == Format::Raw && capacity > 0 && size > capacity {
            return refuse(
                self,
                format!("the image is {size} bytes, the eMMC {capacity}"),
            );
        }
        let disk = match OpenOptions::new().write(true).open(&self.context.disk) {
            Ok(disk) => disk,
            Err(problem) => {
                return refuse(
                    self,
                    format!("can't open {}: {problem}", self.context.disk.display()),
                );
            }
        };
        self.last = None;
        self.context.set_emmc(Emmc::Partial);
        self.context.ring.show(Look::Writing(0.0));
        self.reply(Reply::Ready);

        let received = Arc::new(AtomicU64::new(0));
        let (data, data_rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let job = {
            let reader = ChannelReader {
                rx: data_rx,
                buffer: Vec::new(),
                at: 0,
                received: received.clone(),
            };
            let progress = Progress::new(
                self.replies.clone(),
                Phase::Writing,
                size,
                received.clone(),
                self.context.ring.clone(),
            );
            thread::spawn(move || write_image(reader, format, disk, progress))
        };

        let mut hasher = Sha256::new();
        let mut got: u64 = 0;
        let mut feeding = true;
        let mut problem: Option<String> = None;
        let mut closed = false;
        loop {
            match self.frames.recv() {
                Ok(Ok(Frame {
                    kind: Kind::Data,
                    payload,
                })) => {
                    if payload.is_empty() {
                        break;
                    }
                    got += payload.len() as u64;
                    if got > size {
                        problem
                            .get_or_insert_with(|| format!("more than the {size} bytes announced"));
                        feeding = false;
                    }
                    hasher.update(&payload);
                    if feeding && data.send(payload).is_err() {
                        // The write job stopped; its error says why. Keep
                        // reading to the end so the link stays in step.
                        feeding = false;
                    }
                }
                Ok(Ok(Frame {
                    kind: Kind::Json, ..
                })) => {
                    problem.get_or_insert_with(|| "a request came before the image ended".into());
                    break;
                }
                Ok(Err(_)) | Err(_) => {
                    closed = true;
                    break;
                }
            }
        }
        drop(data);
        let written = job
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the write job panicked")));
        if closed {
            self.context.ring.show(Look::Failed);
            return Some(Outcome::Closed);
        }
        let stream_sha = hex(&hasher.finalize());
        let reply = match (problem, written) {
            (Some(problem), _) => error(problem),
            (None, Err(problem)) => error(format!("writing the eMMC: {problem}")),
            (None, Ok(_)) if got != size => error(format!("got {got} of {size} bytes")),
            (None, Ok(_))
                if sha256
                    .as_ref()
                    .is_some_and(|want| !want.eq_ignore_ascii_case(&stream_sha)) =>
            {
                error(format!(
                    "the image arrived with SHA-256 {stream_sha}, not {}",
                    sha256.unwrap_or_default()
                ))
            }
            (None, Ok((bytes, sha256))) => {
                self.last = Some((bytes, sha256.clone()));
                Reply::Written { bytes, sha256 }
            }
        };
        if matches!(reply, Reply::Written { .. }) {
            self.context.set_emmc(Emmc::Written);
        } else {
            self.context.ring.show(Look::Failed);
        }
        self.reply(reply);
        None
    }

    fn verify(&self) -> io::Result<Reply> {
        let Some((bytes, want)) = self.last.clone() else {
            return Ok(error("nothing written to verify"));
        };
        if self.context.drop_caches {
            let _ = std::fs::write("/proc/sys/vm/drop_caches", "3");
        }
        let mut disk = File::open(&self.context.disk)?;
        let done = Arc::new(AtomicU64::new(0));
        let mut progress = Progress::new(
            self.replies.clone(),
            Phase::Verifying,
            bytes,
            done.clone(),
            self.context.ring.clone(),
        );
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; IO_CHUNK];
        let mut left = bytes;
        while left > 0 {
            let take = buffer.len().min(left as usize);
            disk.read_exact(&mut buffer[..take])?;
            hasher.update(&buffer[..take]);
            left -= take as u64;
            done.store(bytes - left, Ordering::Relaxed);
            progress.tick(bytes - left, false);
        }
        progress.tick(bytes, true);
        let sha256 = hex(&hasher.finalize());
        let ok = sha256 == want;
        if ok {
            self.context.set_emmc(Emmc::Verified);
            self.context.ring.show(Look::Done);
        } else {
            self.context.set_emmc(Emmc::Partial);
            self.context.ring.show(Look::Failed);
        }
        Ok(Reply::Verified { ok, sha256 })
    }
}

/// The image as it arrives, read by the write job.
struct ChannelReader {
    rx: Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    at: usize,
    received: Arc<AtomicU64>,
}

impl ChannelReader {
    /// Whether the stream has ended (waits for more when it can come).
    fn at_end(&mut self) -> bool {
        while self.at == self.buffer.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.received
                        .fetch_add(next.len() as u64, Ordering::Relaxed);
                    self.buffer = next;
                    self.at = 0;
                }
                Err(_) => return true,
            }
        }
        false
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at_end() {
            return Ok(0);
        }
        let n = out.len().min(self.buffer.len() - self.at);
        out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

struct Progress {
    replies: Sender<Reply>,
    phase: Phase,
    of: u64,
    received: Arc<AtomicU64>,
    last: Instant,
    ring: Arc<dyn Show>,
}

impl Progress {
    fn new(
        replies: Sender<Reply>,
        phase: Phase,
        of: u64,
        received: Arc<AtomicU64>,
        ring: Arc<dyn Show>,
    ) -> Self {
        Self {
            replies,
            phase,
            of,
            received,
            last: Instant::now(),
            ring,
        }
    }

    fn tick(&mut self, written: u64, last: bool) {
        if !last && self.last.elapsed() < PROGRESS_EVERY {
            return;
        }
        self.last = Instant::now();
        // The ring's arc: how much of the image has come in (writing: the
        // size is the stream's, compressed or not) or been read back.
        let share = |done: u64| {
            if self.of == 0 {
                1.0
            } else {
                done as f32 / self.of as f32
            }
        };
        self.ring.show(match self.phase {
            Phase::Writing => Look::Writing(share(self.received.load(Ordering::Relaxed))),
            Phase::Verifying => Look::Verifying(share(written)),
        });
        let _ = self.replies.send(Reply::Progress {
            phase: self.phase,
            received: self.received.load(Ordering::Relaxed),
            of: self.of,
            written,
        });
    }
}

/// The eMMC end of a write: counts and hashes what goes onto it.
struct Sink {
    disk: BufWriter<File>,
    hasher: Sha256,
    written: u64,
    progress: Progress,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = self.disk.write(bytes)?;
        self.hasher.update(&bytes[..n]);
        self.written += n as u64;
        self.progress.tick(self.written, false);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.disk.flush()
    }
}

fn write_image(
    mut reader: ChannelReader,
    format: Format,
    disk: File,
    progress: Progress,
) -> io::Result<(u64, String)> {
    let mut sink = Sink {
        disk: BufWriter::with_capacity(IO_CHUNK, disk),
        hasher: Sha256::new(),
        written: 0,
        progress,
    };
    match format {
        Format::Raw => {
            io::copy(&mut reader, &mut sink)?;
        }
        // One zstd frame after another, as `zstd` and `cat` make them.
        Format::Zst => {
            while !reader.at_end() {
                let mut decoder =
                    ruzstd::decoding::StreamingDecoder::new(&mut reader).map_err(|problem| {
                        io::Error::new(io::ErrorKind::InvalidData, problem.to_string())
                    })?;
                io::copy(&mut decoder, &mut sink)?;
            }
        }
        Format::Xz => {
            let mut input = io::BufReader::with_capacity(1 << 20, &mut reader);
            lzma_rs::xz_decompress(&mut input, &mut sink).map_err(|problem| match problem {
                lzma_rs::error::Error::IoError(inner) => inner,
                other => io::Error::new(io::ErrorKind::InvalidData, format!("{other:?}")),
            })?;
        }
    }
    sink.disk.flush()?;
    let Sink {
        disk,
        hasher,
        written,
        mut progress,
    } = sink;
    disk.into_inner()
        .map_err(|problem| problem.into_error())?
        .sync_all()?;
    progress.tick(written, true);
    Ok((written, hex(&hasher.finalize())))
}
