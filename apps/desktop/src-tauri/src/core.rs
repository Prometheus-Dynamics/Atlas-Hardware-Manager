//! The app's state and background work.
//!
//! [`Core::open`] reads what was saved (the inventory, the settings) and returns: the window can
//! then show the last known inventory at once. The scan starts in the background and reports by
//! events; [`Core`] folds those into the inventory and publishes the result as topics
//! ([`crate::view`]). No method here waits on a device.
//!
//! When a device goes away: the scan's `Discovered` event is the complete list of addresses one
//! driver found on one link. A record is marked absent only when every driver that serves each
//! link it was seen on has answered since the app opened and none of them lists it. Until then a
//! record from the last run is shown as it was last known, and one slow driver never makes
//! another driver's devices look gone.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atlas_driver::DriverRegistry;
use atlas_inventory::{Inventory, JsonFileStore, LoadOutcome, Outcome, Presence};
use atlas_jobs::{
    Backoff, DeviceHolds, Event, Events, JobKind, JobTiming, Jobs, LinkSource, ScanConfig,
    ScanControl, start_scan,
};
use atlas_log::Logger;
use atlas_types::{DeviceKey, Family, Link, LinkId, LinkKind};
use serde_json::Value;
use tokio::sync::{Notify, broadcast};

use crate::drivers::TeamList;
use crate::paths::Dirs;
use crate::settings::Settings;
use crate::view::{InventoryView, ScanProblem, ScanView, Update, topic};

/// Which devices the app looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The computer's own links and the real drivers.
    Real,
    /// Simulated devices only (feature `sim`). Nothing on the computer is touched.
    Sim,
}

impl Mode {
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Sim => "sim",
        }
    }
}

/// What [`Core::open`] is given.
#[derive(Debug)]
pub struct Options {
    /// Where the app's files live. `None` runs without saving anything.
    pub dirs: Option<Dirs>,
    pub mode: Mode,
    pub scan: ScanConfig,
    /// Which simulated world the simulated mode builds: `demo` (one device in every state) or
    /// `empty` (nothing plugged in). Read only in the simulated mode.
    pub scene: String,
    /// Divides every wait between tries (the jobs' backoff and the update feed's), keeping its
    /// doubling and its cap in the same proportion. 1 in the app; only the UI bridge, which
    /// always runs the simulated world, sets more, so the click tests see a whole retry sequence
    /// in a fraction of the time.
    pub retry_scale: u32,
}

impl Options {
    #[must_use]
    pub fn new(dirs: Option<Dirs>, mode: Mode) -> Self {
        Self {
            dirs,
            mode,
            scan: ScanConfig::default(),
            scene: "demo".into(),
            retry_scale: 1,
        }
    }
}

/// The links the scan looks at. The host keeps the list current; the scan reads a snapshot.
#[derive(Debug, Default)]
pub struct Links {
    links: Mutex<Vec<Link>>,
}

impl Links {
    pub fn set(&self, links: Vec<Link>) {
        *lock(&self.links) = links;
    }

    #[must_use]
    pub fn get(&self) -> Vec<Link> {
        lock(&self.links).clone()
    }

    fn kind_of(&self, id: &LinkId) -> Option<LinkKind> {
        lock(&self.links)
            .iter()
            .find(|link| &link.id == id)
            .map(|link| link.kind.clone())
    }
}

impl LinkSource for Links {
    fn links(&self) -> Vec<Link> {
        self.get()
    }
}

/// What the app was built from before the scan starts: the drivers, the links they look at, and
/// the job kinds. A mode fills one in ([`crate::sim`], or the real drivers).
#[derive(Default)]
pub struct Parts {
    pub registry: DriverRegistry,
    pub links: Vec<Link>,
    pub job_kinds: Vec<Arc<dyn JobKind>>,
    /// The session registry the board driver keeps its nodes in. The board page reaches a node
    /// through it, so a node has one connection.
    pub sessions: Option<Arc<atlas_session::Sessions>>,
    /// The boot ROM loaders this build has, one per chip family (the flash flow).
    pub roms: Vec<Arc<dyn crate::flash::BootRom>>,
    /// What writes a removable disk. `None` leaves the choice to the core (the helper).
    pub disk_writer: Option<Arc<dyn crate::flash::DiskWriter>>,
    /// The network the app's own updates are read through. `None` is the real one.
    pub fetch: Option<Arc<dyn atlas_release::Fetch>>,
}

impl std::fmt::Debug for Parts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parts")
            .field("links", &self.links)
            .finish_non_exhaustive()
    }
}

/// What the scan has said since the app opened.
#[derive(Debug, Default)]
struct Seen {
    /// The addresses each driver last listed on each link.
    present: BTreeMap<(Family, LinkId), Vec<String>>,
    /// Who answered at each address.
    answered: BTreeMap<(LinkId, String), DeviceKey>,
    /// Discoveries failing now.
    problems: BTreeMap<(Family, LinkId), ScanProblem>,
    paused: bool,
}

struct Inner {
    mode: Mode,
    dirs: Option<Dirs>,
    log: Logger,
    events: Events,
    updates: broadcast::Sender<Update>,
    latest: Mutex<BTreeMap<String, Value>>,
    inventory: Mutex<Inventory>,
    store: Option<JsonFileStore>,
    save_wanted: Notify,
    settings: Mutex<Settings>,
    links: Arc<Links>,
    /// The teams the robot drivers look for, kept equal to the inventory's robots.
    teams: TeamList,
    registry: DriverRegistry,
    seen: Mutex<Seen>,
    jobs: Jobs,
    holds: DeviceHolds,
    scan: Mutex<Option<ScanControl>>,
    sessions: Option<Arc<atlas_session::Sessions>>,
    boards: crate::board::Boards,
    flashes: crate::flash::Flashes,
    app_update: crate::update::Updates,
    trust: crate::trust::Trust,
    /// What nothing-opened-yet falls back to, and what a test reads.
    remembered: Arc<crate::open::Remembered>,
    opener: Mutex<Option<Arc<dyn crate::open::Opener>>>,
    #[cfg(feature = "sim")]
    sim: Option<crate::sim::World>,
}

/// The app's core. Cheap to clone; clones share one state.
#[derive(Clone)]
pub struct Core {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core")
            .field("mode", &self.inner.mode)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Core {
    /// Reads what was saved, starts the background work and returns. Needs a running tokio
    /// runtime. It never fails: a file that cannot be used is put aside and said in the log, and
    /// the app opens without it.
    pub async fn open(options: Options) -> Self {
        let Options {
            dirs,
            mode,
            scan,
            scene,
            retry_scale,
        } = options;
        let mut timing = atlas_files::job_timing();
        timing.backoff = scaled(timing.backoff, retry_scale);
        let log = dirs
            .as_ref()
            .and_then(|dirs| Logger::open(dirs.logs(), atlas_log::Config::default()).ok())
            .unwrap_or_else(Logger::disabled);
        log.breadcrumb(&format!(
            "app opened, version {}, {} devices",
            crate::VERSION,
            mode.word()
        ));

        let store = dirs
            .as_ref()
            .map(|dirs| JsonFileStore::new(dirs.inventory()));
        let inventory = match &store {
            Some(store) => {
                let (inventory, outcome) = store.load();
                match outcome {
                    LoadOutcome::Fresh => {}
                    LoadOutcome::Restored {
                        skipped_records,
                        dropped_values,
                    } => {
                        if skipped_records + dropped_values > 0 {
                            log.notice(&format!(
                                "inventory restored: {skipped_records} records and {dropped_values} values this build does not know were left out"
                            ));
                        }
                    }
                    LoadOutcome::Reset { reason, aside } => log.notice(&format!(
                        "the saved inventory could not be used ({reason}); starting empty, the file was kept at {aside:?}"
                    )),
                }
                inventory
            }
            None => Inventory::new(),
        };
        let settings = dirs
            .as_ref()
            .map(|dirs| Settings::load(&dirs.settings()))
            .unwrap_or_default();

        let teams = TeamList::default();
        teams.follow(&inventory);
        #[cfg(feature = "sim")]
        let (parts, sim, unused) = if mode == Mode::Sim {
            let mut parts = Parts::default();
            let (world, unused) = crate::sim::World::install(
                &mut parts,
                crate::sim::Scene::named(&scene),
                &teams,
                retry_scale,
            );
            (parts, Some(world), unused)
        } else {
            let (parts, unused) = crate::drivers::real(&teams);
            (parts, None, unused)
        };
        #[cfg(not(feature = "sim"))]
        let (parts, unused) = {
            let _ = scene;
            crate::drivers::real(&teams)
        };
        for line in unused {
            log.notice(&format!("left out at start: {line}"));
        }
        let Parts {
            registry,
            links: link_list,
            mut job_kinds,
            sessions,
            roms,
            disk_writer,
            fetch,
        } = parts;
        // A real disk is written by the privileged helper, when it is beside the app.
        let disk_writer = disk_writer.or_else(|| {
            let work = dirs.as_ref()?.state.join("disk-writes");
            let helper = crate::flash::HelperWriter::find(&work)?;
            Some(Arc::new(helper) as Arc<dyn crate::flash::DiskWriter>)
        });
        let flashes = crate::flash::Flashes::new(
            roms,
            disk_writer,
            dirs.as_ref().map(|dirs| dirs.data.as_path()),
        );
        job_kinds.push(flashes.kind());
        let app_update = crate::update::Updates::new(
            fetch,
            dirs.as_ref().map(|dirs| dirs.data.as_path()),
            scaled(crate::update::RETRY, retry_scale),
        );
        job_kinds.push(app_update.kind());

        let events = Events::new(1024);
        let holds = DeviceHolds::new();
        let jobs = open_jobs(dirs.as_ref(), job_kinds, &events, &holds, timing, &log).await;
        let links = Arc::new(Links::default());
        links.set(link_list);
        let (updates, _) = broadcast::channel(256);

        let boards = crate::board::Boards::new(dirs.as_ref().map(|dirs| dirs.data.as_path()));
        #[cfg(feature = "sim")]
        if let Some(world) = &sim {
            boards.set_picker(world.picker());
        }
        let trust = crate::trust::Trust::new(dirs.as_ref().map(|dirs| dirs.data.as_path()));
        let core = Self {
            inner: Arc::new(Inner {
                mode,
                dirs,
                log,
                events: events.clone(),
                updates,
                latest: Mutex::new(BTreeMap::new()),
                inventory: Mutex::new(inventory),
                store,
                save_wanted: Notify::new(),
                settings: Mutex::new(settings),
                links: Arc::clone(&links),
                teams,
                registry: registry.clone(),
                seen: Mutex::new(Seen::default()),
                jobs,
                holds: holds.clone(),
                scan: Mutex::new(None),
                sessions,
                boards,
                flashes,
                app_update,
                trust,
                remembered: Arc::new(crate::open::Remembered::default()),
                opener: Mutex::new(None),
                #[cfg(feature = "sim")]
                sim,
            }),
        };
        core.publish_inventory();
        core.publish_scan();
        core.publish_jobs();
        core.publish_settings();
        crate::trust::publish(&core);

        // Subscribe before the scan starts, so its first event is not missed.
        let receiver = events.subscribe();
        tokio::spawn(core.clone().follow(receiver));
        tokio::spawn(core.clone().save_when_wanted());
        tokio::spawn(core.flashes().clone().follow(core.clone()));
        tokio::spawn(core.app_update().clone().follow(core.clone()));
        let control = start_scan(registry, links, events, scan, holds);
        *lock(&core.inner.scan) = Some(control);
        #[cfg(feature = "sim")]
        if let Some(world) = core.sim() {
            world.settle(&core);
        }
        core
    }

    /// The app's own update.
    #[must_use]
    pub fn app_update(&self) -> &crate::update::Updates {
        &self.inner.app_update
    }

    /// The flash screens' state.
    #[must_use]
    pub fn flashes(&self) -> &crate::flash::Flashes {
        &self.inner.flashes
    }

    #[must_use]
    pub fn mode(&self) -> &Mode {
        &self.inner.mode
    }

    #[must_use]
    pub fn log(&self) -> &Logger {
        &self.inner.log
    }

    #[must_use]
    pub fn dirs(&self) -> Option<&Dirs> {
        self.inner.dirs.as_ref()
    }

    /// The bus the scan and the jobs report on. A page's own job kinds report here too.
    #[must_use]
    pub fn events(&self) -> &Events {
        &self.inner.events
    }

    #[must_use]
    pub fn jobs(&self) -> &Jobs {
        &self.inner.jobs
    }

    /// The session registry the scan keeps Orion nodes in, when the board driver is registered.
    #[must_use]
    pub fn sessions(&self) -> Option<&Arc<atlas_session::Sessions>> {
        self.inner.sessions.as_ref()
    }

    /// The board pages' shared state.
    #[must_use]
    pub fn boards(&self) -> &crate::board::Boards {
        &self.inner.boards
    }

    /// The trusted keys and the record of signature checks.
    #[must_use]
    pub fn trust(&self) -> &crate::trust::Trust {
        &self.inner.trust
    }

    /// What opens a folder or a page outside the app: the window's, else one that only
    /// remembers what was asked.
    #[must_use]
    pub fn opener(&self) -> Arc<dyn crate::open::Opener> {
        lock(&self.inner.opener)
            .clone()
            .unwrap_or_else(|| self.inner.remembered.clone())
    }

    pub fn set_opener(&self, opener: Arc<dyn crate::open::Opener>) {
        *lock(&self.inner.opener) = Some(opener);
    }

    /// What was asked to be opened where there is no window.
    #[must_use]
    pub fn opened(&self) -> Vec<String> {
        self.inner.remembered.asked()
    }

    /// Publishes the inventory again: something beside the records changed (a board's default
    /// slot was read).
    pub fn republish_inventory(&self) {
        self.publish_inventory();
    }

    #[must_use]
    pub fn holds(&self) -> &DeviceHolds {
        &self.inner.holds
    }

    /// The simulated devices, in the simulated mode.
    #[cfg(feature = "sim")]
    #[must_use]
    pub fn sim(&self) -> Option<&crate::sim::World> {
        self.inner.sim.as_ref()
    }

    /// Reads the inventory under its lock. Keep the closure short.
    pub fn with_inventory<T>(&self, read: impl FnOnce(&Inventory) -> T) -> T {
        read(&lock(&self.inner.inventory))
    }

    /// Changes the inventory, then publishes and saves it.
    pub fn change_inventory<T>(&self, change: impl FnOnce(&mut Inventory, u64) -> T) -> T {
        let result = change(&mut lock(&self.inner.inventory), now_ms());
        self.inventory_changed();
        result
    }

    // ----- topics ---------------------------------------------------------------------------

    /// Follows every update from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Update> {
        self.inner.updates.subscribe()
    }

    /// The latest value of every topic.
    #[must_use]
    pub fn topics(&self) -> BTreeMap<String, Value> {
        lock(&self.inner.latest).clone()
    }

    /// Sets a topic's value and tells every subscriber. A value equal to the current one is not
    /// sent again.
    pub fn publish(&self, topic: &str, data: Value) {
        {
            let mut latest = lock(&self.inner.latest);
            if latest.get(topic) == Some(&data) {
                return;
            }
            latest.insert(topic.to_owned(), data.clone());
        }
        let _ = self.inner.updates.send(Update {
            topic: topic.to_owned(),
            data,
        });
    }

    /// Drops a topic (a page that closed). Subscribers are told with a null value.
    pub fn retire(&self, topic: &str) {
        if lock(&self.inner.latest).remove(topic).is_some() {
            let _ = self.inner.updates.send(Update {
                topic: topic.to_owned(),
                data: Value::Null,
            });
        }
    }

    fn publish_as<T: serde::Serialize>(&self, topic: &str, value: &T) {
        match serde_json::to_value(value) {
            Ok(data) => self.publish(topic, data),
            Err(error) => self.inner.log.ui_error(
                "core",
                &format!("topic {topic} could not be written: {error}"),
            ),
        }
    }

    fn publish_inventory(&self) {
        let mut view = self.with_inventory(|inventory| InventoryView::of(inventory, now_ms()));
        let defaults = self.inner.boards.defaults();
        for device in &mut view.devices {
            device.default_version = defaults.get(&device.id).cloned();
        }
        self.publish_as(topic::INVENTORY, &view);
    }

    fn publish_scan(&self) {
        let links = self.inner.links.get();
        let seen = lock(&self.inner.seen);
        let view = ScanView {
            paused: seen.paused,
            links: links.clone(),
            first_round_done: links
                .iter()
                .all(|link| self.link_reported(&seen, &link.id, &link.kind)),
            problems: seen.problems.values().cloned().collect(),
        };
        drop(seen);
        self.publish_as(topic::SCAN, &view);
    }

    fn publish_jobs(&self) {
        self.publish_as(topic::JOBS, &self.inner.jobs.list());
    }

    fn publish_settings(&self) {
        let settings = lock(&self.inner.settings).clone();
        self.publish_as(topic::SETTINGS, &settings);
    }

    fn inventory_changed(&self) {
        self.with_inventory(|inventory| self.inner.teams.follow(inventory));
        self.publish_inventory();
        self.inner.save_wanted.notify_one();
    }

    // ----- settings -------------------------------------------------------------------------

    #[must_use]
    pub fn settings(&self) -> Settings {
        lock(&self.inner.settings).clone()
    }

    /// Replaces the settings, publishes them and saves them. A value the app does not offer goes
    /// back to its default.
    pub async fn set_settings(&self, settings: Settings) -> Result<Settings, String> {
        let settings = settings.checked();
        *lock(&self.inner.settings) = settings.clone();
        self.publish_settings();
        if let Some(dirs) = self.inner.dirs.clone() {
            let saved = settings.clone();
            tokio::task::spawn_blocking(move || saved.save(&dirs.settings()))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| format!("the settings could not be saved: {error}"))?;
        }
        Ok(settings)
    }

    // ----- the scan -------------------------------------------------------------------------

    /// The manual off: the scan starts no new calls until it is resumed.
    pub fn pause_scan(&self) {
        if let Some(control) = lock(&self.inner.scan).as_ref() {
            control.pause_all();
        }
    }

    pub fn resume_scan(&self) {
        if let Some(control) = lock(&self.inner.scan).as_ref() {
            control.resume_all();
        }
    }

    /// Whether every driver that serves this link has answered on it since the app opened. A
    /// link that is not there any more has nothing left to wait for.
    fn link_reported(&self, seen: &Seen, link: &LinkId, kind: &LinkKind) -> bool {
        self.inner.registry.for_link(kind).iter().all(|driver| {
            seen.present
                .contains_key(&(driver.manifest().family.clone(), link.clone()))
        })
    }

    async fn follow(self, mut receiver: broadcast::Receiver<Event>) {
        loop {
            match receiver.recv().await {
                Ok(event) => self.take(event),
                // Fell behind: the next round says everything again, and the job list is read whole.
                Err(broadcast::error::RecvError::Lagged(_)) => self.publish_jobs(),
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    fn take(&self, event: Event) {
        match event {
            Event::Identified {
                identity,
                capabilities,
            } => {
                let kind = self
                    .inner
                    .links
                    .kind_of(&identity.link)
                    // The link went away between the answer and this event: a routed network is
                    // the kind that claims least.
                    .unwrap_or(LinkKind::Ethernet);
                lock(&self.inner.seen).answered.insert(
                    (identity.link.clone(), identity.address.clone()),
                    identity.key.clone(),
                );
                let key = identity.key.clone();
                let outcome = {
                    let mut inventory = lock(&self.inner.inventory);
                    let outcome = inventory.observe(identity, kind, capabilities, now_ms());
                    // A robot's own device joins the robot of its team.
                    let key = inventory.resolve(&key);
                    crate::robot::place(&mut inventory, &key, now_ms());
                    outcome
                };
                // An answer is rare (a device is asked who it is when it appears and then about
                // once a minute), and even one that changes nothing else moves when each link
                // last answered, which is what the list shows a record's links by.
                let _: Outcome = outcome;
                self.inventory_changed();
            }
            Event::Discovered {
                family,
                link,
                addresses,
            } => {
                {
                    let mut seen = lock(&self.inner.seen);
                    seen.present
                        .insert((family.clone(), link.clone()), addresses);
                    seen.problems.remove(&(family, link));
                }
                self.mark_absent();
                self.publish_scan();
            }
            Event::ProbeFailed {
                family,
                link,
                error,
                timed_out,
                attempt,
                retry_in_ms,
            } => {
                lock(&self.inner.seen).problems.insert(
                    (family.clone(), link.clone()),
                    ScanProblem {
                        family: family.to_string(),
                        link: link.to_string(),
                        error,
                        timed_out,
                        attempt,
                        retry_in_ms,
                    },
                );
                self.publish_scan();
            }
            Event::ProbeRecovered { family, link } => {
                lock(&self.inner.seen).problems.remove(&(family, link));
                self.publish_scan();
            }
            Event::ScanPaused { family: None } => {
                lock(&self.inner.seen).paused = true;
                self.publish_scan();
            }
            Event::ScanResumed { family: None } => {
                lock(&self.inner.seen).paused = false;
                self.publish_scan();
            }
            Event::Job { .. } => self.publish_jobs(),
            Event::StoreFailed { message } => self.inner.log.failed_call("jobs.save", "", &message),
            Event::Unanswered { .. }
            | Event::Settled { .. }
            | Event::ScanPaused { .. }
            | Event::ScanResumed { .. } => {}
        }
    }

    /// Marks absent the online records that no driver lists any more. See the module text for
    /// the rule.
    ///
    /// A record is still there when a driver lists an address it was seen at and nobody else
    /// has answered at that address since; an address that is listed but has not been asked who
    /// it is yet still counts for the record last seen there.
    fn mark_absent(&self) {
        let links = self.inner.links.get();
        let still_there: BTreeSet<DeviceKey> = {
            let seen = lock(&self.inner.seen);
            let listed = |link: &LinkId, address: &str| {
                seen.present.iter().any(|((_, on), addresses)| {
                    on == link && addresses.iter().any(|a| a == address)
                })
            };
            self.with_inventory(|inventory| {
                inventory
                    .all()
                    .into_iter()
                    .filter(|record| record.presence == Presence::Online)
                    .filter(|record| {
                        record.links.iter().any(|sighting| {
                            let answered = seen
                                .answered
                                .get(&(sighting.link.clone(), sighting.address.clone()));
                            let here = listed(&sighting.link, &sighting.address)
                                && answered.is_none_or(|key| inventory.resolve(key) == record.key);
                            // Or a driver that serves this link has not answered yet.
                            let waiting = links.iter().any(|link| link.id == sighting.link)
                                && !self.link_reported(&seen, &sighting.link, &sighting.kind);
                            here || waiting
                        })
                    })
                    .map(|record| record.key.clone())
                    .collect()
            })
        };
        let gone = lock(&self.inner.inventory).mark_absent(&still_there, now_ms());
        if !gone.is_empty() {
            self.inventory_changed();
        }
    }

    /// Saves the inventory when it changed, at most every two seconds, off the async threads.
    async fn save_when_wanted(self) {
        let Some(store) = self.inner.store.clone() else {
            return;
        };
        loop {
            self.inner.save_wanted.notified().await;
            let inventory = lock(&self.inner.inventory).clone();
            let store = store.clone();
            let saved = tokio::task::spawn_blocking(move || store.save(&inventory)).await;
            if let Ok(Err(error)) = saved {
                self.inner
                    .log
                    .failed_call("inventory.save", "", &error.to_string());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Saves the inventory now. Called when the window closes.
    pub async fn save_now(&self) {
        if let Some(store) = self.inner.store.clone() {
            let inventory = lock(&self.inner.inventory).clone();
            let _ = tokio::task::spawn_blocking(move || store.save(&inventory)).await;
        }
        self.inner.log.flush();
    }
}

/// Opens the job list. A list that cannot be read is put aside (nothing in it is lost) and the
/// app opens with an empty one.
/// `backoff` with its first wait and its cap divided by `scale` (see [`Options::retry_scale`]).
#[must_use]
pub fn scaled(backoff: Backoff, scale: u32) -> Backoff {
    let scale = scale.max(1);
    Backoff {
        first: backoff.first / scale,
        cap: backoff.cap / scale,
    }
}

async fn open_jobs(
    dirs: Option<&Dirs>,
    kinds: Vec<Arc<dyn JobKind>>,
    events: &Events,
    holds: &DeviceHolds,
    timing: JobTiming,
    log: &Logger,
) -> Jobs {
    let path = match dirs {
        Some(dirs) => dirs.jobs(),
        // Nowhere to save: a path that is never written unless a job is submitted.
        None => std::env::temp_dir().join(format!("atlas-jobs-{}.json", std::process::id())),
    };
    let open = |path: std::path::PathBuf| {
        Jobs::open(path, kinds.clone(), events.clone(), timing, holds.clone())
    };
    match open(path.clone()).await {
        Ok(jobs) => jobs,
        Err(error) => {
            let aside = path.with_extension(format!("unreadable-{}.json", now_ms()));
            let moved = std::fs::rename(&path, &aside).is_ok();
            log.notice(&format!(
                "the saved job list could not be read ({error}); starting empty, the file was {}",
                if moved {
                    format!("kept at {}", aside.display())
                } else {
                    "left in place".into()
                }
            ));
            let fresh = if moved { path } else { aside };
            match open(fresh).await {
                Ok(jobs) => jobs,
                Err(error) => {
                    log.notice(&format!("the job list cannot be opened at all: {error}"));
                    open(std::env::temp_dir().join(format!("atlas-jobs-{}.json", now_ms())))
                        .await
                        .expect("an empty job list at a new path always opens")
                }
            }
        }
    }
}
