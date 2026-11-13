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
//! Connections are signed with this monitor's own client key, which is generated when the
//! monitor starts and is not kept anywhere: a node with security on accepts a signed client, a
//! plain one it refuses.
//!
//! # The key a node was seen with
//!
//! A session pins the node key it first saw at its address for its whole life. Every later
//! connect is made to the address in Orion's pin form (`orion+tcp://host#<key>`), so Orion
//! refuses a different key during the handshake, before any call is made. A node that answers
//! with another key is never sent a call and never becomes [`Presence::Found`]: presence is
//! [`Presence::Replaced`] while the monitor keeps looking for the expected node. The expected
//! node answering again makes the session Found again. Only a person's explicit act, through
//! [`Monitor::accept`] (or [`crate::Sessions::accept_key`]), makes another key the new expected
//! one, and that starts a fresh session.
//!
//! A monitor may start with a key the caller already knows ([`Monitor::start_pinned`]): its
//! first connect is pinned too. Only an address that has never been seen, and has no known key,
//! is connected unpinned (trust on first use): that first contact is the one unpinned moment.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orion::node::Node;
use orion::{Address, ClientError, Identity, Options, Orion};
use tokio::sync::{oneshot, watch};

use crate::backoff::Backoff;
use crate::long::{
    Cancel, MAX_LONG_OPERATIONS, Outcome, STEP_WAIT, Step, StepError, SteppedTransfer,
};

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
    /// Not tried yet, or re-pinned by a person and not yet connected again.
    Searching,
    /// Connected and identified: the node's hello document as it answered on connect. Its key is
    /// the session's pin.
    Found(Node),
    /// Not reachable now. The reason is the last error, in words.
    Lost(String),
    /// The address answers with a node key other than the one this session is pinned to. No
    /// call is sent to that node. `expected` is the pinned key. `found` is the key the other node
    /// showed when Orion says it; `None` at the locked Orion revision, whose refusal does not carry
    /// it. It holds until the expected node answers again or a person accepts the new key.
    Replaced {
        /// The key this session was pinned to.
        expected: [u8; 32],
        /// The key the other node showed, if known.
        found: Option<[u8; 32]>,
    },
}

/// The most frames the session takes between two turns' steps while an operation runs. Each
/// frame is a few microseconds; the cap keeps a flood of answers from starving the steps.
const MAX_FRAMES_PER_TURN: usize = 64;

/// The reason a long operation gives when a person changes the node's key under it.
const CHANGED_BY_A_PERSON: &str = "the node's key was changed by a person";

/// The words Orion's client gives when a pinned connect meets another key (Orion at 6dfb990,
/// `orion_connect` transport).
// LOCK: at 6dfb990 the refusal is `ClientError::Denied` with these words and no key. The lock
// rev after it returns `ClientError::OtherKey { asked, answered }`; match that here, and fill
// `found` from `answered`.
const OTHER_KEY: &str = "the node is not the one that was expected";

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
/// there is none. A long operation's start is a job too: it runs once, and then waits in `ops`.
trait Job: Send {
    /// Runs the call on the live session and answers its caller. `Err` when the session broke
    /// during the call, with the reason. A long operation adds itself to `ops`.
    fn run(self: Box<Self>, orion: &Orion, ops: &mut Vec<Box<dyn Op>>) -> Result<(), String>;
    /// Answers the caller without running the call: there is no session.
    fn refuse(self: Box<Self>, reason: &str);
    /// Whether this job starts a long operation, which counts against [`MAX_LONG_OPERATIONS`].
    fn is_long(&self) -> bool {
        false
    }
}

/// A long operation running on the session, advanced one turn at a time.
trait Op {
    /// Takes one step (or notices a cancel), and says what became of the operation.
    fn turn(&mut self) -> Turn;
    /// The session ends under the operation: its caller is told `reason`.
    fn end(self: Box<Self>, reason: &str);
}

/// What one turn of an operation did.
enum Turn {
    /// Still going.
    Stepped,
    /// Finished, and its caller has been answered.
    Finished,
    /// The session broke during its step. Its caller has been answered; the session ends.
    Lost(String),
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
    fn run(self: Box<Self>, orion: &Orion, _ops: &mut Vec<Box<dyn Op>>) -> Result<(), String> {
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

/// A long operation's start, waiting for the session thread. Its operation is kept in `ops`
/// from then on.
struct Long<T, F> {
    start: F,
    reply: oneshot::Sender<Result<Outcome<T>, SessionError>>,
    cancel: Cancel,
}

impl<T, F> Job for Long<T, F>
where
    T: Send + 'static,
    F: FnOnce(&Orion) -> Result<Box<dyn SteppedTransfer<T>>, orion::Error> + Send + 'static,
{
    fn run(self: Box<Self>, orion: &Orion, ops: &mut Vec<Box<dyn Op>>) -> Result<(), String> {
        let Self {
            start,
            reply,
            cancel,
        } = *self;
        if reply.is_closed() {
            return Ok(());
        }
        if cancel.is_cancelled() {
            let _ = reply.send(Ok(Outcome::Cancelled));
            return Ok(());
        }
        match start(orion) {
            Ok(transfer) => {
                ops.push(Box::new(Running {
                    transfer,
                    reply: Some(reply),
                    cancel,
                }));
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

    fn is_long(&self) -> bool {
        true
    }
}

/// A started long operation: its transfer, its caller's answer, and its cancel request.
struct Running<T> {
    transfer: Box<dyn SteppedTransfer<T>>,
    reply: Option<oneshot::Sender<Result<Outcome<T>, SessionError>>>,
    cancel: Cancel,
}

impl<T: Send + 'static> Op for Running<T> {
    fn turn(&mut self) -> Turn {
        let Some(reply) = self.reply.take() else {
            return Turn::Finished;
        };
        // A cancel, or a caller that stopped waiting, lands here: at the boundary, never mid-step.
        if self.cancel.is_cancelled() || reply.is_closed() {
            self.transfer.cancel();
            let _ = reply.send(Ok(Outcome::Cancelled));
            return Turn::Finished;
        }
        match self.transfer.step(STEP_WAIT) {
            Ok(Step::Progress) => {
                self.reply = Some(reply);
                Turn::Stepped
            }
            Ok(Step::Done(value)) => {
                let _ = reply.send(Ok(Outcome::Done(value)));
                Turn::Finished
            }
            Err(StepError::Failed(reason)) => {
                let _ = reply.send(Err(SessionError::Failed(reason)));
                Turn::Finished
            }
            Err(StepError::Lost(reason)) => {
                let _ = reply.send(Err(SessionError::Gone(reason.clone())));
                Turn::Lost(reason)
            }
        }
    }

    fn end(mut self: Box<Self>, reason: &str) {
        if let Some(reply) = self.reply.take() {
            self.transfer.cancel();
            let _ = reply.send(Err(SessionError::Gone(reason.to_owned())));
        }
    }
}

/// A person's order to a monitor. Orders are taken between connects and between a session's
/// calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    /// This address now means this key: the session is pinned to it and starts fresh.
    Accept([u8; 32]),
}

/// The monitor of one node. Dropping it stops its thread.
#[derive(Debug)]
pub struct Monitor {
    handle: Handle,
    orders: mpsc::Sender<Order>,
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

    /// How long a call on this node waits for its answer (see [`CALL_WAIT`]).
    pub const fn call_wait(&self) -> Duration {
        self.call_wait
    }

    /// What the node is doing now, as the last connect saw it.
    pub fn presence(&self) -> Presence {
        self.presence.borrow().clone()
    }

    /// A receiver of presence changes, for code that waits on them.
    pub fn watch(&self) -> watch::Receiver<Presence> {
        self.presence.clone()
    }

    /// The key of the node this session is connected to now, if it is connected. A caller that
    /// resumes work on a node compares this with the key the work started on: a different key is
    /// a different node.
    pub fn node_key(&self) -> Option<[u8; 32]> {
        match &*self.presence.borrow() {
            Presence::Found(node) => Some(node.public_key),
            Presence::Searching | Presence::Lost(_) | Presence::Replaced { .. } => None,
        }
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
    /// used. It must not block for long: while it runs, the node's other calls wait. A transfer
    /// that takes long is a long operation ([`Handle::long`]), which the session advances in steps.
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
        self.ready()?;
        let (reply, answer) = oneshot::channel();
        let job: Box<dyn Job> = Box::new(Call { call, reply });
        self.queue(job)?;
        self.answer(answer).await?
    }

    /// Starts a long operation on the node's live session and waits for how it ends.
    ///
    /// `start` runs on the session thread, like a call's closure, and returns the transfer. The
    /// session then advances it one step at a time, between its other calls (see
    /// [`crate::long`] for the fairness rule). The call answers [`Outcome::Done`] when the
    /// transfer finishes, [`Outcome::Cancelled`] when `cancel` was made, or an error.
    ///
    /// Dropping the returned future cancels the operation at its next step boundary. The wait
    /// for the answer is the handle's call wait: a long operation takes a handle made with
    /// [`Handle::waiting`] that allows for its length.
    ///
    /// # Errors
    ///
    /// As [`Handle::with_client`]: [`SessionError::Gone`] when the node is not connected, the
    /// session breaks during the operation (resume on a later session), or the operation waits
    /// past the handle's call wait. [`SessionError::Failed`] when the transfer fails for good on
    /// a live session.
    pub async fn long<T, F>(&self, cancel: &Cancel, start: F) -> Result<Outcome<T>, SessionError>
    where
        T: Send + 'static,
        F: FnOnce(&Orion) -> Result<Box<dyn SteppedTransfer<T>>, orion::Error> + Send + 'static,
    {
        self.ready()?;
        let (reply, answer) = oneshot::channel();
        let job: Box<dyn Job> = Box::new(Long {
            start,
            reply,
            cancel: cancel.clone(),
        });
        self.queue(job)?;
        self.answer(answer).await?
    }

    /// Whether a call may be queued now: the node must be connected under its pinned key.
    fn ready(&self) -> Result<(), SessionError> {
        match self.presence() {
            Presence::Found(_) => Ok(()),
            Presence::Lost(reason) => Err(SessionError::Gone(reason)),
            Presence::Searching => Err(SessionError::Gone("not connected yet".into())),
            Presence::Replaced { .. } => Err(SessionError::Gone(REPLACED_WORDS.into())),
        }
    }

    fn queue(&self, job: Box<dyn Job>) -> Result<(), SessionError> {
        self.calls
            .send(job)
            .map_err(|_| SessionError::Gone("the monitor for this node has stopped".into()))
    }

    async fn answer<T>(&self, answer: oneshot::Receiver<T>) -> Result<T, SessionError> {
        match tokio::time::timeout(self.call_wait, answer).await {
            Ok(Ok(answered)) => Ok(answered),
            Ok(Err(_)) => Err(SessionError::Gone(
                "the session ended before it answered".into(),
            )),
            Err(_) => Err(SessionError::Gone("the node did not answer in time".into())),
        }
    }
}

impl Monitor {
    /// Starts watching the node at `address`, with no key known yet: the first node that answers
    /// is taken as the expected one (trust on first use). The first try happens on the new
    /// thread. Calls on the handle wait `call_wait` for their answer.
    pub fn start(address: Address, pause: Arc<dyn Pause>, call_wait: Duration) -> Self {
        Self::start_with_pin(address, None, pause, call_wait)
    }

    /// As [`Monitor::start`], for a node whose key the caller already knows: the first connect is
    /// pinned to `key`, so a different node at the address is refused from the start.
    pub fn start_pinned(
        address: Address,
        key: [u8; 32],
        pause: Arc<dyn Pause>,
        call_wait: Duration,
    ) -> Self {
        Self::start_with_pin(address, Some(key), pause, call_wait)
    }

    fn start_with_pin(
        address: Address,
        pin: Option<[u8; 32]>,
        pause: Arc<dyn Pause>,
        call_wait: Duration,
    ) -> Self {
        let (sender, presence) = watch::channel(Presence::Searching);
        let (calls, received) = mpsc::channel::<Box<dyn Job>>();
        let (orders, taken) = mpsc::channel::<Order>();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_address, thread_stop) = (address.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name(format!("node-{address}"))
            .spawn(move || {
                let mut keeper = Keeper {
                    address: thread_address,
                    sender,
                    calls: received,
                    orders: taken,
                    stop: thread_stop,
                    pause,
                    secret: Identity::generate().secret(),
                    pin,
                    replacement: None,
                    parked: VecDeque::new(),
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
            orders,
            stop,
            thread,
        }
    }

    /// What the rest of Atlas holds of this monitor.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// A person's act: this address now means `key`. The session is pinned to it from now on,
    /// the old session's pin ends, and a session with the old node (if one is up) is dropped.
    /// Nothing calls this on its own. Returns `false` if the monitor has stopped.
    ///
    /// The new session is a new identity: nothing that was running on the old node resumes on
    /// this one. A caller that resumes work compares [`Handle::node_key`] with the key it started
    /// on.
    pub fn accept(&self, key: [u8; 32]) -> bool {
        self.orders.send(Order::Accept(key)).is_ok()
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

/// What a call gets while the address answers with another key.
const REPLACED_WORDS: &str = "the node at this address was replaced: it answers with another node key, and nothing is sent to it";

/// The state the monitor thread keeps across connections.
struct Keeper {
    address: Address,
    sender: watch::Sender<Presence>,
    calls: mpsc::Receiver<Box<dyn Job>>,
    orders: mpsc::Receiver<Order>,
    stop: Arc<AtomicBool>,
    pause: Arc<dyn Pause>,
    /// This monitor's client key, as its secret bytes: a connect makes its identity from them.
    secret: [u8; 32],
    /// The node key this session is pinned to. `None` until the first node answers.
    pin: Option<[u8; 32]>,
    /// Set while another key answers at the address: what it showed, if known.
    replacement: Option<Option<[u8; 32]>>,
    /// Long operations waiting for a slot, in the order they came. Never stepped while parked.
    parked: VecDeque<Box<dyn Job>>,
}

impl Keeper {
    fn run(&mut self) {
        let mut backoff = Backoff::default();
        while !self.stopped() {
            self.take_orders();
            match self.connect() {
                Ok(orion) => match orion.node() {
                    Ok(node) if self.accepts(&node) => {
                        // Trust on first use only when nothing is pinned yet. From here the
                        // node that answers is the expected one.
                        self.pin = Some(node.public_key);
                        self.replacement = None;
                        backoff.reset();
                        self.sender.send_replace(Presence::Found(node));
                        let ended = self.serve(&orion);
                        if self.stopped() {
                            break;
                        }
                        match ended {
                            Some(reason) => self.lost(reason),
                            // A person accepted another key: start again at once.
                            None => {
                                // Calls queued for the old node are not run on the new one.
                                self.refuse_waiting("the node's key was changed by a person");
                                backoff.reset();
                                continue;
                            }
                        }
                    }
                    // Orion refuses a different key in the handshake, so this is not expected. It
                    // is still refused here, before any call is made on the session.
                    Ok(node) => self.replaced(Some(node.public_key)),
                    Err(error) => self.lost(error.to_string()),
                },
                Err(error) if self.is_other_key(&error) => self.replaced(None),
                Err(error) => self.lost(error.to_string()),
            }
            let down = if self.replacement.is_some() {
                REPLACED_WORDS
            } else {
                "the node is not connected"
            };
            self.refuse_waiting(down);
            self.pause.pause(backoff.next_delay(), &self.stop);
        }
        self.refuse_waiting("the monitor for this node has stopped");
    }

    /// Connects to the address, pinned to the key the session holds when there is one.
    fn connect(&self) -> Result<Orion, orion::Error> {
        let address = match &self.address {
            Address::Network { host, .. } => Address::Network {
                host: host.clone(),
                key: self.pin,
            },
            other => other.clone(),
        };
        let options = Options {
            timeout: CONNECT_TIMEOUT,
            identity: Some(Identity::from_secret(&self.secret)),
            ..Options::default()
        };
        Orion::connect_with(&address, PROGRAM, options)
    }

    /// Whether `node` is the node this session is pinned to. A node is accepted when nothing is
    /// pinned yet, or when its key is the pinned one.
    fn accepts(&self, node: &Node) -> bool {
        self.pin.is_none_or(|pin| pin == node.public_key)
    }

    /// Whether a pinned connect was refused because another key answered at the address.
    fn is_other_key(&self, error: &orion::Error) -> bool {
        self.pin.is_some()
            && matches!(
                error,
                orion::Error::Client(ClientError::Denied(why)) if *why == OTHER_KEY
            )
    }

    /// Takes the orders a person gave while no session was running.
    fn take_orders(&mut self) {
        while let Ok(order) = self.orders.try_recv() {
            self.apply(order);
        }
    }

    fn apply(&mut self, order: Order) {
        match order {
            Order::Accept(key) => {
                self.pin = Some(key);
                self.replacement = None;
                self.sender.send_replace(Presence::Searching);
            }
        }
    }

    /// Records that another key answers at the address.
    fn replaced(&mut self, found: Option<[u8; 32]>) {
        self.replacement = Some(found);
        self.show_replaced();
    }

    fn show_replaced(&self) {
        if let (Some(expected), Some(found)) = (self.pin, self.replacement) {
            self.sender
                .send_replace(Presence::Replaced { expected, found });
        }
    }

    /// The session is down for `reason`, unless another key is what answers the address.
    fn lost(&self, reason: String) {
        if self.replacement.is_some() {
            self.show_replaced();
        } else {
            self.sender.send_replace(Presence::Lost(reason));
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Keeps the session up until it breaks, the monitor is stopped, or a person accepts another
    /// key. Runs calls and long operations on the way. Returns why it broke, or `None` when it was
    /// re-pinned. Every operation still running ends with the same reason: none is stepped on a
    /// session after its own has ended.
    fn serve(&mut self, orion: &Orion) -> Option<String> {
        let mut ops: Vec<Box<dyn Op>> = Vec::new();
        let ended = self.turns(orion, &mut ops);
        let reason = ended
            .clone()
            .unwrap_or_else(|| CHANGED_BY_A_PERSON.to_owned());
        for op in ops {
            op.end(&reason);
        }
        ended
    }

    /// The session's turns, as described in [`crate::long`]: queued calls, then one step of each
    /// operation. Returns as [`Keeper::serve`] does.
    fn turns(&mut self, orion: &Orion, ops: &mut Vec<Box<dyn Op>>) -> Option<String> {
        while !self.stopped() {
            if let Ok(order) = self.orders.try_recv() {
                self.apply(order);
                return None;
            }
            // The calls of this turn: parked operations first (they waited for a slot), then
            // every call queued now. A call that arrives during the turn waits for the next one.
            let mut batch: VecDeque<Box<dyn Job>> = std::mem::take(&mut self.parked);
            while let Ok(job) = self.calls.try_recv() {
                batch.push_back(job);
            }
            while let Some(job) = batch.pop_front() {
                if job.is_long() && ops.len() >= MAX_LONG_OPERATIONS {
                    self.parked.push_back(job);
                    continue;
                }
                if let Err(reason) = job.run(orion, ops) {
                    // The session broke under this call: the rest of the turn is refused.
                    for rest in batch {
                        rest.refuse(&reason);
                    }
                    return Some(reason);
                }
            }
            let mut index = 0;
            while index < ops.len() {
                match ops[index].turn() {
                    Turn::Stepped => index += 1,
                    Turn::Finished => {
                        ops.remove(index);
                    }
                    Turn::Lost(reason) => {
                        ops.remove(index);
                        return Some(reason);
                    }
                }
            }
            // With nothing to step, the thread waits on the session for frames. Otherwise it only
            // takes the frames that are already here, at most MAX_FRAMES_PER_TURN, so a session
            // that broke is noticed between steps and a flood of frames cannot starve the steps.
            if ops.is_empty() {
                if let Err(error) = orion.pump(POLL) {
                    return Some(error.to_string());
                }
            } else {
                for _ in 0..MAX_FRAMES_PER_TURN {
                    match orion.pump(Duration::ZERO) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(error) => return Some(error.to_string()),
                    }
                }
            }
        }
        Some(String::new())
    }

    /// Answers every call that is waiting with `reason`, so none is run later by surprise.
    fn refuse_waiting(&mut self, reason: &str) {
        for job in std::mem::take(&mut self.parked) {
            job.refuse(reason);
        }
        while let Ok(job) = self.calls.try_recv() {
            job.refuse(reason);
        }
    }
}
