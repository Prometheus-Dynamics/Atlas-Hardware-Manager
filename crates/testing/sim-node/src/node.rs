//! The simulated node: an Orion comm core running in this process, with node logic that
//! answers the board's actions, topics and files the way a board does.
//!
//! Three parts, each one Orion's own code:
//! - the comm core is `orion-stubs`' [`Board`]: the real server, broker, hello and keys on a
//!   real socket and a real network port, with stand-ins for the record store, the action
//!   router, the file receiver and the workload tracker. A detached run survives its caller's
//!   session and can be attached to again; a cancel reaches the run only from the session and
//!   call that started it (the stub's router does both, as Orion's own does);
//! - the node logic is an `orion` client on that socket. It publishes the hello facts, registers
//!   every action in [`names::ACTIONS`], declares the topics, and answers each run;
//! - a loopback link proxy sits in front of the network port, so a connection can be cut
//!   mid-transfer or held still. That is the only piece here that is not Orion's.
//!
//! A slot install over Orion is `accept <size>`, a file push of the bundle into
//! [`names::INCOMING`] (the stub's file receiver holds it on the [`Shelf`]), then
//! `install_file <name> <pin> <keep_old>`, a long job. The job's progress is bundle bytes over
//! verify and unpack. The file is removed when the job ends, however it ends.
//!
//! Other runs are applied to the [`Model`] when they finish, never before, so a run that is
//! refused, fails or is cancelled before it finishes changes nothing.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orion::actions::{
    ActionCall, ActionDecl, Admits, Finished, Outcome, Policy, Registration, Request, Target,
};
use orion::topics::{FieldId, Publisher, SampleTime, TopicDecl};
use orion::{Address, Orion, fields};
use orion_stubs::{Board, Options, Security};

use crate::bundle;
use crate::lease;
use crate::model::Model;
use crate::names;
use crate::readings;
use crate::script::{Behaviour, LinkFault, Script};
use crate::slots::trial;
use crate::slots::{NewSlot, State, Status, code, valid_id};

/// The name the simulated board gives itself.
pub const NODE_NAME: &str = "board";
/// The contract ids of the devices a running sim node runs (Lemnos's names for them): its
/// identity, its reboot and its slots.
const OFFERS: &[&str] = &["system.identity/1", "system.reboot/1", "system.slots/1"];
/// How often the node logic moves runs on and publishes readings.
const TICK: Duration = Duration::from_millis(20);

/// The comm core's heartbeat: Orion's default, a session is asked to ping every two seconds.
const HEARTBEAT_MS: u32 = 2_000;
/// How long a session may go unheard before the comm core drops it. Orion's default is three
/// heartbeats (6 s), which a loaded test machine (load 80 and up) can exceed for a healthy session:
/// a starved process is not a lost one. Fifteen heartbeats keeps a test's session up through that.
/// A test that wants a session gone drops it itself (a dropped connection, or the node stopped).
const IDLE_MS: u32 = 30_000;
/// The one overall deadline for anything the sim waits on: a call's outcome, a run's end, a
/// condition a test polls for. Nothing here is a fixed delay standing in for "it should have
/// happened by now"; this only ever matters when something is truly stuck, so it is generous.
pub const DEADLINE: Duration = Duration::from_secs(60);
/// How many bundle bytes an install verifies and unpacks per tick.
pub const INSTALL_STEP: u64 = 64 * 1024;

/// The target every simulated action is registered and run against.
#[must_use]
pub fn target() -> Target {
    Target::Node(
        NODE_NAME
            .parse()
            .expect("the simulated node's name is a valid name"),
    )
}

/// Runs `name` with `args` (UTF-8 words) on the node `orion` is connected to, and waits for how
/// it ended.
///
/// # Errors
///
/// If the connection breaks. A refused or failed run is an `Ok` with its outcome.
pub fn call(orion: &Orion, name: &str, args: &[u8]) -> Result<Finished, orion::Error> {
    let mut run = orion.run(&ActionCall {
        name,
        target: target(),
        args,
        deadline_ms: 0,
        detached: false,
    })?;
    Ok(run.finish(DEADLINE)?)
}

/// The state a node is in, as its hello `state` fact says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Booted and running its own software.
    Running,
    /// Waiting in a recovery mode.
    Recovery,
    /// A freshly loaded flasher image.
    Flasher,
}

impl Mode {
    /// The word the `state` fact carries.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Recovery => "recovery",
            Self::Flasher => "flasher",
        }
    }
}

/// The facts the node publishes about itself, as a board's own program publishes them (see
/// [`names`] for each fact's owner). An empty text is a fact the device does not give: it is not
/// published, so Atlas reads it as not reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// `hello.family`: the board's name, `[a-z0-9-]`.
    pub family: String,
    /// `hello.serial`: lowercase hex, 8 to 32 digits.
    pub serial: String,
    /// `hello.state`, Orion's own fact: the mode the node is in.
    pub mode: Mode,
    /// `hello.model`.
    pub model: String,
    /// `hello.version.slot`: the running slot. It is written when the node starts, the way a boot
    /// writes it, and no confirm changes it.
    pub slot: String,
    /// `hello.version.product`.
    pub product: String,
    /// `hello.version.os`: the running slot's version.
    pub os_version: String,
    /// `hello.version.lemnos`: the daemon's version.
    pub lemnos_version: String,
    /// `hello.offers`: the contract ids of the devices running now. Published sorted, distinct
    /// and space-separated; changing this list is a live change, not a restart.
    pub offers: Vec<String>,
}

impl Default for Hello {
    /// A running simulated node: a simulated board with its slot, product and Lemnos, offering the
    /// contracts its devices run.
    fn default() -> Self {
        Self {
            family: "simulated".into(),
            serial: "0123456789abcdef".into(),
            mode: Mode::Running,
            model: "simulated-board".into(),
            slot: "slot-1".into(),
            product: "simulated-product".into(),
            os_version: "1.0.0".into(),
            lemnos_version: "1.0.0".into(),
            offers: OFFERS.iter().map(|offer| (*offer).to_string()).collect(),
        }
    }
}

impl Hello {
    /// Each hello topic that is published, and the text it carries. An empty text is left out.
    fn facts(&self) -> Vec<(&'static str, String)> {
        let mut offers = self.offers.clone();
        offers.sort();
        offers.dedup();
        let facts = [
            (names::HELLO_STATE, self.mode.word().to_string()),
            (names::HELLO_FAMILY, self.family.clone()),
            (names::HELLO_SERIAL, self.serial.clone()),
            (names::HELLO_MODEL, self.model.clone()),
            (names::HELLO_VERSION_SLOT, self.slot.clone()),
            (names::HELLO_VERSION_PRODUCT, self.product.clone()),
            (names::HELLO_VERSION_OS, self.os_version.clone()),
            (names::HELLO_VERSION_LEMNOS, self.lemnos_version.clone()),
            (names::HELLO_OFFERS, offers.join(" ")),
        ];
        facts
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect()
    }
}

/// What kind of Orion node this is. A node with Lemnos publishes its hardware facts and offers, and
/// serves the slot and reboot surface. A plain Orion node has none of that: it is normal (a PC, a
/// board of someone else's, a controller without our package) and still serves Orion's own things:
/// files, its own actions and its logs. Its identity is its key fingerprint from the hello document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A node with Lemnos, publishing these facts.
    WithLemnos(Hello),
    /// A plain Orion node: no hardware facts, no offers, no Lemnos actions or readings.
    Plain,
}

impl Default for Kind {
    fn default() -> Self {
        Self::WithLemnos(Hello::default())
    }
}

/// Whether a node of this kind registers `name`. A plain node has no Lemnos actions (so its slot and
/// reboot surface is absent: calls to it are not found), and only its own file actions.
fn offered(kind: &Kind, name: &str) -> bool {
    match kind {
        Kind::WithLemnos(_) => true,
        Kind::Plain => name.starts_with("fs."),
    }
}

/// The state shared between the test and the node logic.
#[derive(Clone, Debug)]
struct Shared {
    model: Arc<Mutex<Model>>,
    /// The hello facts as they are now. A test changes them with [`SimNode::change_hello`]; the
    /// node logic publishes the change. Only a node with Lemnos publishes them.
    hello: Arc<Mutex<Hello>>,
    script: Arc<Mutex<Script>>,
    stop: Arc<AtomicBool>,
    restart: Arc<AtomicBool>,
    /// Set while Lemnos is stopped on a node that has it: the node stays up and its Lemnos calls
    /// fail. That is not the same as Lemnos being absent.
    lemnos_down: Arc<AtomicBool>,
    /// Set while an install job is held where it is: it still runs, but moves no bytes until a
    /// test lets it go. Holding is how a test makes "the install is still running" true for as
    /// long as it needs, whatever the machine's load.
    hold_installs: Arc<AtomicBool>,
    /// How many runs have been stalled by the script (see [`Behaviour::Stall`]).
    stalled: Arc<AtomicUsize>,
    /// How many calls the node logic has been asked to start (every run request it received).
    calls: Arc<AtomicUsize>,
    /// Withdraws the device resources while set (see [`readings`]).
    devices: readings::Control,
    /// How many runs have been asked for, by action name, refused ones included.
    runs: Arc<Mutex<BTreeMap<String, usize>>>,
    /// The lease per caller identity, on a simulated clock (see [`crate::lease`]).
    leases: Arc<lease::Leases>,
}

/// A simulated board. Dropping it stops the node logic, the link and the comm core, and frees
/// its network port. Starting one again in the same directory on the same port brings it back
/// as the same device: a lost node is found again.
#[derive(Debug)]
pub struct SimNode {
    board: Arc<Board>,
    link: SocketAddr,
    shared: Shared,
    node: Option<JoinHandle<()>>,
    proxy: Option<JoinHandle<()>>,
}

impl SimNode {
    /// Starts a running node with Lemnos in `dir` on a free port. See [`SimNode::start_with`].
    ///
    /// # Errors
    ///
    /// As [`SimNode::start_with`].
    pub fn start(dir: &Path) -> Result<Self, String> {
        Self::start_with(dir, &Kind::default(), None)
    }

    /// Starts a simulated Orion node in `dir`, which is created if it is missing. Its socket lives
    /// there, so the path must stay short enough for a Unix socket. `kind` says whether it runs
    /// Lemnos ([`Kind::WithLemnos`], publishing that hello) or is plain Orion ([`Kind::Plain`]).
    /// It listens on `port` (any free port when `None`).
    ///
    /// # Errors
    ///
    /// If the directory cannot be made, the port cannot be taken, or the node logic cannot
    /// connect to the comm core or declare its facts.
    pub fn start_with(dir: &Path, kind: &Kind, port: Option<u16>) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let board = Arc::new(Board::start_with(
            dir,
            Options {
                security: Security::Off,
                port,
                // A node keeps its key across a restart in the same directory, as a board does.
                on_disk: true,
                heartbeat_ms: HEARTBEAT_MS,
                idle_ms: IDLE_MS,
                ..Options::default()
            },
        ));
        let hello = match kind {
            Kind::WithLemnos(hello) => hello.clone(),
            Kind::Plain => Hello::default(),
        };
        let shared = Shared {
            model: Arc::new(Mutex::new(Model::new())),
            hello: Arc::new(Mutex::new(hello)),
            script: Arc::new(Mutex::new(Script::new())),
            stop: Arc::new(AtomicBool::new(false)),
            restart: Arc::new(AtomicBool::new(false)),
            lemnos_down: Arc::new(AtomicBool::new(false)),
            hold_installs: Arc::new(AtomicBool::new(false)),
            stalled: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            devices: readings::Control::default(),
            runs: Arc::new(Mutex::new(BTreeMap::new())),
            leases: Arc::new(lease::Leases::new()),
        };

        let (ready, started) = mpsc::channel();
        let socket = board.socket().to_path_buf();
        let node_shared = shared.clone();
        let node_kind = kind.clone();
        let node_board = Arc::clone(&board);
        let node = thread::spawn(move || {
            serve(&socket, &node_shared, &node_kind, &node_board, &ready);
        });
        started
            .recv()
            .map_err(|_| "the node logic stopped before it started".to_string())??;

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let link = listener.local_addr().map_err(|error| error.to_string())?;
        let upstream = board.network();
        let proxy_shared = shared.clone();
        let proxy = thread::spawn(move || proxy(&listener, upstream, &proxy_shared));

        Ok(Self {
            board,
            link,
            shared,
            node: Some(node),
            proxy: Some(proxy),
        })
    }

    /// Connects to the board's socket, as a program on the board would.
    ///
    /// # Errors
    ///
    /// If the comm core turns the session away.
    pub fn connect(&self, name: &str) -> Result<Orion, orion::Error> {
        Orion::connect(&Address::Local(self.board.socket().to_path_buf()), name)
    }

    /// Connects through the loopback link, as a program on the network would. Faults
    /// queued in [`SimNode::script`] apply to the connection this makes.
    ///
    /// # Errors
    ///
    /// If the comm core turns the session away.
    pub fn connect_through_link(&self, name: &str) -> Result<Orion, orion::Error> {
        Orion::connect(
            &Address::Network {
                host: self.link.to_string(),
                key: None,
            },
            name,
        )
    }

    /// The board's state, for a test to read or set.
    pub fn model(&self) -> MutexGuard<'_, Model> {
        lock(&self.shared.model)
    }

    /// What the next runs and connections do, for a test to queue failures in.
    pub fn script(&self) -> MutexGuard<'_, Script> {
        lock(&self.shared.script)
    }

    /// Holds an install job where it is (`true`) or lets it move on (`false`). A held install is
    /// still running: it can be cancelled, restarted or attached to, and it finishes only once it
    /// is let go.
    pub fn hold_installs(&self, hold: bool) {
        self.shared.hold_installs.store(hold, Ordering::SeqCst);
    }

    /// How many calls this node has been asked to start, since it started. A test that must show
    /// a node saw nothing reads this: a session that refused a node sends it no call.
    #[must_use]
    pub fn calls_received(&self) -> usize {
        self.shared.calls.load(Ordering::SeqCst)
    }

    /// How many runs the script has stalled so far. A test waits on this to know a stalled run
    /// has started, rather than on a delay.
    #[must_use]
    pub fn stalled_runs(&self) -> usize {
        self.shared.stalled.load(Ordering::SeqCst)
    }

    /// Withdraws the device resources (`true`) or lists them again (`false`), the way the
    /// bridge's session is published again when a fact appears or disappears.
    pub fn hold_devices(&self, hold: bool) {
        self.shared.devices.hold(hold);
    }

    /// Restarts the service now: running jobs are lost, and nothing resumes.
    pub fn restart_daemon(&self) {
        self.shared.restart.store(true, Ordering::SeqCst);
    }

    /// Stops Lemnos on this node while the node stays up: its facts stay published, and every
    /// Lemnos call fails as "not answering" (`-4`). Only a node with Lemnos has anything to stop.
    pub fn stop_lemnos(&self) {
        self.shared.lemnos_down.store(true, Ordering::SeqCst);
    }

    /// Lemnos answers again after [`SimNode::stop_lemnos`].
    pub fn resume_lemnos(&self) {
        self.shared.lemnos_down.store(false, Ordering::SeqCst);
    }

    /// Changes the hello facts while the node runs. A changed value is published as it changes,
    /// on the same session. A fact that went away is withdrawn on the same session. A fact that
    /// appeared republishes the node's own session: its topics and actions are said again on a new
    /// session, and a client sees a short gap in them. A client's own connection is not dropped.
    pub fn change_hello(&self, change: impl FnOnce(&mut Hello)) {
        change(&mut lock(&self.shared.hello));
    }

    /// The files the comm core has received and committed, by destination. A bundle pushed for
    /// `install_file` is here until its job ends.
    #[must_use]
    pub fn shelf(&self) -> &orion_stubs::Shelf {
        self.board.shelf()
    }

    /// Advances the simulated slot clock by `secs`, then runs the trial's rules: a trial whose
    /// daemon has been up for `confirm_uptime_s` confirms, and at `confirm_deadline_s` with a
    /// condition unmet, falls back. Nothing else moves the clock.
    pub fn slot_clock_advance(&self, secs: u64) {
        lock(&self.shared.model).slots.advance_clock(secs);
    }

    /// Sets whether the board is healthy. A trial confirms only while it is.
    pub fn slot_set_healthy(&self, healthy: bool) {
        lock(&self.shared.model).slots.healthy = healthy;
    }

    /// Sets how many of the next trial boots do not reach the daemon. Each such boot spends one
    /// attempt; after `trial_attempts` of them the trial falls back. Each boot is a restart: `trial`
    /// starts the first one, and the reboot action or [`SimNode::slot_power_cut`] starts the next.
    pub fn slot_boots_without_daemon(&self, count: u32) {
        lock(&self.shared.model).slots.boots_without_daemon = count;
    }

    /// Cuts the power while a trial's boot runs. A boot that reached the daemon is cut short: no
    /// attempt is spent, and the next restart boots the trial again.
    pub fn slot_power_cut(&self) {
        lock(&self.shared.model).slots.power_cut();
    }

    /// Makes the driver's writes of the trial record fail (`true`) or succeed (`false`). A pending
    /// trial then carries `record_error`, and a fall back keeps its reason in `trial_reason`.
    pub fn slot_fail_record_writes(&self, fail: bool) {
        lock(&self.shared.model).slots.set_record_writes_fail(fail);
    }

    /// Changes the trial's config keys (I5f), for example `trial_attempts`, `confirm_uptime_s`,
    /// `confirm_deadline_s` or `auto_confirm`.
    pub fn slot_config(&self, change: impl FnOnce(&mut trial::Config)) {
        change(&mut lock(&self.shared.model).slots.trial_config);
    }

    /// How many runs of `name` the node has been asked to start, refused ones included. A run is
    /// counted when the node takes it, so a test can tell that an install was not issued twice.
    #[must_use]
    pub fn runs_started(&self, name: &str) -> usize {
        lock(&self.shared.runs).get(name).copied().unwrap_or(0)
    }

    /// How many runs of a device's `.call` the node took, over all devices: the bridge's requests
    /// that renew a lease (see `crate::lease`). Refused ones included.
    #[must_use]
    pub fn bridge_calls_started(&self) -> usize {
        lock(&self.shared.runs)
            .iter()
            .filter(|(name, _)| is_device_call(name))
            .map(|(_, count)| *count)
            .sum()
    }

    /// Moves the simulated lease clock by `secs`. A caller silent for longer than the lease period
    /// loses its lease: its running runs end with the lease's words, and [`SimNode::lease_drops`]
    /// counts it. Nothing else moves the clock.
    pub fn lease_advance(&self, secs: u64) {
        self.shared.leases.advance(Duration::from_secs(secs));
    }

    /// Sets the simulated lease period and stall time, shorter for a test. The defaults are
    /// [`lease::PERIOD`] and [`lease::STALL`].
    pub fn lease_set(&self, period: Duration, stall: Duration) {
        self.shared.leases.set(period, stall);
    }

    /// How many drops the lease has applied: callers that lost their lease and stalled runs that
    /// were dropped, counted when the node applied them.
    #[must_use]
    pub fn lease_drops(&self) -> usize {
        self.shared.leases.dropped()
    }

    /// Where the network port is, for a client that does not use the link.
    #[must_use]
    pub fn network(&self) -> SocketAddr {
        self.board.network()
    }
}

impl Drop for SimNode {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(node) = self.node.take() {
            let _ = node.join();
        }
        if let Some(proxy) = self.proxy.take() {
            let _ = proxy.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// The node logic's thread: connect, register, publish, then serve until stopped.
fn serve(
    socket: &Path,
    shared: &Shared,
    kind: &Kind,
    board: &Arc<Board>,
    ready: &mpsc::Sender<Result<(), String>>,
) {
    let mut ready = Some(ready);
    loop {
        match Node::open(socket, shared.clone(), kind, Arc::clone(board)) {
            Ok(mut node) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                node.run();
                // A fact came or went: the session is published again, on a new session.
                if !node.reopen || shared.stop.load(Ordering::SeqCst) {
                    return;
                }
            }
            Err(error) => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(error));
                }
                return;
            }
        }
    }
}

/// One hello topic the node publishes, and the text it last carried.
struct HelloTopic {
    name: &'static str,
    publisher: Publisher,
    value: String,
}

/// A timed run the node is moving along.
#[derive(Debug)]
struct Run {
    call: u64,
    name: String,
    args: Vec<u8>,
    /// Who asked for the run (the audit name Orion gives it).
    caller: String,
    left: u32,
    total: u32,
    stalled: bool,
}

/// The slot install in progress: the bundle file it works on, and how far it has got.
#[derive(Debug)]
struct Install {
    call: u64,
    name: String,
    size: u64,
    done: u64,
    pin: bool,
    keep_old: bool,
}

/// The node logic: one Orion session, with its actions, topics and runs.
struct Node {
    // Kept alive for the session: dropping it closes the connection.
    _orion: Orion,
    registration: Registration,
    logs: Publisher,
    shared: Shared,
    board: Arc<Board>,
    /// Readings, on a node with Lemnos only.
    readings: Option<Publisher>,
    /// The hello facts, one topic each, on a node with Lemnos only. Kept alive for the session.
    hello: Vec<HelloTopic>,
    /// Whether the node publishes hello facts (a node with Lemnos).
    live: bool,
    /// Set when the topics must be published again on a new session (a fact came or went).
    reopen: bool,
    /// The devices a Lemnos node describes, on a node with Lemnos only.
    devices: Option<readings::Devices>,
    active: Vec<Run>,
    install: Option<Install>,
    started: Instant,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl Node {
    fn open(socket: &Path, shared: Shared, kind: &Kind, board: Arc<Board>) -> Result<Self, String> {
        // The driver clears its incoming directory when it starts.
        clear_incoming(&board);
        let orion = Orion::connect(&Address::Local(socket.to_path_buf()), "sim-node")
            .map_err(|error| error.to_string())?;
        let mut registration = None;
        let all = names::ACTIONS.iter().chain(crate::actions::NAMES.iter());
        for name in all.filter(|name| offered(kind, name)) {
            let policy = Policy::new(Admits::ANYONE).without_grant();
            let decl = ActionDecl {
                name,
                target: target(),
                description: "simulated board action",
            };
            let handle = orion
                .register_action(&decl, policy)
                .map_err(|e| e.to_string())?;
            registration.get_or_insert(handle);
        }
        // A node with Lemnos names each device's `.call` the way the bridge does: `lemnos.<device>.call`.
        if matches!(kind, Kind::WithLemnos(_)) {
            for device in readings::DEVICES {
                let name = format!("lemnos.{}.call", device.id);
                let policy = Policy::new(Admits::ANYONE).without_grant();
                let decl = ActionDecl {
                    name: &name,
                    target: target(),
                    description: "the device's call, as the bridge names it",
                };
                let handle = orion
                    .register_action(&decl, policy)
                    .map_err(|e| e.to_string())?;
                registration.get_or_insert(handle);
            }
        }
        let logs = orion
            .declare(&TopicDecl {
                name: names::LOGS,
                fields: &fields![line: Text],
                history: 64,
                expiry_ms: 0,
                max_payload: 256,
            })
            .map_err(|e| e.to_string())?;

        // The slot table knows its board: the hello `family` fact of a node with Lemnos.
        let live = matches!(kind, Kind::WithLemnos(_));
        let hello = lock(&shared.hello).clone();
        if live {
            lock(&shared.model).slots.board.clone_from(&hello.family);
        }
        let devices = match kind {
            Kind::WithLemnos(_) => {
                Some(readings::Devices::declare(&orion, shared.devices.clone())?)
            }
            Kind::Plain => None,
        };
        // Only a node with Lemnos has readings, a hardware description and hello facts.
        let mut readings = None;
        let mut hello_topics = Vec::new();
        if live {
            readings = Some(
                orion
                    .declare(&TopicDecl {
                        name: names::READINGS,
                        fields: &fields![temperature: Float "C", voltage: Float "V", fan: Int "rpm"],
                        history: 64,
                        expiry_ms: 0,
                        max_payload: 8,
                    })
                    .map_err(|e| e.to_string())?,
            );
            let mut hardware = orion
                .declare(&TopicDecl {
                    name: names::HARDWARE,
                    fields: &fields![description: Text],
                    history: 1,
                    expiry_ms: 0,
                    max_payload: 256,
                })
                .map_err(|e| e.to_string())?;
            let description = format!(
                "model={} serial={} os={}",
                hello.model, hello.serial, hello.os_version
            );
            hardware
                .publish(
                    FieldId(0),
                    SampleTime::from_nanos(1),
                    description.as_bytes(),
                )
                .map_err(|e| e.to_string())?;
            for (name, value) in hello.facts() {
                let mut publisher = orion
                    .declare(&TopicDecl {
                        name,
                        fields: &fields![value: Text],
                        history: 2,
                        expiry_ms: 0,
                        max_payload: 256,
                    })
                    .map_err(|e| e.to_string())?;
                publisher
                    .publish(FieldId(0), SampleTime::from_nanos(1), value.as_bytes())
                    .map_err(|e| e.to_string())?;
                hello_topics.push(HelloTopic {
                    name,
                    publisher,
                    value,
                });
            }
        }
        let registration = registration.ok_or("no actions to register")?;
        Ok(Self {
            _orion: orion,
            registration,
            readings,
            logs,
            shared,
            board,
            hello: hello_topics,
            live,
            reopen: false,
            devices,
            active: Vec::new(),
            install: None,
            started: Instant::now(),
        })
    }

    fn run(&mut self) {
        let mut last_tick = Instant::now();
        while !self.shared.stop.load(Ordering::SeqCst) && !self.reopen {
            match self.registration.wait(Duration::from_millis(5)) {
                Ok(Some(Request::Run {
                    call,
                    name,
                    args,
                    caller,
                    ..
                })) => {
                    self.shared.calls.fetch_add(1, Ordering::SeqCst);
                    self.shared.leases.request(&caller);
                    self.start(call, &name, args, &caller);
                }
                Ok(Some(Request::Cancel { call })) => {
                    if let Some(caller) = self
                        .active
                        .iter()
                        .find(|run| run.call == call)
                        .map(|run| run.caller.clone())
                    {
                        self.shared.leases.request(&caller);
                    }
                    self.cancel(call);
                }
                Ok(None) => {}
                // The comm core went away: nothing more can be answered.
                Err(_) => break,
            }
            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                self.tick();
            }
        }
    }

    fn now(&self) -> SampleTime {
        let nanos = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        SampleTime::from_nanos(nanos.saturating_add(1))
    }

    fn finish(&mut self, call: u64, outcome: Outcome, output: &[u8], detail: &str) {
        let _ = self.registration.finish(call, outcome, output, detail);
    }

    fn finish_status(&mut self, call: u64, status: &Status) {
        self.finish(call, outcome_for(status.code), &[], &status.to_string());
    }

    /// Ends the runs the lease has dropped: a caller's running runs, or one stalled run.
    fn apply_lease_drops(&mut self) {
        for drop in self.shared.leases.take() {
            match drop {
                lease::Drop::Caller(caller) => {
                    let ended: Vec<u64> = self
                        .active
                        .iter()
                        .filter(|run| run.caller == caller)
                        .map(|run| run.call)
                        .collect();
                    self.active.retain(|run| run.caller != caller);
                    for call in ended {
                        self.finish(call, Outcome::Failed, &[], lease::CALLER_WORDS);
                    }
                }
                lease::Drop::Run(call) => {
                    if let Some(at) = self.active.iter().position(|run| run.call == call) {
                        self.active.remove(at);
                        self.finish(call, Outcome::Failed, &[], lease::STALL_WORDS);
                    }
                }
            }
        }
    }

    fn start(&mut self, call: u64, name: &str, args: Vec<u8>, caller: &str) {
        *lock(&self.shared.runs).entry(name.to_string()).or_default() += 1;
        if names::is_lemnos(name) && self.shared.lemnos_down.load(Ordering::SeqCst) {
            // Lemnos is stopped on this node: the node is up, its Lemnos calls are not answered.
            self.finish_status(
                call,
                &Status::new(code::INTERRUPTED, "lemnos is not answering on this node"),
            );
            return;
        }
        if is_device_call(name) && args.is_empty() {
            // The bridge's `.call` with no method: refused before anything is read or written.
            self.finish(call, Outcome::Rejected, &[], "call needs a method");
            return;
        }
        if is_device_call(name) && args == b"describe" {
            // The renewal at I6h (`.call describe`): a read of the description, which changes nothing.
            // LOCK: Lemnos I6h describe.
            self.finish(call, Outcome::Succeeded, &[], "");
            return;
        }
        let behaviour = lock(&self.shared.script).take_action(name);
        match behaviour {
            Behaviour::Refuse(reason) => self.finish(call, Outcome::Rejected, &[], &reason),
            Behaviour::Fail(reason) => self.finish(call, Outcome::Failed, &[], &reason),
            Behaviour::Stall => {
                self.shared.leases.stalled(call);
                self.shared.stalled.fetch_add(1, Ordering::SeqCst);
                self.active.push(Run {
                    call,
                    name: name.to_string(),
                    args,
                    caller: caller.to_string(),
                    left: 1,
                    total: 1,
                    stalled: true,
                });
            }
            Behaviour::RestartDaemon => {
                self.restart();
                self.finish(call, Outcome::Failed, &[], "-4: the service restarted");
            }
            Behaviour::Answer if name == names::SLOTS_ACCEPT => self.accept(call, &args),
            Behaviour::Answer if name == names::SLOTS_INSTALL_FILE => {
                self.install_start(call, &args);
            }
            Behaviour::Answer => {
                let total = crate::actions::ticks(name, &args).unwrap_or_else(|| ticks_for(name));
                self.active.push(Run {
                    call,
                    name: name.to_string(),
                    args,
                    caller: caller.to_string(),
                    left: total,
                    total,
                    stalled: false,
                });
            }
        }
    }

    /// `system.slots.accept <size>`: checks there is room for a bundle of `size` bytes. Nothing
    /// is written: the sender pushes the file next.
    fn accept(&mut self, call: u64, args: &[u8]) {
        if self.install.is_some() {
            self.finish_status(call, &Status::new(code::BUSY, "an install is running"));
            return;
        }
        let size = match words(args)
            .first()
            .and_then(|word| word.parse::<u64>().ok())
        {
            Some(size) => size,
            None => {
                self.finish_status(
                    call,
                    &Status::new(code::INVALID_ARGUMENT, "accept takes the bundle's size"),
                );
                return;
            }
        };
        let checked = lock(&self.shared.model).slots.check_install_start(size);
        match checked {
            Ok(()) => self.finish(call, Outcome::Succeeded, &[], ""),
            Err(status) => {
                lock(&self.shared.model)
                    .slots
                    .error
                    .clone_from(&status.message);
                self.finish_status(call, &status);
            }
        }
    }

    /// `system.slots.install-file <name> <pin> <keep_old>`: starts the long job that installs
    /// the bundle pushed as `INCOMING/<name>`.
    fn install_start(&mut self, call: u64, args: &[u8]) {
        if self.install.is_some() {
            self.finish_status(call, &Status::new(code::BUSY, "an install is running"));
            return;
        }
        let parsed = (|| -> Result<(String, bool, bool), Status> {
            let found = words(args);
            let name = found.first().cloned().unwrap_or_default();
            if !names::valid_name(&name) {
                return Err(Status::new(
                    code::INVALID_ARGUMENT,
                    format!("{name:?} is not one file name"),
                ));
            }
            Ok((name, flag(&found, 1)?, flag(&found, 2)?))
        })();
        let (name, pin, keep_old) = match parsed {
            Ok(parsed) => parsed,
            Err(status) => {
                self.finish_status(call, &status);
                return;
            }
        };
        let size = {
            let committed = lock_shelf(&self.board);
            committed
                .get(&pushed(&name))
                .map(|bytes| bytes.len() as u64)
        };
        let Some(size) = size else {
            self.finish_status(
                call,
                &Status::new(code::NOT_FOUND, format!("there is no pushed file {name}")),
            );
            return;
        };
        let checked = {
            let mut model = lock(&self.shared.model);
            model
                .slots
                .check_install_start(size)
                .map(|()| model.slots.staging())
        };
        match checked {
            Ok(()) => {
                self.install = Some(Install {
                    call,
                    name,
                    size,
                    done: 0,
                    pin,
                    keep_old,
                });
            }
            Err(status) => {
                lock(&self.shared.model)
                    .slots
                    .error
                    .clone_from(&status.message);
                self.finish_status(call, &status);
            }
        }
    }

    /// The install job's last step: verify and unpack the file, install it or say why not. The
    /// file goes whatever the outcome.
    fn complete_install(&mut self) {
        let Some(install) = self.install.take() else {
            return;
        };
        let bytes = lock_shelf(&self.board).remove(&pushed(&install.name));
        let result = {
            let mut model = lock(&self.shared.model);
            let outcome = match bytes {
                None => Err(Status::new(
                    code::IO,
                    "the pushed file went away mid-install",
                )),
                Some(bytes) => {
                    let trusted = model.slots.trusted.clone();
                    let board = model.slots.board.clone();
                    bundle::verify(&bytes, &trusted, &board).and_then(|verified| {
                        model.slots.check_signed(&verified.signed)?;
                        model
                            .slots
                            .install_done(new_slot(verified), install.pin, install.keep_old)
                    })
                }
            };
            if let Err(status) = &outcome {
                model.slots.state = State::Error;
                model.slots.error.clone_from(&status.message);
            }
            outcome
        };
        match result {
            Ok(id) => self.finish(install.call, Outcome::Succeeded, id.as_bytes(), ""),
            Err(status) => self.finish_status(install.call, &status),
        }
    }

    /// Cancelling: an install job removes its file and changes nothing else; a timed run changed
    /// nothing. Orion only sends a cancel from the run's own starter.
    fn cancel(&mut self, call: u64) {
        if self
            .install
            .as_ref()
            .is_some_and(|install| install.call == call)
        {
            if let Some(install) = self.install.take() {
                lock_shelf(&self.board).remove(&pushed(&install.name));
            }
            lock(&self.shared.model).slots.cancelled();
            self.finish(
                call,
                Outcome::Cancelled,
                &[],
                "cancelled before the slot was written",
            );
        } else if let Some(index) = self.active.iter().position(|run| run.call == call) {
            self.active.remove(index);
            self.finish(
                call,
                Outcome::Cancelled,
                &[],
                "cancelled before any change was made",
            );
        }
    }

    /// A service restart: every running job is lost. An install cut short is recorded, and the
    /// incoming directory is cleared as the driver does when it starts.
    fn restart(&mut self) {
        let mut lost = Vec::new();
        if let Some(install) = self.install.take() {
            lock(&self.shared.model).slots.interrupted();
            lost.push(install.call);
        }
        lost.extend(self.active.drain(..).map(|run| run.call));
        clear_incoming(&self.board);
        lock(&self.shared.model).slots.daemon_restarted();
        for call in lost {
            self.finish(
                call,
                Outcome::Failed,
                &[],
                "-4: the service restarted before this finished",
            );
        }
        lock(&self.shared.model).log("the service restarted");
    }

    /// Makes the hello topics say what the hello facts say now. A value that changed is published
    /// as it changes. A fact that went away is withdrawn on this session: its topic is taken back,
    /// so the document no longer carries it. A fact that appeared cannot be declared on a session
    /// that is already published, so the node asks to be published again on a new session, where
    /// every fact is said again.
    fn sync_hello(&mut self) {
        let wanted = lock(&self.shared.hello).facts();
        if wanted
            .iter()
            .any(|(name, _)| !self.hello.iter().any(|topic| topic.name == *name))
        {
            self.reopen = true;
            return;
        }
        let mut kept = Vec::with_capacity(self.hello.len());
        for topic in self.hello.drain(..) {
            if wanted.iter().any(|(name, _)| *name == topic.name) {
                kept.push(topic);
            } else {
                // The fact is gone: the topic is withdrawn, and the document drops it.
                let _ = topic.publisher.withdraw();
            }
        }
        self.hello = kept;
        let now = self.now();
        for (name, value) in wanted {
            if let Some(topic) = self.hello.iter_mut().find(|topic| topic.name == name)
                && topic.value != value
            {
                let _ = topic.publisher.publish(FieldId(0), now, value.as_bytes());
                topic.value = value;
            }
        }
    }

    fn tick(&mut self) {
        if self.shared.restart.swap(false, Ordering::SeqCst) {
            self.restart();
        }
        self.apply_lease_drops();
        if self.live {
            self.sync_hello();
        }
        let (readings, lines) = {
            let mut model = lock(&self.shared.model);
            model.ticks += 1;
            model.slots.evaluate();
            (model.readings(), model.take_new_logs())
        };
        let now = self.now();
        if let Some(devices) = self.devices.as_mut() {
            devices.tick(&self._orion, now);
        }
        if let Some(readings_topic) = self.readings.as_mut() {
            let _ = readings_topic.publish(FieldId(0), now, &readings.temperature_c.to_le_bytes());
            let _ = readings_topic.publish(FieldId(1), now, &readings.input_v.to_le_bytes());
            let _ = readings_topic.publish(FieldId(2), now, &readings.fan_rpm.to_le_bytes());
        }
        for line in lines {
            let mut text = line;
            text.truncate(256);
            let _ = self.logs.publish(FieldId(0), now, text.as_bytes());
        }

        // The install job moves over the bundle bytes, and finishes when they are all done.
        let mut finished_install = false;
        let held = self.shared.hold_installs.load(Ordering::SeqCst);
        if let Some(install) = self.install.as_mut().filter(|_| !held) {
            install.done = (install.done + INSTALL_STEP).min(install.size);
            let percent = (install.done * 100)
                .checked_div(install.size)
                .unwrap_or(100) as u8;
            let message = format!("{}/{} bytes", install.done, install.size);
            let _ = self
                .registration
                .progress(install.call, percent, &message, &[]);
            finished_install = install.done == install.size;
        }
        if finished_install {
            self.complete_install();
        }

        let mut done = Vec::new();
        for (index, run) in self.active.iter_mut().enumerate() {
            if run.stalled {
                continue;
            }
            run.left = run.left.saturating_sub(1);
            if run.left == 0 {
                done.push(index);
            } else {
                let percent = (u64::from(run.total - run.left) * 100 / u64::from(run.total)) as u8;
                let message = format!("{} {}/{}", run.name, run.total - run.left, run.total);
                let _ = self.registration.progress(run.call, percent, &message, &[]);
            }
        }
        for index in done.into_iter().rev() {
            let run = self.active.remove(index);
            let outcome = {
                let mut model = lock(&self.shared.model);
                apply(&mut model, &run.name, &run.args, &run.caller)
            };
            match outcome {
                Ok(output) => self.finish(run.call, Outcome::Succeeded, &output, ""),
                Err(status) => self.finish_status(run.call, &status),
            }
        }
    }
}

/// The path a pushed bundle is kept at on the shelf.
/// A device's `.call` action: `lemnos.<device>.call`, the bridge's name for it.
fn is_device_call(name: &str) -> bool {
    name.starts_with("lemnos.") && name.ends_with(".call")
}

fn pushed(name: &str) -> String {
    format!("{}{name}", names::INCOMING)
}

fn lock_shelf(board: &Board) -> MutexGuard<'_, std::collections::BTreeMap<String, Vec<u8>>> {
    lock(&board.shelf().committed)
}

/// Removes every pushed file the driver keeps.
fn clear_incoming(board: &Board) {
    lock_shelf(board).retain(|path, _| !path.starts_with(names::INCOMING));
}

/// How many ticks a timed action takes. Only the long ones show progress.
fn ticks_for(name: &str) -> u32 {
    match name {
        names::SELF_TEST => 4,
        _ => 1,
    }
}

/// The outcome a Lemnos status gives a run.
fn outcome_for(code_value: i32) -> Outcome {
    match code_value {
        0 => Outcome::Succeeded,
        code::CANCELLED => Outcome::Cancelled,
        code::IO | code::INTERRUPTED => Outcome::Failed,
        _ => Outcome::Rejected,
    }
}

/// The words of an argument: UTF-8, separated by whitespace.
fn words(args: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(args)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// The bool at `index`, as Lemnos's text encoding reads one: true/on/1 or false/off/0.
fn flag(words: &[String], index: usize) -> Result<bool, Status> {
    match words.get(index).map(String::as_str) {
        Some("true" | "on" | "1") => Ok(true),
        Some("false" | "off" | "0") => Ok(false),
        Some(other) => Err(Status::new(
            code::INVALID_ARGUMENT,
            format!("{other:?} is not a bool"),
        )),
        None => Err(Status::new(code::INVALID_ARGUMENT, "a value is missing")),
    }
}

/// The word at `index`, or a refusal naming what it should be.
fn word(words: &[String], index: usize, what: &str) -> Result<String, Status> {
    words
        .get(index)
        .cloned()
        .ok_or_else(|| Status::new(code::INVALID_ARGUMENT, format!("missing {what}")))
}

fn slot_id(words: &[String]) -> Result<String, Status> {
    let id = word(words, 0, "a slot id")?;
    if valid_id(&id) {
        Ok(id)
    } else {
        Err(Status::new(
            code::INVALID_ARGUMENT,
            format!("{id:?} is not a slot id"),
        ))
    }
}

fn hex_bytes(text: &str) -> Result<Vec<u8>, Status> {
    let bad = || Status::new(code::INVALID_ARGUMENT, format!("{text:?} is not hex bytes"));
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(bad());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).map_err(|_| bad()))
        .collect()
}

fn hex_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn new_slot(verified: bundle::Verified) -> NewSlot {
    NewSlot {
        id: verified.id,
        product: verified.product,
        board: verified.board,
        version: verified.version,
        variant: verified.variant,
        kernel: verified.kernel,
        display_name: verified.display_name,
        size: verified.size,
        verity: verified.verity,
        signed: verified.signed,
    }
}

/// The largest reply a method with an out buffer can give (Lemnos M8a: 60 KiB).
const LIST_LIMIT: usize = 60 * 1024;

/// Applies a finished timed run to the board. Its output goes back to the caller.
fn apply(model: &mut Model, name: &str, args: &[u8], caller: &str) -> Result<Vec<u8>, Status> {
    let nothing = Vec::new();
    let found = words(args);
    match name {
        // LOCK: Lemnos L1 (the list refusal as part of the L1 rev). Pre-L1 board: no refusal by name.
        // LOCK: Lemnos M8a 5dcf1a78. A reply over 60 KiB is refused by name, with no partial
        // output. Today: the Atlas lock (4a2d3b7d) predates M8a, so a real board cannot answer
        // `list` until the lock moves.
        names::SLOTS_LIST => {
            let listing = model.slots.list_json();
            if listing.len() > LIST_LIMIT {
                Err(Status::new(
                    code::INVALID_ARGUMENT,
                    "system.slots.list: the reply is over 60 KiB; refused by name, nothing partial is sent",
                ))
            } else {
                Ok(listing.into_bytes())
            }
        }
        names::SLOTS_SET_DEFAULT => model
            .slots
            .set_default(&slot_id(&found)?, flag(&found, 1).unwrap_or(false))
            .map(|()| nothing),
        names::SLOTS_TRIAL => model
            .slots
            .trial(
                &slot_id(&found)?,
                flag(&found, 1)?,
                flag(&found, 2)?,
                flag(&found, 3)?,
            )
            .map(|()| nothing),
        names::SLOTS_CONFIRM => model.slots.confirm(caller).map(|()| nothing),
        names::SLOTS_REMOVE => model.slots.remove(&slot_id(&found)?).map(|()| nothing),
        names::SLOTS_PIN => model
            .slots
            .pin(&slot_id(&found)?, flag(&found, 1)?)
            .map(|()| nothing),
        names::SLOTS_SET_KEEP => {
            let keep: u32 = word(&found, 0, "keep")?
                .parse()
                .map_err(|_| Status::new(code::INVALID_ARGUMENT, "keep is not a number"))?;
            model.slots.set_keep(keep);
            Ok(nothing)
        }
        names::REBOOT => {
            model.slots.reboot();
            Ok(model.status_text().into_bytes())
        }
        names::STATUS => Ok(model.status_text().into_bytes()),
        names::LOCATE => Ok(b"blinking".to_vec()),
        names::SELF_TEST => Ok(b"passed".to_vec()),
        names::SET_CLOCK => {
            if model.clock_settable {
                model.clock_offset_s = None;
                Ok(b"clock set".to_vec())
            } else {
                Err(Status::new(code::IO, "the clock did not take"))
            }
        }
        names::SUPPORT_BUNDLE => {
            let mut text = model.status_text();
            text.push('\n');
            text.push_str(&model.slots.list_json());
            text.push('\n');
            for line in model.logs().iter().rev().take(20).rev() {
                text.push_str(line);
                text.push('\n');
            }
            Ok(text.into_bytes())
        }
        names::DATA_BACKUP => Ok(hex_text(&model.data_backup()).into_bytes()),
        names::DATA_RESTORE => {
            let blob = hex_bytes(&word(&found, 0, "a backup")?)?;
            model.data_restore(&blob).map(|()| nothing)
        }
        names::FS_LIST => {
            let prefix = found.first().cloned().unwrap_or_default();
            let mut text = String::new();
            for path in model.data.keys().filter(|path| path.starts_with(&prefix)) {
                text.push_str(path);
                text.push('\n');
            }
            Ok(text.into_bytes())
        }
        names::FS_READ => {
            let path = word(&found, 0, "a path")?;
            model
                .data
                .get(&path)
                .map(|bytes| hex_text(bytes).into_bytes())
                .ok_or_else(|| Status::new(code::NOT_FOUND, format!("there is no file {path}")))
        }
        names::FS_WRITE => {
            let path = word(&found, 0, "a path")?;
            let body = hex_bytes(found.get(1).map_or("", String::as_str))?;
            model.data.insert(path, body);
            Ok(nothing)
        }
        names::FS_REMOVE => {
            let path = word(&found, 0, "a path")?;
            match model.data.remove(&path) {
                Some(_) => Ok(nothing),
                None => Err(Status::new(
                    code::NOT_FOUND,
                    format!("there is no file {path}"),
                )),
            }
        }
        other if crate::actions::handles(other) => crate::actions::apply(other, args),
        other => Err(Status::new(
            code::NOT_FOUND,
            format!("no simulated action {other}"),
        )),
    }
}

/// The loopback link: accepts connections and carries each one to the comm core, with the
/// fault queued for it, if any.
fn proxy(listener: &TcpListener, upstream: SocketAddr, shared: &Shared) {
    let _ = listener.set_nonblocking(true);
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, _)) => {
                let fault = lock(&shared.script).take_link();
                let _ = client.set_nonblocking(false);
                if let Ok(server) = TcpStream::connect(upstream) {
                    splice(client, server, fault);
                }
            }
            Err(_) => thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Carries bytes both ways until one side closes, or until the fault cuts the link.
fn splice(client: TcpStream, server: TcpStream, fault: Option<LinkFault>) {
    let pause = match fault {
        Some(LinkFault::Stall { pause }) => pause,
        _ => Duration::ZERO,
    };
    let limit = match fault {
        Some(LinkFault::Drop { after_bytes }) => Some(after_bytes),
        _ => None,
    };
    let (Ok(mut client_in), Ok(mut server_out)) = (client.try_clone(), server.try_clone()) else {
        return;
    };
    let (Ok(client_cut), Ok(server_cut)) = (client.try_clone(), server.try_clone()) else {
        return;
    };
    let (Ok(mut server_in), Ok(mut client_out)) = (server.try_clone(), client.try_clone()) else {
        return;
    };
    thread::spawn(move || {
        thread::sleep(pause);
        let cut = limit.map(|_| (client_cut, server_cut));
        forward(&mut client_in, &mut server_out, limit, cut.as_ref());
    });
    thread::spawn(move || {
        thread::sleep(pause);
        forward(&mut server_in, &mut client_out, None, None);
    });
}

/// Copies from `from` to `to`. With a limit, stops after that many bytes and cuts both
/// sides, as a link that drops mid-transfer would.
fn forward(
    from: &mut TcpStream,
    to: &mut TcpStream,
    limit: Option<u64>,
    cut: Option<&(TcpStream, TcpStream)>,
) {
    let mut buffer = [0_u8; 16 * 1024];
    let mut sent: u64 = 0;
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let chunk = &buffer[..read];
        if let (Some(limit), Some((client, server))) = (limit, cut) {
            let room = limit.saturating_sub(sent);
            if (chunk.len() as u64) > room {
                let room = usize::try_from(room).unwrap_or(0);
                let _ = to.write_all(&chunk[..room]);
                let _ = client.shutdown(Shutdown::Both);
                let _ = server.shutdown(Shutdown::Both);
                return;
            }
        }
        if to.write_all(chunk).is_err() {
            break;
        }
        sent += read as u64;
    }
    let _ = to.shutdown(Shutdown::Write);
}
