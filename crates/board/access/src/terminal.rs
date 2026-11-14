//! A terminal on a board: an interactive byte stream a person types into and reads from.
//!
//! It is the only thing this crate hands out that runs on a board, and it is interactive only:
//! there is no call that runs a command and returns its output. A terminal ends once, with a
//! reason: the shell exited, the link broke, the session to the node was lost, or the person
//! closed it. A lost session ends it at once; nothing reopens a shell on its own. A new shell is
//! a person's choice, made by calling [`open`] again.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use atlas_session::{Handle, Hold, Presence};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::carrier::{Carrier, Control, Output, OutputEvent, ShellRequest};
use crate::error::AccessError;
use crate::kind::{Identity, ShellKind, current, pinned, state_of};

/// The longest terminal kind name a shell is asked for.
const MAX_TERM: usize = 64;

/// What a person asks for when opening a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    /// The terminal's width in characters. Must not be zero.
    pub cols: u16,
    /// The terminal's height in characters. Must not be zero.
    pub rows: u16,
    /// The terminal's kind, as `TERM` names it: letters, digits and `-_.+`.
    pub term: String,
    /// The node key this board was known by. A node that now shows another key is refused, and
    /// nothing is opened. `None` when the caller does not know the key.
    pub expected_key: Option<[u8; 32]>,
}

impl OpenRequest {
    /// A request for a terminal of this size, of the usual kind, with no key pinned.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            term: "xterm-256color".into(),
            expected_key: None,
        }
    }
}

/// How a terminal ended. It ends once: the first reason wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The shell exited with this status.
    Exited(i32),
    /// The session to the node was lost, with the session's words. Nothing reopens the shell.
    Lost(String),
    /// The address now answers with another node key: the board was replaced. Nothing is sent to
    /// the other node, and nothing reopens the shell on it. Distinct from [`End::Lost`]: the board
    /// that was open is not the one that answers now.
    Replaced(String),
    /// The link to the shell broke while the node was still connected, with its words.
    Broke(String),
    /// The session was silent for longer than the board's lease while this shell's hold existed:
    /// the board may have dropped the shell. Nothing reopens it; a new shell is a person's choice.
    HoldLapsed(String),
    /// The person closed the terminal.
    Closed,
}

/// What a terminal reports, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Bytes the shell printed.
    Output(Vec<u8>),
    /// The terminal has ended. Nothing follows it.
    Ended(End),
}

/// The one place a terminal's end is decided, and the channel its events go out on.
#[derive(Debug)]
struct Finish {
    end: Mutex<Option<End>>,
    events: mpsc::UnboundedSender<Event>,
    control: Arc<dyn Control>,
    /// The shell's hold on the board. It lives from the open to the end, and is dropped with it.
    hold: Mutex<Option<Hold>>,
}

impl Finish {
    /// Ends the terminal with `end`, unless it has already ended: the link is hung up and the
    /// end is reported once.
    fn finish(&self, end: End) {
        let mut slot = self.end.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_some() {
            return;
        }
        *slot = Some(end.clone());
        // The board no longer needs this shell's hold: its lease need not be kept for it.
        self.hold
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.control.hang_up();
        let _ = self.events.send(Event::Ended(end));
    }

    fn ended(&self) -> Option<End> {
        self.end
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A terminal open on a node's shell. Dropping it hangs the shell up.
#[derive(Debug)]
pub struct Terminal {
    kind: ShellKind,
    identity: Identity,
    control: Arc<dyn Control>,
    finish: Arc<Finish>,
    events: mpsc::UnboundedReceiver<Event>,
    watchers: Vec<JoinHandle<()>>,
    done: bool,
}

impl Terminal {
    /// Whether this is the normal shell or the recovery shell, as the node said.
    #[must_use]
    pub const fn kind(&self) -> ShellKind {
        self.kind
    }

    /// The node this shell is on, identified by its signed hello. There is no separate question
    /// to answer about its key: a changed key is refused when it is opened.
    #[must_use]
    pub const fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Sends bytes to the shell's input.
    ///
    /// # Errors
    ///
    /// [`AccessError::Stream`] if the terminal has ended or the bytes could not be sent.
    pub async fn send(&self, bytes: &[u8]) -> Result<(), AccessError> {
        self.check_open()?;
        let (control, data) = (self.control.clone(), bytes.to_vec());
        tokio::task::spawn_blocking(move || control.send(&data))
            .await
            .map_err(|error| AccessError::Stream(error.to_string()))?
            .map_err(AccessError::Stream)
    }

    /// Tells the shell its terminal is now this size.
    ///
    /// # Errors
    ///
    /// [`AccessError::BadRequest`] for a zero size, [`AccessError::Stream`] if the terminal has
    /// ended or the size could not be sent.
    pub async fn resize(&self, cols: u16, rows: u16) -> Result<(), AccessError> {
        check_size(cols, rows)?;
        self.check_open()?;
        let control = self.control.clone();
        tokio::task::spawn_blocking(move || control.resize(cols, rows))
            .await
            .map_err(|error| AccessError::Stream(error.to_string()))?
            .map_err(AccessError::Stream)
    }

    /// The next thing the terminal reports: output, then its end. After the end this returns
    /// `None`, and it does not wait for anything more.
    pub async fn recv(&mut self) -> Option<Event> {
        if self.done {
            return None;
        }
        let event = self.events.recv().await?;
        if matches!(event, Event::Ended(_)) {
            self.done = true;
            self.stop_watching();
        }
        Some(event)
    }

    /// Ends the terminal as the person closed it. What was printed before is still delivered
    /// by [`Terminal::recv`], then the end, [`End::Closed`].
    pub fn close(&mut self) {
        self.finish.finish(End::Closed);
    }

    fn check_open(&self) -> Result<(), AccessError> {
        match self.finish.ended() {
            None => Ok(()),
            Some(end) => Err(AccessError::Stream(format!("the shell has ended: {end:?}"))),
        }
    }

    fn stop_watching(&mut self) {
        for watcher in self.watchers.drain(..) {
            watcher.abort();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.finish.finish(End::Closed);
        self.stop_watching();
    }
}

/// Opens a terminal on the node `handle` keeps, through `carrier`.
///
/// The node must be connected now, its hello must say whether it runs or is in recovery, and
/// its key must be the one `request` names if it names one. Only then is a shell asked for.
///
/// # Errors
///
/// - [`AccessError::Gone`] if the node is not connected;
/// - [`AccessError::KeyChanged`] if its key is not the one the caller knew;
/// - [`AccessError::NoState`] or [`AccessError::UnknownState`] if its hello does not say a state
///   this crate knows;
/// - [`AccessError::BadRequest`] for a zero size or a terminal kind that is not plain;
/// - [`AccessError::Refused`] if the carrier or the node refused.
pub async fn open(
    handle: &Handle,
    carrier: Arc<dyn Carrier>,
    request: OpenRequest,
) -> Result<Terminal, AccessError> {
    let node = current(handle).await?;
    let identity = Identity::of(&node);
    pinned(&identity, request.expected_key)?;
    let kind = ShellKind::for_state(state_of(&node)?);
    check_size(request.cols, request.rows)?;
    check_term(&request.term)?;
    let shell = ShellRequest {
        kind,
        cols: request.cols,
        rows: request.rows,
        term: request.term,
    };
    let channel = tokio::task::spawn_blocking(move || carrier.open_shell(&shell))
        .await
        .map_err(|error| AccessError::Refused(error.to_string()))?
        .map_err(AccessError::Refused)?;

    let (events, received) = mpsc::unbounded_channel();
    let finish = Arc::new(Finish {
        end: Mutex::new(None),
        events,
        control: channel.control.clone(),
        hold: Mutex::new(Some(handle.hold("an open terminal"))),
    });
    spawn_reader(channel.output, finish.clone());
    let presence = tokio::spawn(watch_presence(handle.watch(), finish.clone()));
    let lapses = tokio::spawn(watch_lapses(handle.lapses(), finish.clone()));
    Ok(Terminal {
        kind,
        identity,
        control: channel.control,
        finish,
        events: received,
        watchers: vec![presence, lapses],
        done: false,
    })
}

fn check_size(cols: u16, rows: u16) -> Result<(), AccessError> {
    if cols == 0 || rows == 0 {
        return Err(AccessError::BadRequest("the terminal has no size"));
    }
    Ok(())
}

fn check_term(term: &str) -> Result<(), AccessError> {
    let plain = !term.is_empty()
        && term.len() <= MAX_TERM
        && term
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte));
    if !plain {
        return Err(AccessError::BadRequest(
            "the terminal kind is not a plain name",
        ));
    }
    Ok(())
}

/// Reads the shell's output on a thread of its own, since reading blocks.
fn spawn_reader(mut output: Box<dyn Output>, finish: Arc<Finish>) {
    thread::spawn(move || {
        loop {
            match output.next() {
                OutputEvent::Data(bytes) => {
                    let _ = finish.events.send(Event::Output(bytes));
                }
                OutputEvent::Exit(code) => {
                    finish.finish(End::Exited(code));
                    return;
                }
                OutputEvent::Broke(words) => {
                    finish.finish(End::Broke(words));
                    return;
                }
            }
        }
    });
}

/// Ends the terminal when the session lapsed while its hold existed: the board may have dropped the
/// shell with the lease. A lapse is seen once; the terminal then ends.
async fn watch_lapses(mut lapses: watch::Receiver<u64>, finish: Arc<Finish>) {
    if lapses.changed().await.is_ok() {
        finish.finish(End::HoldLapsed(
            "the board may have dropped this shell: no request reached its bridge within the lease. Nothing reopens it".into(),
        ));
    }
}

/// Ends the terminal the moment the node is no longer connected. It never waits for a
/// reconnect: the shell does not come back by itself.
async fn watch_presence(mut presence: watch::Receiver<Presence>, finish: Arc<Finish>) {
    loop {
        let ended = match &*presence.borrow_and_update() {
            Presence::Lost(reason) => Some(End::Lost(reason.clone())),
            Presence::Replaced { .. } => Some(End::Replaced(
                "the board was replaced: its address now answers with another node key".into(),
            )),
            Presence::Searching | Presence::Found(_) => None,
        };
        if let Some(end) = ended {
            finish.finish(end);
            return;
        }
        if presence.changed().await.is_err() {
            finish.finish(End::Lost("the monitor for this node has stopped".into()));
            return;
        }
    }
}
