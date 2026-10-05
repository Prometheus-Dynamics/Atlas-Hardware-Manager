use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_driver::{
    ChangeNotifier, DeviceKey, DeviceMode, Driver, Family, Link, LinkId, LinkKind, LinkSource,
    UpdateStep,
};

use crate::{MockDriver, SIM_HELIOS, SIM_MCU};

/// The id of the one simulated link every top-level mock device sits on.
pub const SIM_LINK_ID: &str = "sim0";

/// How a simulated device behaves when updated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MockBehavior {
    #[default]
    Succeeds,
    /// Fails at this step. Before `Apply` nothing changes on the device.
    FailsAt(UpdateStep),
    /// Never confirms the new version and rolls back to the old one.
    NeverConfirms,
    /// Is left unusable after `Apply` and needs recovery.
    BricksAfterApply,
    /// The driver panics mid-update, like a library bug would.
    Panics,
}

/// One simulated device.
#[derive(Clone, Debug)]
pub struct MockDevice {
    pub key: DeviceKey,
    pub model: String,
    pub name: Option<String>,
    pub version: String,
    pub mode: DeviceMode,
    pub behavior: MockBehavior,
    /// The gateway this device sits behind, if any.
    pub parent: Option<DeviceKey>,
    pub online: bool,
    /// Simulated time per update step.
    pub step_time: Duration,
    /// When the device last started; uptime and logs count from here.
    pub booted: Instant,
}

impl MockDevice {
    pub fn new(family: &str, serial: &str) -> Self {
        Self {
            key: DeviceKey::new(family, serial),
            model: format!("{family}-board"),
            name: None,
            version: "1.0.0".into(),
            mode: DeviceMode::Normal,
            behavior: MockBehavior::Succeeds,
            parent: None,
            online: true,
            step_time: Duration::from_millis(40),
            // Simulated devices have been up a while when Atlas starts.
            booted: Instant::now()
                .checked_sub(Duration::from_secs(3 * 3600 + 17 * 60))
                .unwrap_or_else(Instant::now),
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = model.into();
        self
    }

    pub fn version(mut self, version: &str) -> Self {
        self.version = version.into();
        self
    }

    /// Starts in a bootloader or boot-ROM mode that only accepts recovery.
    pub fn recovery(mut self) -> Self {
        self.mode = DeviceMode::Recovery;
        self
    }

    pub fn behavior(mut self, behavior: MockBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    pub fn behind(mut self, gateway: &DeviceKey) -> Self {
        self.parent = Some(gateway.clone());
        self
    }

    pub fn step_time(mut self, step_time: Duration) -> Self {
        self.step_time = step_time;
        self
    }
}

#[derive(Default)]
pub(crate) struct FleetState {
    pub(crate) devices: BTreeMap<DeviceKey, MockDevice>,
    pub(crate) running: BTreeMap<String, usize>,
    pub(crate) peaks: BTreeMap<String, usize>,
    pub(crate) actions: Vec<(DeviceKey, String)>,
    /// Told whenever a device is plugged in or unplugged.
    pub(crate) watchers: Vec<ChangeNotifier>,
}

/// Shared state for a set of simulated devices. Cheap to clone.
#[derive(Clone, Default)]
pub struct MockFleet {
    state: Arc<Mutex<FleetState>>,
}

/// Counter name for updates running across all devices.
pub(crate) const ALL_UPDATES: &str = "all";

impl MockFleet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(self, device: MockDevice) -> Self {
        self.lock().devices.insert(device.key.clone(), device);
        self
    }

    /// A small robot: three cameras, two boards behind the front camera,
    /// and one board plugged in directly in recovery mode.
    pub fn demo() -> Self {
        let front = DeviceKey::new(SIM_HELIOS, "H-1001");
        Self::new()
            .with(camera("H-1001", "cam-front"))
            .with(camera("H-1002", "cam-left"))
            .with(camera("H-1003", "cam-rear"))
            .with(board("M-2001", "drive-mcu-1").behind(&front))
            .with(board("M-2002", "drive-mcu-2").behind(&front))
            .with(
                board("M-2003", "arm-mcu")
                    .version("bootloader-2.1")
                    .recovery(),
            )
    }

    /// The demo robot with trouble: one camera never confirms its update.
    pub fn flaky() -> Self {
        let fleet = Self::demo();
        if let Some(rear) = fleet
            .lock()
            .devices
            .get_mut(&DeviceKey::new(SIM_HELIOS, "H-1003"))
        {
            rear.behavior = MockBehavior::NeverConfirms;
        }
        fleet
    }

    /// One driver per family present in the fleet.
    pub fn drivers(&self) -> Vec<Arc<dyn Driver>> {
        let families: BTreeSet<Family> = self
            .lock()
            .devices
            .keys()
            .map(|key| key.family.clone())
            .collect();
        families
            .into_iter()
            .map(|family| Arc::new(MockDriver::new(self.clone(), family)) as Arc<dyn Driver>)
            .collect()
    }

    pub fn link_source(&self) -> Arc<dyn LinkSource> {
        Arc::new(MockLinks {
            fleet: self.clone(),
        })
    }

    /// Plugs a device in or out, and tells anyone watching, like hotplug.
    pub fn set_online(&self, key: &DeviceKey, online: bool) {
        let watchers = {
            let mut state = self.lock();
            match state.devices.get_mut(key) {
                Some(device) if device.online != online => device.online = online,
                _ => return,
            }
            state.watchers.clone()
        };
        for watcher in watchers {
            watcher.notify();
        }
    }

    pub fn device(&self, key: &DeviceKey) -> Option<MockDevice> {
        self.lock().devices.get(key).cloned()
    }

    /// The device if it is online and running normally.
    pub(crate) fn online(&self, key: &DeviceKey) -> Result<MockDevice, atlas_driver::DriverError> {
        self.device(key)
            .filter(|device| device.online && device.mode == DeviceMode::Normal)
            .ok_or_else(|| atlas_driver::DriverError::Unreachable(key.to_string()))
    }

    /// Most updates that ran at once, across all devices.
    pub fn peak_running(&self) -> usize {
        self.peak_running_for(ALL_UPDATES)
    }

    /// Most updates that ran at once while holding one exclusive resource.
    pub fn peak_running_for(&self, resource: &str) -> usize {
        self.lock().peaks.get(resource).copied().unwrap_or(0)
    }

    /// Every action run so far, oldest first.
    pub fn actions_log(&self) -> Vec<(DeviceKey, String)> {
        self.lock().actions.clone()
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, FleetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn camera(serial: &str, name: &str) -> MockDevice {
    MockDevice::new(SIM_HELIOS, serial)
        .name(name)
        .model("cm5")
        .version("2026.2.4")
}

fn board(serial: &str, name: &str) -> MockDevice {
    MockDevice::new(SIM_MCU, serial)
        .name(name)
        .model("stm32g4")
        .version("1.4.0")
}

struct MockLinks {
    fleet: MockFleet,
}

#[async_trait]
impl LinkSource for MockLinks {
    fn name(&self) -> &str {
        "Simulated"
    }

    fn watch(&self, notify: ChangeNotifier) -> bool {
        self.fleet.lock().watchers.push(notify);
        true
    }

    async fn links(&self) -> Vec<Link> {
        vec![Link {
            id: LinkId(SIM_LINK_ID.into()),
            kind: LinkKind::Simulated,
            label: "Simulated robot".into(),
        }]
    }
}
