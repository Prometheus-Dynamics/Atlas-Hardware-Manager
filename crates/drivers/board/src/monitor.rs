//! One node, kept: a thread that connects, says who the node is, and keeps the connection.
//!
//! Orion's client is blocking and not `Send`, so each node's session lives on one thread of
//! its own, and the rest of Atlas reads what that thread reports through a watch channel. The
//! thread follows the failure rule: a connection that fails or breaks is tried again after a
//! delay that doubles up to the cap, and a success starts the delays again. It never gives up
//! by itself; [`Monitor`] dropped (or turned off by the driver) is the only way it stops.
//!
//! Every later PR that calls a node (readings, logs, actions, files, shell) runs on this same
//! thread, while its session is up.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use std::sync::mpsc;

use orion::node::Node;
use orion::{Address, Options, Orion};
use tokio::sync::{oneshot, watch};

use crate::backoff::Backoff;

/// The program name this driver connects as.
pub const PROGRAM: &str = "atlas";
/// How long one wait on the session lasts before the thread looks at its stop flag again.
const POLL: Duration = Duration::from_millis(100);
/// How long one connect and its hello may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a request for a fresh hello document waits for the session to answer.
const ASK_WAIT: Duration = Duration::from_secs(2);

/// A request for the node's hello document as it is now, answered on the session's thread.
type Ask = oneshot::Sender<Result<Node, String>>;

/// What a node is doing, as its monitor last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Not tried yet, or tried for the first time.
    Searching,
    /// Connected and identified: the node's hello document as it answered.
    Found(Node),
    /// Not reachable now. The reason is the last error, in words.
    Lost(String),
}

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

/// What the rest of Atlas holds of a monitor: its presence, and a way to ask the node for its
/// hello document now. Cheap to clone.
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

    /// What the node is doing now.
    pub fn presence(&self) -> Presence {
        self.presence.borrow().clone()
    }

    /// A receiver of presence changes, for code that waits on them.
    pub fn watch(&self) -> watch::Receiver<Presence> {
        self.presence.clone()
    }

    /// The node's hello document as it answers now. Facts a board publishes after it has
    /// connected show up here, which the presence snapshot does not.
    ///
    /// # Errors
    ///
    /// If the session is not up, or does not answer in time. The message says why.
    pub async fn ask(&self) -> Result<Node, String> {
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
                run(
                    &thread_address,
                    &sender,
                    &asks_received,
                    &thread_stop,
                    pause.as_ref(),
                );
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

fn run(
    address: &Address,
    sender: &watch::Sender<Presence>,
    asks: &mpsc::Receiver<Ask>,
    stop: &AtomicBool,
    pause: &dyn Pause,
) {
    let mut backoff = Backoff::default();
    let options = || Options {
        timeout: CONNECT_TIMEOUT,
        ..Options::default()
    };
    while !stop.load(Ordering::SeqCst) {
        match Orion::connect_with(address, PROGRAM, options()) {
            Ok(orion) => match orion.node() {
                Ok(node) => {
                    backoff.reset();
                    sender.send_replace(Presence::Found(node));
                    let reason = serve(&orion, sender, asks, stop);
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    sender.send_replace(Presence::Lost(reason));
                }
                Err(error) => {
                    sender.send_replace(Presence::Lost(error.to_string()));
                }
            },
            Err(error) => {
                sender.send_replace(Presence::Lost(error.to_string()));
            }
        }
        pause.pause(backoff.next_delay(), stop);
    }
}

/// Keeps the session up until it breaks or the monitor is stopped, answering requests for the
/// node's hello document on the way. Returns why it broke.
fn serve(
    orion: &Orion,
    sender: &watch::Sender<Presence>,
    asks: &mpsc::Receiver<Ask>,
    stop: &AtomicBool,
) -> String {
    while !stop.load(Ordering::SeqCst) {
        while let Ok(reply) = asks.try_recv() {
            match orion.node() {
                Ok(node) => {
                    sender.send_replace(Presence::Found(node.clone()));
                    let _ = reply.send(Ok(node));
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
