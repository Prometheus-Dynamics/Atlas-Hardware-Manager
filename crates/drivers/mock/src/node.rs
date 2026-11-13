//! The simulated node: an Orion comm core running in this process, with node logic that
//! answers the board's actions, topics and files the way a real board does.
//!
//! Three parts, each one Orion's own code:
//! - the comm core is `orion-stubs`' [`Board`]: the real server, broker, hello and keys on a
//!   real socket and a real network port, with stand-ins for the record store, the action
//!   router, the file receiver and the workload tracker;
//! - the node logic is an `orion` client on that socket. It registers every action in
//!   [`names::ACTIONS`], declares the topics in [`names::TOPICS`], and answers each run;
//! - a loopback link proxy sits in front of the network port, so a connection can be cut
//!   mid-transfer or held still. That is the only piece here that is not Orion's.
//!
//! A slot install is a long job: `system.slots.install` starts it, `system.slots.write`
//! streams its bytes in order, and the last write installs it. Cancelling the install, or
//! its starter disconnecting (Orion's router sends the cancel), discards what was written.
//! A service restart loses every running job, and nothing resumes.
//!
//! Other runs are applied to the [`Model`] when they finish, never before, so a run that is
//! refused, fails or is cancelled before it finishes changes nothing.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orion::actions::{
    ActionCall, ActionDecl, Admits, Finished, Outcome, Policy, Registration, Request, Target,
};
use orion::topics::{FieldId, Publisher, SampleTime, TopicDecl};
use orion::{Address, Orion, fields};
use orion_stubs::{Board, Security, Shelf};

use crate::bundle;
use crate::model::Model;
use crate::names;
use crate::script::{Behaviour, LinkFault, Script};
use crate::slots::{NewSlot, State, Status, code, valid_id};

/// The name the simulated board gives itself.
pub const NODE_NAME: &str = "board";
/// How often the node logic moves runs on and publishes readings.
const TICK: Duration = Duration::from_millis(20);
/// How long a call waits for its outcome before the caller gives up.
const CALL_WAIT: Duration = Duration::from_secs(10);
/// The largest chunk one write may carry.
pub const MAX_CHUNK: usize = 32 * 1024;

/// The target every simulated action is registered and run against.
#[must_use]
pub fn target() -> Target {
    Target::Node(
        NODE_NAME
            .parse()
            .expect("the simulated node's name is a valid name"),
    )
}

/// Runs `name` with `args` on the node `orion` is connected to, and waits for how it ended.
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
    Ok(run.finish(CALL_WAIT)?)
}

/// The state shared between the test and the node logic.
#[derive(Clone, Debug)]
struct Shared {
    model: Arc<Mutex<Model>>,
    script: Arc<Mutex<Script>>,
    stop: Arc<AtomicBool>,
    restart: Arc<AtomicBool>,
}

/// A simulated board. Dropping it stops the node logic, the link and the comm core.
#[derive(Debug)]
pub struct SimNode {
    board: Board,
    link: SocketAddr,
    shared: Shared,
    node: Option<JoinHandle<()>>,
    proxy: Option<JoinHandle<()>>,
}

impl SimNode {
    /// Starts a simulated board in `dir`, which is created if it is missing. Its socket
    /// lives there, so the path must stay short enough for a Unix socket.
    ///
    /// # Errors
    ///
    /// If the directory cannot be made, or the node logic cannot connect to the comm core.
    pub fn start(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let board = Board::start_in(dir, Security::Off, None);
        let shared = Shared {
            model: Arc::new(Mutex::new(Model::new())),
            script: Arc::new(Mutex::new(Script::new())),
            stop: Arc::new(AtomicBool::new(false)),
            restart: Arc::new(AtomicBool::new(false)),
        };

        let (ready, started) = mpsc::channel();
        let socket = board.socket().to_path_buf();
        let node_shared = shared.clone();
        let node = thread::spawn(move || serve(&socket, &node_shared, &ready));
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

    /// Restarts the service now: running jobs are lost, and nothing resumes.
    pub fn restart_daemon(&self) {
        self.shared.restart.store(true, Ordering::SeqCst);
    }

    /// The files the comm core has committed, as a file receiver holds them.
    #[must_use]
    pub fn shelf(&self) -> &Shelf {
        self.board.shelf()
    }

    /// Where the network port is, for a client that does not use the link.
    #[must_use]
    pub const fn network(&self) -> SocketAddr {
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
fn serve(socket: &Path, shared: &Shared, ready: &mpsc::Sender<Result<(), String>>) {
    match Node::open(socket, shared.clone()) {
        Ok(mut node) => {
            let _ = ready.send(Ok(()));
            node.run();
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

/// A timed run the node is moving along.
#[derive(Debug)]
struct Run {
    call: u64,
    name: String,
    args: Vec<u8>,
    left: u32,
    total: u32,
    stalled: bool,
}

/// The slot install in progress: its bytes so far.
#[derive(Debug)]
struct Install {
    call: u64,
    size: u64,
    pin: bool,
    keep_old: bool,
    buf: Vec<u8>,
}

/// The node logic: one Orion session, with its actions, topics and runs.
struct Node {
    // Kept alive for the session: dropping it closes the connection.
    _orion: Orion,
    registration: Registration,
    readings: Publisher,
    logs: Publisher,
    shared: Shared,
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
    fn open(socket: &Path, shared: Shared) -> Result<Self, String> {
        let orion = Orion::connect(&Address::Local(socket.to_path_buf()), "sim-node")
            .map_err(|error| error.to_string())?;
        let mut registration = None;
        for name in names::ACTIONS {
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
        let readings = orion
            .declare(&TopicDecl {
                name: names::READINGS,
                fields: &fields![temperature: Float "C", voltage: Float "V", fan: Int "rpm"],
                history: 64,
                expiry_ms: 0,
                max_payload: 8,
            })
            .map_err(|e| e.to_string())?;
        let mut hardware = orion
            .declare(&TopicDecl {
                name: names::HARDWARE,
                fields: &fields![description: Text],
                history: 1,
                expiry_ms: 0,
                max_payload: 256,
            })
            .map_err(|e| e.to_string())?;
        let logs = orion
            .declare(&TopicDecl {
                name: names::LOGS,
                fields: &fields![line: Text],
                history: 64,
                expiry_ms: 0,
                max_payload: 256,
            })
            .map_err(|e| e.to_string())?;
        let description = "model=simulated-board serial=SIM-RUN-1 os=1.0.0";
        hardware
            .publish(
                FieldId(0),
                SampleTime::from_nanos(1),
                description.as_bytes(),
            )
            .map_err(|e| e.to_string())?;
        let registration = registration.ok_or("no actions to register")?;
        Ok(Self {
            _orion: orion,
            registration,
            readings,
            logs,
            shared,
            active: Vec::new(),
            install: None,
            started: Instant::now(),
        })
    }

    fn run(&mut self) {
        let mut last_tick = Instant::now();
        while !self.shared.stop.load(Ordering::SeqCst) {
            match self.registration.wait(Duration::from_millis(5)) {
                Ok(Some(Request::Run {
                    call, name, args, ..
                })) => self.start(call, &name, args),
                Ok(Some(Request::Cancel { call })) => self.cancel(call),
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

    fn start(&mut self, call: u64, name: &str, args: Vec<u8>) {
        let behaviour = lock(&self.shared.script).take_action(name);
        match behaviour {
            Behaviour::Refuse(reason) => self.finish(call, Outcome::Rejected, &[], &reason),
            Behaviour::Fail(reason) => self.finish(call, Outcome::Failed, &[], &reason),
            Behaviour::Stall => self.active.push(Run {
                call,
                name: name.to_string(),
                args,
                left: 1,
                total: 1,
                stalled: true,
            }),
            Behaviour::RestartDaemon => {
                self.restart();
                self.finish(call, Outcome::Failed, &[], "-4: the service restarted");
            }
            Behaviour::Answer if name == names::SLOTS_INSTALL => self.install_start(call, &args),
            Behaviour::Answer if name == names::SLOTS_WRITE => self.write(call, &args),
            Behaviour::Answer => {
                let total = ticks_for(name);
                self.active.push(Run {
                    call,
                    name: name.to_string(),
                    args,
                    left: total,
                    total,
                    stalled: false,
                });
            }
        }
    }

    /// `system.slots.install`: starts the long job. Args: size (u64 LE), pin, keep_old.
    fn install_start(&mut self, call: u64, args: &[u8]) {
        if self.install.is_some() {
            self.finish(
                call,
                Outcome::Rejected,
                &[],
                "-16: another install is running",
            );
            return;
        }
        let (Some(size), Some(&pin), Some(&keep_old)) = (le_u64(args, 0), args.get(8), args.get(9))
        else {
            self.finish_status(
                call,
                &Status::new(code::INVALID_ARGUMENT, "install takes a size and two flags"),
            );
            return;
        };
        let started = {
            let mut model = lock(&self.shared.model);
            model
                .slots
                .check_install_start(size)
                .map(|()| model.slots.staging())
        };
        match started {
            Ok(()) => {
                self.install = Some(Install {
                    call,
                    size,
                    pin: pin == 1,
                    keep_old: keep_old == 1,
                    buf: Vec::new(),
                });
            }
            Err(status) => self.finish_status(call, &status),
        }
    }

    /// `system.slots.write`: appends a chunk at the offset the install expects.
    fn write(&mut self, call: u64, args: &[u8]) {
        let Some(offset) = le_u64(args, 0) else {
            self.finish_status(
                call,
                &Status::new(code::INVALID_ARGUMENT, "write takes an offset"),
            );
            return;
        };
        let chunk = args.get(8..).unwrap_or_default();
        let Some(install) = self.install.as_mut() else {
            self.finish_status(call, &Status::new(code::NOT_FOUND, "no install is running"));
            return;
        };
        let written = install.buf.len() as u64;
        let problem = if offset != written {
            Some(format!(
                "offset {offset} is not where the last write ended, at {written}"
            ))
        } else if written + chunk.len() as u64 > install.size {
            Some("the write goes past the size the install was started with".to_string())
        } else if chunk.len() > MAX_CHUNK {
            Some("the chunk is larger than the largest allowed".to_string())
        } else {
            None
        };
        if let Some(problem) = problem {
            // The install goes on: the write is refused, nothing changes.
            self.finish_status(call, &Status::new(code::INVALID_ARGUMENT, problem));
            return;
        }
        install.buf.extend_from_slice(chunk);
        let complete = install.buf.len() as u64 == install.size;
        self.finish(call, Outcome::Succeeded, &[], "");
        if complete {
            self.complete_install();
        }
    }

    /// The last write is in: check the bundle and install it, or say why not.
    fn complete_install(&mut self) {
        let Some(install) = self.install.take() else {
            return;
        };
        let result = {
            let mut model = lock(&self.shared.model);
            let trusted = model.slots.trusted.clone();
            let outcome = bundle::verify(&install.buf, &trusted).and_then(|verified| {
                model.slots.check_signed(&verified.signed)?;
                model
                    .slots
                    .install_done(new_slot(verified), install.pin, install.keep_old)
            });
            if outcome.is_err() {
                model.slots.state = State::Error;
                model.slots.error = outcome
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default();
            }
            outcome
        };
        match result {
            Ok(id) => self.finish(install.call, Outcome::Succeeded, id.as_bytes(), ""),
            Err(status) => self.finish_status(install.call, &status),
        }
    }

    /// Cancelling: an install discards what it wrote; a timed run changed nothing.
    fn cancel(&mut self, call: u64) {
        if self
            .install
            .as_ref()
            .is_some_and(|install| install.call == call)
        {
            self.install = None;
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

    /// A service restart: every running job is lost. An install cut short is recorded.
    fn restart(&mut self) {
        let mut lost = Vec::new();
        if let Some(install) = self.install.take() {
            lock(&self.shared.model).slots.interrupted();
            lost.push(install.call);
        }
        lost.extend(self.active.drain(..).map(|run| run.call));
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

    fn tick(&mut self) {
        if self.shared.restart.swap(false, Ordering::SeqCst) {
            self.restart();
        }
        let (readings, lines) = {
            let mut model = lock(&self.shared.model);
            model.ticks += 1;
            (model.readings(), model.take_new_logs())
        };
        let now = self.now();
        let _ = self
            .readings
            .publish(FieldId(0), now, &readings.temperature_c.to_le_bytes());
        let _ = self
            .readings
            .publish(FieldId(1), now, &readings.input_v.to_le_bytes());
        let _ = self
            .readings
            .publish(FieldId(2), now, &readings.fan_rpm.to_le_bytes());
        for line in lines {
            let mut text = line;
            text.truncate(256);
            let _ = self.logs.publish(FieldId(0), now, text.as_bytes());
        }

        if let Some(install) = &self.install {
            let (call, size, written) = (install.call, install.size, install.buf.len() as u64);
            let percent = (written * 100).checked_div(size).unwrap_or(100).min(100) as u8;
            let message = format!("{written}/{size} bytes");
            let _ = self.registration.progress(call, percent, &message, &[]);
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
                apply(&mut model, &run.name, &run.args)
            };
            match outcome {
                Ok(output) => self.finish(run.call, Outcome::Succeeded, &output, ""),
                Err(status) => self.finish_status(run.call, &status),
            }
        }
    }
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

fn le_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}

fn new_slot(verified: bundle::Verified) -> NewSlot {
    NewSlot {
        id: verified.id,
        family: verified.family,
        version: verified.version,
        variant: verified.variant,
        kernel: verified.kernel,
        display_name: verified.display_name,
        size: verified.size,
        verity: verified.verity,
        signed: verified.signed,
    }
}

/// The value of `key=` in the args: the first line that starts with it.
fn arg(args: &[u8], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    args.split(|byte| *byte == b'\n').find_map(|line| {
        line.strip_prefix(prefix.as_bytes())
            .map(|value| String::from_utf8_lossy(value).into_owned())
    })
}

fn required(args: &[u8], key: &str) -> Result<String, Status> {
    arg(args, key).ok_or_else(|| Status::new(code::INVALID_ARGUMENT, format!("missing {key}=")))
}

fn slot_id(args: &[u8]) -> Result<String, Status> {
    let id = required(args, "id")?;
    if valid_id(&id) {
        Ok(id)
    } else {
        Err(Status::new(
            code::INVALID_ARGUMENT,
            format!("{id:?} is not a slot id"),
        ))
    }
}

fn flag(args: &[u8], key: &str) -> bool {
    arg(args, key).is_some_and(|value| value == "true")
}

/// Applies a finished timed run to the board. Its output goes back to the caller.
fn apply(model: &mut Model, name: &str, args: &[u8]) -> Result<Vec<u8>, Status> {
    let nothing = Vec::new();
    match name {
        names::SLOTS_LIST => Ok(model.slots.list_json().into_bytes()),
        names::SLOTS_SET_DEFAULT => model
            .slots
            .set_default(&slot_id(args)?, flag(args, "force"))
            .map(|()| nothing),
        names::SLOTS_TRIAL => model
            .slots
            .trial(
                &slot_id(args)?,
                flag(args, "keep_default"),
                flag(args, "keep_old"),
                flag(args, "force"),
            )
            .map(|()| nothing),
        names::SLOTS_CONFIRM => model.slots.confirm().map(|()| nothing),
        names::SLOTS_REMOVE => model.slots.remove(&slot_id(args)?).map(|()| nothing),
        names::SLOTS_PIN => model
            .slots
            .pin(&slot_id(args)?, flag(args, "pinned"))
            .map(|()| nothing),
        names::SLOTS_SET_KEEP => {
            let keep: u32 = required(args, "keep")?
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
        names::DATA_BACKUP => Ok(model.data_backup()),
        names::DATA_RESTORE => model.data_restore(args).map(|()| nothing),
        names::FS_LIST => {
            let prefix = arg(args, "path").unwrap_or_default();
            let mut text = String::new();
            for path in model.data.keys().filter(|path| path.starts_with(&prefix)) {
                text.push_str(path);
                text.push('\n');
            }
            Ok(text.into_bytes())
        }
        names::FS_READ => {
            let path = required(args, "path")?;
            model
                .data
                .get(&path)
                .cloned()
                .ok_or_else(|| Status::new(code::NOT_FOUND, format!("there is no file {path}")))
        }
        names::FS_WRITE => {
            let path = required(args, "path")?;
            let body = args
                .split(|byte| *byte == b'\n')
                .skip(1)
                .collect::<Vec<_>>()
                .join(&b'\n');
            model.data.insert(path, body);
            Ok(nothing)
        }
        names::FS_REMOVE => {
            let path = required(args, "path")?;
            match model.data.remove(&path) {
                Some(_) => Ok(nothing),
                None => Err(Status::new(
                    code::NOT_FOUND,
                    format!("there is no file {path}"),
                )),
            }
        }
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
