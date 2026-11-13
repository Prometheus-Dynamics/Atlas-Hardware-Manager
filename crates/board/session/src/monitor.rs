//! One node, kept: a thread that connects, reads the node's hello document, and keeps the
//! connection. Orion's client is blocking and not `Send`, so each node's session lives on one
//! thread of its own. The rest of Atlas reaches it through a [`Handle`]: it reads the node's
//! [`Presence`] from a watch channel, and runs calls on the live client with
//! [`Handle::with_client`].
//!
//! The thread follows the failure rule: a connection that fails or breaks is tried again after
//! a delay that doubles up to the cap ([`crate::backoff`]), and a success starts the delays
//! again. It never gives up by itself; dropping the [`Monitor`] is the only way it stops.
//! There is no timer of its own: it waits on the session, and calls are answered on the same
//! thread between pumps of the session.
//!
//! Connections are security off (plain). A node that needs a key refuses them, and the reason
//! is kept as the presence.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orion::node::Node;
use orion::{Address, Options, Orion};
use tokio::sync::{oneshot, watch};

use crate::backoff::Backoff;

/// The program name this crate connects as.
pub const PROGRAM: &str = "atlas";
/// How long one wait on the session lasts before the thread looks at its calls again.
const POLL: Duration = Duration::from_millis(100);
/// How long one connect and its hello may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a call waits for its answer by default, before the caller is told the node did not
/// answer. A [`Sessions`](crate::Sessions) may be built with another wait.
pub const CALL_WAIT: Duration = Duration::from_secs(5);

/// What a node is doing, as its monitor last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Not tried yet.
    Searching,
    /// Connected and identified: the node's hello document as it answered on connect.
    Found(Node),
    /// Not reachable now. The reason is the last error, in words.
    Lost(String),
}

/// Why a call on a node gave no answer. The two cases ask for different things.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The node is gone: not connected, the session broke, the monitor stopped, or the node did
    /// not answer in time. Retry later, under the failure rule. A call that timed out may still
    /// have run on the node; a caller that must know checks the node again.
    Gone(String),
    /// The call reached the node on a live session and failed there: a refusal, or an error
    /// from the node. Retrying the same call gives the same answer.
    Failed(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone(reason) => write!(f, "the node is gone: {reason}"),
            Self::Failed(reason) => write!(f, "the call failed: {reason}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// The wait between tries. The monitor asks it to wait; the real one sleeps, a test's records
/// the delay. It returns early once `stop` is set.
pub trait Pause: Send + Sync + fmt::Debug {
    fn pause(&self, delay: Duration, stop: &AtomicBool);
}

/// Sleeps for real, in slices short enough that a stop is noticed at once.
#[derive(Debug, Default)]
pub struct RealPause;

impl Pause for RealPause {
    fn pause(&self, delay: Duration, stop: &AtomicBool) {
        let end = Instant::now() + delay;
        while !stop.load(Ordering::SeqCst) {
            let now = Instant::now();
            if now >= end {
                return;
            }
            thread::sleep((end - now).min(POLL));
        }
    }
}

/// One call waiting for the monitor thread. It runs on the live session, or is refused when
/// there is none.
trait Job: Send {
    /// Runs the call on the live session and answers its caller. `Err` when the session broke
    /// during the call, with the reason.
    fn run(self: Box<Self>, orion: &Orion) -> Result<(), String>;
    /// Answers the caller without running the call: there is no session.
    fn refuse(self: Box<Self>, reason: &str);
}

struct Call<T, F> {
    call: F,
    reply: oneshot::Sender<Result<T, SessionError>>,
}

impl<T, F> Job for Call<T, F>
where
    T: Send + 'static,
    F: FnOnce(&Orion) -> Result<T, orion::Error> + Send + 'static,
{
    fn run(self: Box<Self>, orion: &Orion) -> Result<(), String> {
        let Self { call, reply } = *self;
        if reply.is_closed() {
            // The caller stopped waiting before its turn: the call is not started.
            return Ok(());
        }
        match call(orion) {
            Ok(value) => {
                let _ = reply.send(Ok(value));
                Ok(())
            }
            Err(error) if error.is_lost() => {
                let text = error.to_string();
                let _ = reply.send(Err(SessionError::Gone(text.clone())));
                Err(text)
            }
            Err(error) => {
                let _ = reply.send(Err(SessionError::Failed(error.to_string())));
                Ok(())
            }
        }
    }

    fn refuse(self: Box<Self>, reason: &str) {
        let _ = self.reply.send(Err(SessionError::Gone(reason.to_owned())));
    }
}

/// The monitor of one node. Dropping it stops its thread.
#[derive(Debug)]
pub struct Monitor {
    handle: Handle,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What the rest of Atlas holds of a monitor: the node's address and presence, and the way to
/// run calls on its live session. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Handle {
    address: Address,
    presence: watch::Receiver<Presence>,
    calls: mpsc::Sender<Box<dyn Job>>,
    call_wait: Duration,
}

impl Handle {
    /// The address this monitor keeps.
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// What the node is doing now, as the last connect saw it.
    pub fn presence(&self) -> Presence {
        self.presence.borrow().clone()
    }

    /// A receiver of presence changes, for code that waits on them.
    pub fn watch(&self) -> watch::Receiver<Presence> {
        self.presence.clone()
    }

    /// The same handle, with its calls waiting `call_wait` for their answer. For a call that is
    /// long by nature, such as a file push: the wait then bounds a node that does not answer,
    /// not how long the call may run. The call still ends when its session ends.
    #[must_use]
    pub fn waiting(&self, call_wait: Duration) -> Self {
        Self {
            call_wait,
            ..self.clone()
        }
    }

    /// Runs `call` on the node's live client and returns what it answers.
    ///
    /// The closure runs on the monitor's own thread, which is the only place the client may be
    /// used. It must not block for long: while it runs, the node's other calls wait. A
    /// long-running job belongs on the node as an Orion run that the caller polls.
    ///
    /// # Errors
    ///
    /// [`SessionError::Gone`] when the node is not connected now, the session breaks during the
    /// call, the monitor has stopped, or the node does not answer within the handle's call wait
    /// (`CALL_WAIT` unless the sessions were built with another). The
    /// closure's own error ends up as [`SessionError::Failed`] when the session is still up.
    pub async fn with_client<T, F>(&self, call: F) -> Result<T, SessionError>
    where
        T: Send + 'static,
        F: FnOnce(&Orion) -> Result<T, orion::Error> + Send + 'static,
    {
        match self.presence() {
            Presence::Found(_) => {}
            Presence::Lost(reason) => return Err(SessionError::Gone(reason)),
            Presence::Searching => {
                return Err(SessionError::Gone("not connected yet".into()));
            }
        }
        let (reply, answer) = oneshot::channel();
        let job: Box<dyn Job> = Box::new(Call { call, reply });
        if self.calls.send(job).is_err() {
            return Err(SessionError::Gone(
                "the monitor for this node has stopped".into(),
            ));
        }
        match tokio::time::timeout(self.call_wait, answer).await {
            Ok(Ok(answered)) => answered,
            Ok(Err(_)) => Err(SessionError::Gone(
                "the session ended before it answered".into(),
            )),
            Err(_) => Err(SessionError::Gone("the node did not answer in time".into())),
        }
    }
}

impl Monitor {
    /// Starts watching the node at `address`. The first try happens on the new thread. Calls on
    /// the handle wait `call_wait` for their answer.
    pub fn start(address: Address, pause: Arc<dyn Pause>, call_wait: Duration) -> Self {
        let (sender, presence) = watch::channel(Presence::Searching);
        let (calls, received) = mpsc::channel::<Box<dyn Job>>();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_address, thread_stop) = (address.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name(format!("node-{address}"))
            .spawn(move || {
                let mut keeper = Keeper {
                    address: thread_address,
                    sender,
                    calls: received,
                    stop: thread_stop,
                    pause,
                };
                keeper.run();
            })
            .ok();
        Self {
            handle: Handle {
                address,
                presence,
                calls,
                call_wait,
            },
            stop,
            thread,
        }
    }

    /// What the rest of Atlas holds of this monitor.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The state the monitor thread keeps across connections.
struct Keeper {
    address: Address,
    sender: watch::Sender<Presence>,
    calls: mpsc::Receiver<Box<dyn Job>>,
    stop: Arc<AtomicBool>,
    pause: Arc<dyn Pause>,
}

impl Keeper {
    fn run(&mut self) {
        let mut backoff = Backoff::default();
        while !self.stopped() {
            match Orion::connect_with(&self.address, PROGRAM, plain()) {
                Ok(orion) => match orion.node() {
                    Ok(node) => {
                        backoff.reset();
                        self.sender.send_replace(Presence::Found(node));
                        let reason = self.serve(&orion);
                        if self.stopped() {
                            break;
                        }
                        self.sender.send_replace(Presence::Lost(reason));
                    }
                    Err(error) => self.lost(error.to_string()),
                },
                Err(error) => self.lost(error.to_string()),
            }
            self.refuse_waiting("the node is not connected");
            self.pause.pause(backoff.next_delay(), &self.stop);
        }
        self.refuse_waiting("the monitor for this node has stopped");
    }

    fn lost(&self, reason: String) {
        self.sender.send_replace(Presence::Lost(reason));
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Keeps the session up until it breaks or the monitor is stopped, running calls on the way.
    /// Returns why it broke.
    fn serve(&mut self, orion: &Orion) -> String {
        while !self.stopped() {
            while let Ok(job) = self.calls.try_recv() {
                if let Err(reason) = job.run(orion) {
                    return reason;
                }
            }
            if let Err(error) = orion.pump(POLL) {
                return error.to_string();
            }
        }
        String::new()
    }

    /// Answers every call that is waiting with `reason`, so none is run later by surprise.
    fn refuse_waiting(&mut self, reason: &str) {
        while let Ok(job) = self.calls.try_recv() {
            job.refuse(reason);
        }
    }
}

/// Plain connections: security off, the only kind this build makes.
fn plain() -> Options {
    Options {
        timeout: CONNECT_TIMEOUT,
        ..Options::default()
    }
}
