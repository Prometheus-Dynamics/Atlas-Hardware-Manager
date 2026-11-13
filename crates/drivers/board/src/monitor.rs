//! One node, kept: a thread that connects, asks the node who it is, and keeps the connection.
//!
//! Orion's client is blocking and not `Send`, so each node's session lives on one thread of its
//! own, and the rest of Atlas reads what that thread reports through a watch channel. The
//! thread follows the failure rule: a connection that fails or breaks is tried again after a
//! delay that doubles up to the cap, and a success starts the delays again. It never gives up
//! by itself; dropping the [`Monitor`] (or turning the node off in the driver) is the only way
//! it stops. There is no timer of its own: it waits on the session, and a request for the
//! node's identity is answered on the same thread.
//!
//! Connections are security off (plain). A node that needs a key refuses them, and the reason
//! is kept as the presence.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orion::actions::{ActionCall, Target};
use orion::node::Node;
use orion::{Address, Options, Orion};
use tokio::sync::{oneshot, watch};

use crate::backoff::Backoff;
use atlas_types::LemnosState;

use crate::lemnos;

/// The program name this driver connects as.
pub const PROGRAM: &str = "atlas";
/// How long one wait on the session lasts before the thread looks at its requests again.
const POLL: Duration = Duration::from_millis(100);
/// How long one connect and its hello may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a request for the node's identity waits for the session to answer.
const ASK_WAIT: Duration = Duration::from_secs(5);

/// What a node is doing, as its monitor last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Not tried yet.
    Searching,
    /// Connected and identified: the node's hello document as it answered.
    Found(Node),
    /// Not reachable now. The reason is the last error, in words.
    Lost(String),
}

/// A node's answer to a request for its identity: its hello document, and what its Lemnos is
/// doing now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub node: Node,
    pub lemnos: LemnosState,
}

type Ask = oneshot::Sender<Result<Answer, String>>;

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

/// The monitor of one node. Dropping it stops its thread.
#[derive(Debug)]
pub struct Monitor {
    handle: Handle,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What the rest of Atlas holds of a monitor: its presence, and a way to ask the node who it is
/// now. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Handle {
    address: Address,
    presence: watch::Receiver<Presence>,
    asks: mpsc::Sender<Ask>,
}

impl Handle {
    /// The address this monitor keeps.
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// What the node is doing now, as the last connect or answer saw it.
    pub fn presence(&self) -> Presence {
        self.presence.borrow().clone()
    }

    /// A receiver of presence changes, for code that waits on them.
    pub fn watch(&self) -> watch::Receiver<Presence> {
        self.presence.clone()
    }

    /// The node's identity as it answers now: its hello document, read again, and Lemnos's
    /// state. Facts a board publishes after it has connected show up here.
    ///
    /// # Errors
    ///
    /// If the session is not up, or does not answer in time. The message says why.
    pub async fn ask(&self) -> Result<Answer, String> {
        let (reply, answer) = oneshot::channel();
        self.asks
            .send(reply)
            .map_err(|_| "the monitor for this node has stopped".to_owned())?;
        match tokio::time::timeout(ASK_WAIT, answer).await {
            Ok(Ok(answered)) => answered,
            Ok(Err(_)) => Err("the session ended before it answered".into()),
            Err(_) => Err("the node did not answer in time".into()),
        }
    }
}

impl Monitor {
    /// Starts watching the node at `address`. The first try happens on the new thread.
    pub fn start(address: Address, pause: Arc<dyn Pause>) -> Self {
        let (sender, presence) = watch::channel(Presence::Searching);
        let (asks, asks_received) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_address, thread_stop) = (address.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name(format!("node-{address}"))
            .spawn(move || {
                let mut keeper = Keeper {
                    address: thread_address,
                    sender,
                    asks: asks_received,
                    stop: thread_stop,
                    pause,
                    expects_lemnos: false,
                };
                keeper.run();
            })
            .ok();
        Self {
            handle: Handle {
                address,
                presence,
                asks,
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
    asks: mpsc::Receiver<Ask>,
    stop: Arc<AtomicBool>,
    pause: Arc<dyn Pause>,
    /// Once a node has published Lemnos facts or offers, it is expected to answer for Lemnos
    /// from then on, even while its facts are gone (after its own restart).
    expects_lemnos: bool,
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
                            return;
                        }
                        self.sender.send_replace(Presence::Lost(reason));
                    }
                    Err(error) => {
                        self.sender.send_replace(Presence::Lost(error.to_string()));
                    }
                },
                Err(error) => {
                    self.sender.send_replace(Presence::Lost(error.to_string()));
                }
            }
            self.pause.pause(backoff.next_delay(), &self.stop);
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Keeps the session up until it breaks or the monitor is stopped, answering requests on
    /// the way. Returns why it broke.
    fn serve(&mut self, orion: &Orion) -> String {
        while !self.stopped() {
            while let Ok(reply) = self.asks.try_recv() {
                match self.answer(orion) {
                    Ok(answer) => {
                        self.sender
                            .send_replace(Presence::Found(answer.node.clone()));
                        let _ = reply.send(Ok(answer));
                    }
                    Err(error) => {
                        let text = error.to_string();
                        let _ = reply.send(Err(text.clone()));
                        if error.is_lost() {
                            return text;
                        }
                    }
                }
            }
            if let Err(error) = orion.pump(POLL) {
                return error.to_string();
            }
        }
        String::new()
    }

    /// Asks the node who it is, and, when it is expected to run Lemnos, whether Lemnos answers.
    fn answer(&mut self, orion: &Orion) -> Result<Answer, orion::Error> {
        let node = orion.node()?;
        if lemnos::published_by(&node) {
            self.expects_lemnos = true;
        }
        let lemnos = if self.expects_lemnos {
            probe(orion, &node)?
        } else {
            LemnosState::Absent
        };
        Ok(Answer { node, lemnos })
    }
}

/// Plain connections: security off, the only kind this build makes.
fn plain() -> Options {
    Options {
        timeout: CONNECT_TIMEOUT,
        ..Options::default()
    }
}

/// Asks Lemnos its read-only question. A session that breaks during it is an error; any other
/// failure is Lemnos not answering, with the reason.
fn probe(orion: &Orion, node: &Node) -> Result<LemnosState, orion::Error> {
    let Ok(name) = node.name.parse() else {
        return Ok(LemnosState::NotAnswering {
            reason: format!("the node's name {:?} is not a valid name", node.name),
        });
    };
    let answered = orion
        .run(&ActionCall {
            name: lemnos::PROBE,
            target: Target::Node(name),
            args: b"",
            deadline_ms: 0,
            detached: false,
        })
        .and_then(|mut run| Ok(run.finish(lemnos::PROBE_WAIT)?));
    let finished = match answered {
        Ok(finished) => finished,
        Err(error) if error.is_lost() => return Err(error),
        Err(error) => {
            return Ok(LemnosState::NotAnswering {
                reason: error.to_string(),
            });
        }
    };
    Ok(lemnos::from_probe(finished.outcome, &finished.detail))
}
