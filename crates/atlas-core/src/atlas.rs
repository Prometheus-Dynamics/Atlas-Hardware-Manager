use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use atlas_driver::{
    Candidate, Capabilities, CapabilitySource, DeviceAction, DeviceKey, Driver, DriverRegistry,
    Identity, LinkSource,
};
use tokio::sync::{Semaphore, broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::activity::{ActivityEntry, ActivityKind, ActivityLevel};
use crate::events::EventBus;
use crate::inventory::Inventory;
use crate::jobs::{JobPlan, JobRecord, UpdateRequest};
use crate::{
    CoreError, DeviceRecord, Event, InventoryStore, JobId, RobotProfile, ScanReport, Snapshot,
    job_runner, scan,
};

/// Tunables with defaults suited to a robot on a desk.
#[derive(Clone, Debug)]
pub struct AtlasOptions {
    /// How long one device may take to answer an identify request.
    pub identify_timeout: Duration,
    /// How many gateways deep a scan follows.
    pub max_gateway_depth: usize,
    /// Upper bound on updates running at once across all jobs.
    pub max_parallel_updates: usize,
    /// Staged rollout turns on automatically at this many devices of one family.
    pub staged_rollout_threshold: usize,
    pub event_capacity: usize,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            identify_timeout: Duration::from_secs(3),
            max_gateway_depth: 3,
            max_parallel_updates: 16,
            staged_rollout_threshold: 3,
            event_capacity: 1024,
        }
    }
}

/// A device that answered in the latest scan, with the handles to act on it.
#[derive(Clone)]
pub(crate) struct LiveDevice {
    pub(crate) driver: Arc<dyn Driver>,
    pub(crate) candidate: Candidate,
    pub(crate) capabilities: Capabilities,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) inventory: Inventory,
    pub(crate) live: HashMap<DeviceKey, LiveDevice>,
    pub(crate) jobs: BTreeMap<JobId, JobRecord>,
    pub(crate) job_done: HashMap<JobId, watch::Receiver<bool>>,
    pub(crate) job_cancel: HashMap<JobId, CancellationToken>,
    pub(crate) next_job: u64,
    pub(crate) robots: BTreeMap<String, RobotProfile>,
    pub(crate) activity: VecDeque<ActivityEntry>,
    /// The last self-test per board serial.
    pub(crate) selftests: BTreeMap<String, crate::SelfTestRecord>,
    /// Boards to self-test when they come back, with a deadline (ms).
    pub(crate) selftest_pending: HashMap<String, u64>,
    /// Boards whose self-test is running now.
    pub(crate) selftest_running: HashSet<String>,
    /// Each board's recent events, by board serial, oldest first.
    pub(crate) board_events: HashMap<String, VecDeque<atlas_driver::DeviceEvent>>,
    /// When each board's events were last fetched (ms).
    pub(crate) events_synced: HashMap<String, u64>,
    /// The seq each board's events were fetched up to, for boards that number
    /// them (from the kept log when Atlas starts).
    pub(crate) event_cursor: HashMap<String, u64>,
    /// How far each boot's clock was from this computer's (board minus
    /// here, seconds), by boot id, as seen while that boot answered.
    pub(crate) clock_offsets: HashMap<String, i64>,
    /// Devices whose push channel is being followed (push.rs).
    pub(crate) followed: HashSet<DeviceKey>,
    /// When each board's clock was last set automatically (ms), by board
    /// serial (clock.rs).
    pub(crate) clock_set_at: HashMap<String, u64>,
}

pub(crate) struct Inner {
    pub(crate) registry: DriverRegistry,
    pub(crate) link_sources: Vec<Arc<dyn LinkSource>>,
    /// Extra capabilities for devices other drivers own (Orion, …).
    pub(crate) capability_sources: Vec<Arc<dyn CapabilitySource>>,
    pub(crate) store: Option<Arc<dyn InventoryStore>>,
    pub(crate) options: AtlasOptions,
    pub(crate) events: EventBus,
    pub(crate) state: Mutex<State>,
    /// One permit per exclusive host resource, shared by every job.
    pub(crate) exclusive: Mutex<HashMap<String, Arc<Semaphore>>>,
    pub(crate) parallel: Arc<Semaphore>,
    /// Scans run one at a time; a second request waits for the first.
    pub(crate) scan_lock: tokio::sync::Mutex<()>,
    pub(crate) discovery: Mutex<crate::DiscoveryStatus>,
    /// Woken by link sources when something may have changed.
    pub(crate) changes: Arc<tokio::sync::Notify>,
    /// Source names that push changes, and those that must be polled.
    pub(crate) subscriptions: std::sync::OnceLock<(Vec<String>, Vec<String>)>,
}

impl Inner {
    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn discovery(&self) -> MutexGuard<'_, crate::DiscoveryStatus> {
        self.discovery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The owning driver's capabilities, filled in from every capability
    /// source.
    pub(crate) fn capabilities(&self, driver: &dyn Driver, identity: &Identity) -> Capabilities {
        let mut capabilities = driver.capabilities(identity);
        for source in &self.capability_sources {
            capabilities.fill_from(source.capabilities_for(identity));
        }
        capabilities
    }

    pub(crate) fn exclusive_slot(&self, resource: &str) -> Arc<Semaphore> {
        self.exclusive
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(resource.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    /// Saves the inventory. A failure is reported as a warning, never fatal.
    pub(crate) fn persist(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let snapshot = {
            let state = self.state();
            Snapshot {
                devices: state.inventory.all(),
                robots: state.robots.values().cloned().collect(),
                activity: state.activity.iter().cloned().collect(),
                selftests: state.selftests.values().cloned().collect(),
                board_events: state
                    .board_events
                    .iter()
                    .map(|(board, events)| crate::BoardEventLog {
                        board_serial: board.clone(),
                        events: events.iter().cloned().collect(),
                    })
                    .collect(),
            }
        };
        if let Err(error) = store.save(&snapshot) {
            self.events.emit(Event::ScanWarning {
                message: format!("Could not save the device inventory: {error}"),
            });
        }
    }
}

pub struct AtlasBuilder {
    registry: DriverRegistry,
    link_sources: Vec<Arc<dyn LinkSource>>,
    capability_sources: Vec<Arc<dyn CapabilitySource>>,
    store: Option<Arc<dyn InventoryStore>>,
    options: AtlasOptions,
}

impl AtlasBuilder {
    pub fn driver(mut self, driver: Arc<dyn Driver>) -> Self {
        self.registry.register(driver);
        self
    }

    pub fn link_source(mut self, source: Arc<dyn LinkSource>) -> Self {
        self.link_sources.push(source);
        self
    }

    /// Adds capabilities to devices other drivers own, such as a management
    /// agent's telemetry and actions for a board found over mDNS.
    pub fn capability_source(mut self, source: Arc<dyn CapabilitySource>) -> Self {
        self.capability_sources.push(source);
        self
    }

    pub fn store(mut self, store: Arc<dyn InventoryStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn options(mut self, options: AtlasOptions) -> Self {
        self.options = options;
        self
    }

    /// Builds Atlas and restores the saved inventory, all devices offline.
    pub fn build(self) -> Result<Atlas, CoreError> {
        let snapshot = match &self.store {
            Some(store) => store.load()?,
            None => Snapshot::default(),
        };
        let state = State {
            inventory: Inventory::from_records(snapshot.devices),
            robots: snapshot
                .robots
                .into_iter()
                .map(|robot| (robot.name.clone(), robot))
                .collect(),
            activity: snapshot.activity.into_iter().collect(),
            selftests: snapshot
                .selftests
                .into_iter()
                .map(|run| (run.board_serial.clone(), run))
                .collect(),
            board_events: snapshot
                .board_events
                .into_iter()
                .map(|log| (log.board_serial, log.events.into_iter().collect()))
                .collect(),
            next_job: 1,
            ..State::default()
        };
        Ok(Atlas {
            inner: Arc::new(Inner {
                registry: self.registry,
                link_sources: self.link_sources,
                capability_sources: self.capability_sources,
                store: self.store,
                events: EventBus::new(self.options.event_capacity),
                parallel: Arc::new(Semaphore::new(self.options.max_parallel_updates.max(1))),
                options: self.options,
                state: Mutex::new(state),
                exclusive: Mutex::new(HashMap::new()),
                scan_lock: tokio::sync::Mutex::new(()),
                discovery: Mutex::new(crate::DiscoveryStatus::default()),
                changes: Arc::new(tokio::sync::Notify::new()),
                subscriptions: std::sync::OnceLock::new(),
            }),
        })
    }
}

/// The Atlas core. Cheap to clone; all clones share one state.
#[derive(Clone)]
pub struct Atlas {
    pub(crate) inner: Arc<Inner>,
}

impl Atlas {
    pub fn builder() -> AtlasBuilder {
        AtlasBuilder {
            registry: DriverRegistry::new(),
            link_sources: Vec::new(),
            capability_sources: Vec::new(),
            store: None,
            options: AtlasOptions::default(),
        }
    }

    /// Whether any device driver is registered.
    pub fn has_drivers(&self) -> bool {
        !self.inner.registry.is_empty()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// Finds every device reachable now. Devices are published as
    /// [`Event::DeviceSeen`] as they answer, before the scan completes.
    pub async fn scan(&self) -> ScanReport {
        let report = scan::run(&self.inner).await;
        // Boards back from a flash or an update get their self-test.
        self.start_due_selftests();
        // Boards with an event log: fetch what happened since.
        self.start_due_event_syncs();
        report
    }

    /// Every known device, online first, then by name.
    pub fn devices(&self) -> Vec<DeviceRecord> {
        let mut devices = self.inner.state().inventory.all();
        devices.sort_by(|a, b| {
            (a.presence as u8, a.display_name()).cmp(&(b.presence as u8, b.display_name()))
        });
        devices
    }

    pub fn device(&self, key: &DeviceKey) -> Option<DeviceRecord> {
        self.inner.state().inventory.get(key).cloned()
    }

    /// Actions the device offers, such as locate or reboot.
    pub fn actions(&self, key: &DeviceKey) -> Result<Vec<DeviceAction>, CoreError> {
        let (live, record) = self.live_device(key)?;
        let actions = live
            .capabilities
            .actions
            .ok_or_else(|| CoreError::NoActionsCapability(key.clone()))?;
        Ok(actions.actions(&record.identity))
    }

    pub async fn run_action(&self, key: &DeviceKey, action: &str) -> Result<(), CoreError> {
        let (live, record) = self.live_device(key)?;
        let actions = live
            .capabilities
            .actions
            .ok_or_else(|| CoreError::NoActionsCapability(key.clone()))?;
        if !actions
            .actions(&record.identity)
            .iter()
            .any(|candidate| candidate.id == action)
        {
            return Err(CoreError::UnknownAction {
                device: key.clone(),
                action: action.to_string(),
            });
        }
        let label = actions
            .actions(&record.identity)
            .into_iter()
            .find(|candidate| candidate.id == action)
            .map_or_else(|| action.to_string(), |candidate| candidate.label);
        let result = actions.run_action(&record.identity, action).await;
        let name = record.display_name();
        self.inner.record_activity(vec![match &result {
            Ok(()) => ActivityEntry::about(
                &record,
                ActivityKind::ActionRun,
                ActivityLevel::Info,
                format!("{label} on {name}"),
            ),
            Err(error) => ActivityEntry::about(
                &record,
                ActivityKind::ActionRun,
                ActivityLevel::Error,
                format!("{label} on {name} failed: {error}"),
            ),
        }]);
        self.inner.persist();
        Ok(result?)
    }

    /// Describes what an update would do, per device, without starting it.
    pub fn plan_update(&self, request: &UpdateRequest) -> Result<JobPlan, CoreError> {
        job_runner::plan(&self.inner, request)
    }

    /// Starts an update job and returns at once. Follow it with
    /// [`subscribe`](Self::subscribe) or [`wait_job`](Self::wait_job).
    pub fn start_update(&self, request: UpdateRequest) -> Result<JobId, CoreError> {
        job_runner::start(&self.inner, request)
    }

    pub fn job(&self, id: JobId) -> Option<JobRecord> {
        self.inner.state().jobs.get(&id).cloned()
    }

    /// All jobs this session, newest first.
    pub fn jobs(&self) -> Vec<JobRecord> {
        self.inner.state().jobs.values().rev().cloned().collect()
    }

    /// Requests cancellation. Devices past the point of no return finish.
    pub fn cancel_job(&self, id: JobId) -> Result<(), CoreError> {
        let token = self
            .inner
            .state()
            .job_cancel
            .get(&id)
            .cloned()
            .ok_or(CoreError::UnknownJob(id))?;
        token.cancel();
        Ok(())
    }

    /// Waits for a job to finish and returns its final record.
    pub async fn wait_job(&self, id: JobId) -> Result<JobRecord, CoreError> {
        let mut done = self
            .inner
            .state()
            .job_done
            .get(&id)
            .cloned()
            .ok_or(CoreError::UnknownJob(id))?;
        // The sender lives until the job ends, so an error also means finished.
        let _ = done.wait_for(|finished| *finished).await;
        self.job(id).ok_or(CoreError::UnknownJob(id))
    }

    pub(crate) fn live_device(
        &self,
        key: &DeviceKey,
    ) -> Result<(LiveDevice, DeviceRecord), CoreError> {
        let state = self.inner.state();
        let record = state
            .inventory
            .get(key)
            .cloned()
            .ok_or_else(|| CoreError::UnknownDevice(key.clone()))?;
        let live = state
            .live
            .get(key)
            .cloned()
            .ok_or_else(|| CoreError::DeviceOffline(key.clone()))?;
        Ok((live, record))
    }
}
