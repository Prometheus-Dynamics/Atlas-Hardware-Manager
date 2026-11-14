//! The source of a push, read as a stream, and the state the push shares between its attempts.
//!
//! A push never holds the file. Orion's `Files::push` reads the source twice: once from the start
//! to hash it (that hash is the transfer's identity on the board), then from the offset the board
//! holds, a chunk at a time. [`Guard`] sits between that and the file. It checks that the second
//! hash matches the first, so a file that changed under a push is refused before anything more is
//! sent, and it counts the bytes that are sent, so a resume can be proven not to resend what the
//! board holds.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use atlas_session::Cancel;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::progress::{Meter, Progress, Stage};

/// A file to push: read from the start, and from any offset. A [`std::fs::File`] is one, and so
/// is a generated stream. It is moved to the job, which reads it on the node's session.
pub trait PushSource: Read + Seek + Send + 'static {}

impl<T: Read + Seek + Send + 'static> PushSource for T {}

/// What the push has done, counted over all its attempts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// How many times the file was begun on a session. One for a push that never lost its
    /// session; one more for each time the session came back.
    pub attempts: u64,
    /// Bytes read from the source to send, over all attempts. Without a resend this is the file's
    /// length; each extra is a byte that was sent again after a lost session.
    pub bytes_sent: u64,
    /// The most bytes read to send that the board had not yet acknowledged, at any moment.
    /// Orion keeps a window of eight chunks of 32 KiB in flight, so this stays within nine.
    pub max_in_flight: u64,
}

/// The state of one push, shared by its attempts and read by its owner.
pub(crate) struct Shared {
    /// The request to stop the push. A long operation sees it at its next step boundary.
    cancel: Cancel,
    /// Set when the push must stop for good, with why: the source changed or could not be read.
    refusal: Mutex<Option<String>>,
    /// The SHA-256 the file had when the first attempt began.
    identity: Mutex<Option<[u8; 32]>>,
    acked: AtomicU64,
    sent_bytes: AtomicU64,
    max_in_flight: AtomicU64,
    attempts: AtomicU64,
    meter: Mutex<Meter>,
    progress: watch::Sender<Progress>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("acked", &self.acked.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// The state of a push of `total` bytes, and the receiver its progress is read from.
    pub(crate) fn new(total: u64) -> (Self, watch::Receiver<Progress>) {
        let (progress, receiver) = watch::channel(Progress::queued(total));
        let shared = Self {
            cancel: Cancel::new(),
            refusal: Mutex::new(None),
            identity: Mutex::new(None),
            acked: AtomicU64::new(0),
            sent_bytes: AtomicU64::new(0),
            max_in_flight: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            meter: Mutex::new(Meter::default()),
            progress,
        };
        (shared, receiver)
    }

    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The cancel request of this push, for the long operation that sends it.
    pub(crate) fn token(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Stops the push for good, unless it already has a reason.
    pub(crate) fn refuse(&self, reason: String) {
        let mut refusal = lock(&self.refusal);
        if refusal.is_none() {
            *refusal = Some(reason);
        }
    }

    pub(crate) fn refusal(&self) -> Option<String> {
        lock(&self.refusal).clone()
    }

    pub(crate) fn acked(&self) -> u64 {
        self.acked.load(Ordering::SeqCst)
    }

    /// Checks the hash of the file as this attempt read it against the first attempt's.
    fn check_identity(&self, found: [u8; 32]) -> Result<(), String> {
        let mut identity = lock(&self.identity);
        match *identity {
            None => {
                *identity = Some(found);
                Ok(())
            }
            Some(known) if known == found => Ok(()),
            Some(_) => Err(
                "the source changed since the push began: it is not the file this push started with"
                    .to_owned(),
            ),
        }
    }

    /// A new attempt begins on a session.
    pub(crate) fn begin_attempt(&self) {
        self.attempts.fetch_add(1, Ordering::SeqCst);
    }

    /// The board said how many bytes it holds, of how many.
    pub(crate) fn report(&self, received: u64, total: u64) {
        self.acked.store(received, Ordering::SeqCst);
        let rate = lock(&self.meter).record(Instant::now(), received);
        self.progress.send_modify(|progress| {
            progress.sent = received;
            progress.total = total;
            progress.rate_bytes_per_sec = rate;
            if matches!(
                progress.stage,
                Stage::Queued | Stage::WaitingForBoard { .. } | Stage::Resending { .. }
            ) {
                progress.stage = Stage::Sending;
            }
        });
    }

    /// The lease ran out while the session was silent: the board may have dropped the partial file,
    /// so the file is sent again from its start. Stays until the board acknowledges bytes.
    pub(crate) fn resending(&self, reason: String) {
        self.progress.send_modify(|progress| {
            if !matches!(
                progress.stage,
                Stage::Committed | Stage::Failed { .. } | Stage::Cancelled
            ) {
                progress.stage = Stage::Resending { reason };
            }
        });
    }

    /// An attempt starts (or the session it needs is back): the push is sending.
    pub(crate) fn sending(&self) {
        self.progress.send_modify(|progress| {
            if matches!(
                progress.stage,
                Stage::Queued | Stage::WaitingForBoard { .. }
            ) {
                progress.stage = Stage::Sending;
            }
        });
    }

    /// The session is gone: the push is waiting for it, and is not moving bytes.
    pub(crate) fn waiting(&self, reason: String) {
        lock(&self.meter).clear();
        self.progress.send_modify(|progress| {
            progress.rate_bytes_per_sec = 0;
            if !matches!(
                progress.stage,
                Stage::Committed | Stage::Failed { .. } | Stage::Cancelled
            ) {
                progress.stage = Stage::WaitingForBoard { reason };
            }
        });
    }

    /// The push is over. Its final stage is what it says.
    pub(crate) fn finish(&self, stage: Stage) {
        lock(&self.meter).clear();
        self.progress.send_modify(|progress| {
            progress.rate_bytes_per_sec = 0;
            if !matches!(
                progress.stage,
                Stage::Committed | Stage::Failed { .. } | Stage::Cancelled
            ) {
                progress.stage = stage;
            }
        });
    }

    pub(crate) fn current(&self) -> Progress {
        self.progress.borrow().clone()
    }

    pub(crate) fn counters(&self) -> Counters {
        Counters {
            attempts: self.attempts.load(Ordering::SeqCst),
            bytes_sent: self.sent_bytes.load(Ordering::SeqCst),
            max_in_flight: self.max_in_flight.load(Ordering::SeqCst),
        }
    }

    /// `read` bytes went to the board from the source, which now stands at `position`.
    fn sent(&self, read: u64, position: u64) {
        self.sent_bytes.fetch_add(read, Ordering::SeqCst);
        let in_flight = position.saturating_sub(self.acked());
        self.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
    }
}

/// One attempt's view of the source: it hashes the file on the way in, then checks that hash and
/// counts the bytes sent. It holds no bytes of its own, and owns handles to the source and the
/// push's state, so it can be moved to whichever adapter sends it.
pub(crate) struct Guard {
    source: Arc<Mutex<Box<dyn PushSource>>>,
    shared: Arc<Shared>,
    position: u64,
    /// Hashing the file from the start. `None` once that pass has ended.
    hashing: Option<Sha256>,
}

impl Guard {
    pub(crate) fn new(source: Arc<Mutex<Box<dyn PushSource>>>, shared: Arc<Shared>) -> Self {
        shared.begin_attempt();
        Self {
            source,
            shared,
            position: 0,
            hashing: Some(Sha256::new()),
        }
    }

    fn source(&self) -> MutexGuard<'_, Box<dyn PushSource>> {
        lock(&self.source)
    }

    fn refused(&self, reason: String) -> io::Error {
        self.shared.refuse(reason.clone());
        io::Error::other(reason)
    }

    // LOCK kept (Orion D15 is in dbd5fc08: `start_push` takes `Known { digest, size }`). The digest
    // is known from the first attempt, so a resend could skip this hashing pass. It stays in A17:
    // a changed source on a resend would then be caught by the board's digest check at commit
    // ("Rejected(digest)") rather than by this guard's "source changed" refusal, a change of the
    // error a person sees. The first attempt has no digest yet and must hash either way.
    fn hashing_read(&mut self, mut hasher: Sha256, buf: &mut [u8]) -> io::Result<usize> {
        let read = match self.source().read(buf) {
            Ok(read) => read,
            Err(error) => {
                return Err(self.refused(format!("the source could not be read: {error}")));
            }
        };
        if read == 0 {
            let found: [u8; 32] = hasher.finalize().into();
            return match self.shared.check_identity(found) {
                Ok(()) => Ok(0),
                Err(reason) => Err(self.refused(reason)),
            };
        }
        hasher.update(&buf[..read]);
        self.hashing = Some(hasher);
        self.position += read as u64;
        Ok(read)
    }
}

impl Read for Guard {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.shared.cancelled() {
            // Not `Interrupted`: `read_exact` retries that kind forever.
            return Err(io::Error::other("the push was cancelled"));
        }
        if let Some(reason) = self.shared.refusal() {
            return Err(io::Error::other(reason));
        }
        if let Some(hasher) = self.hashing.take() {
            // The hashing pass: it reads the file from its start to its end, once.
            return self.hashing_read(hasher, buf);
        }
        let read = match self.source().read(buf) {
            Ok(read) => read,
            Err(error) => {
                return Err(self.refused(format!("the source could not be read: {error}")));
            }
        };
        if read == 0 {
            return Err(self.refused("the source got shorter while it was being sent".to_owned()));
        }
        self.position += read as u64;
        self.shared.sent(read as u64, self.position);
        Ok(read)
    }
}

impl Seek for Guard {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = self.source().seek(to)?;
        self.position = position;
        Ok(position)
    }
}
