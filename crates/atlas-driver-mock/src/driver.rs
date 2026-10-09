use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{
    ActionsCapability, CancellationToken, Candidate, Capabilities, Concurrency, DeviceAction,
    DeviceKey, DeviceMode, Driver, DriverError, DriverManifest, Family, Identity, Link, LinkId,
    LinkKind, ProgressSink, ReleaseRef, UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};

use crate::fleet::ALL_UPDATES;
use crate::observe::{MockLogs, MockTelemetry, TEST_PATTERN};
use crate::{MockBehavior, MockDevice, MockFleet, SIM_HELIOS};

const STEPS: [UpdateStep; 5] = [
    UpdateStep::Preflight,
    UpdateStep::Transfer,
    UpdateStep::Apply,
    UpdateStep::Reboot,
    UpdateStep::Confirm,
];

/// A driver for one simulated family.
pub struct MockDriver {
    fleet: MockFleet,
    manifest: DriverManifest,
}

impl MockDriver {
    pub fn new(fleet: MockFleet, family: Family) -> Self {
        Self {
            manifest: DriverManifest {
                name: format!("Simulated {family}"),
                family,
                version: env!("CARGO_PKG_VERSION").into(),
                link_kinds: vec![LinkKind::Simulated],
                priority: 0,
            },
            fleet,
        }
    }

    fn device(&self, key: &DeviceKey) -> Result<MockDevice, DriverError> {
        self.fleet
            .device(key)
            .filter(|device| device.online)
            .ok_or_else(|| DriverError::Unreachable(key.to_string()))
    }
}

/// The version name a family reports: an OS for gateways, firmware for boards.
fn version_name(device: &MockDevice) -> &'static str {
    match (device.mode, device.key.family.as_str()) {
        (DeviceMode::Recovery, _) => "bootloader",
        (DeviceMode::Normal, SIM_HELIOS) => "os",
        (DeviceMode::Normal, _) => "firmware",
    }
}

/// What a running simulated device tells Atlas beyond its version.
fn attributes(device: &MockDevice) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    if let Some(serial) = &device.board_serial {
        attributes.insert(
            atlas_driver::attributes::BOARD_SERIAL.into(),
            serial.clone(),
        );
    }
    if device.mode == DeviceMode::Recovery {
        return attributes;
    }
    if device.key.family.as_str() == SIM_HELIOS {
        let host = device
            .name
            .clone()
            .unwrap_or_else(|| device.key.serial.0.to_ascii_lowercase());
        attributes.insert(
            atlas_driver::attributes::HOSTNAME.into(),
            format!("{host}.local"),
        );
        attributes.insert(
            atlas_driver::attributes::WEB_UI.into(),
            format!("http://{host}.local:5800/"),
        );
        attributes.insert(
            atlas_driver::attributes::CAMERA_STREAM.into(),
            TEST_PATTERN.into(),
        );
        attributes.insert("os".into(), "helios".into());
        attributes.insert("pipeline".into(), "apriltag".into());
    } else {
        attributes.insert("bus".into(), "CAN 1 Mbit/s".into());
        attributes.insert("can_id".into(), format!("{}", phase_id(device)));
    }
    attributes
}

fn phase_id(device: &MockDevice) -> u32 {
    device.key.serial.0.bytes().map(u32::from).sum::<u32>() % 60 + 1
}

fn concurrency(device: &MockDevice) -> Concurrency {
    match (&device.mode, &device.parent) {
        (DeviceMode::Recovery, _) => Concurrency::Exclusive("usb-boot".into()),
        (DeviceMode::Normal, Some(gateway)) => Concurrency::Exclusive(format!("gateway:{gateway}")),
        (DeviceMode::Normal, None) => Concurrency::Parallel,
    }
}

#[async_trait]
impl Driver for MockDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        if link.kind != LinkKind::Simulated {
            return Ok(Vec::new());
        }
        Ok(self
            .fleet
            .lock()
            .devices
            .values()
            .filter(|device| {
                device.online
                    && device.parent.is_none()
                    && device.key.family == self.manifest.family
            })
            .map(|device| Candidate {
                link: link.id.clone(),
                family: device.key.family.clone(),
                address: device.key.serial.0.clone(),
            })
            .collect())
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        let key = DeviceKey::new(candidate.family.as_str(), candidate.address.as_str());
        let device = self.device(&key)?;
        Ok(Identity {
            model: device.model.clone(),
            mode: device.mode,
            versions: BTreeMap::from([(version_name(&device).to_string(), device.version.clone())]),
            name: device.name.clone(),
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: attributes(&device),
            key,
        })
    }

    fn capabilities(&self, device: &Identity) -> Capabilities {
        let running = device.mode == DeviceMode::Normal;
        Capabilities {
            update: Some(Arc::new(MockUpdater {
                fleet: self.fleet.clone(),
            })),
            actions: running.then(|| {
                Arc::new(MockActions {
                    fleet: self.fleet.clone(),
                }) as Arc<dyn ActionsCapability>
            }),
            telemetry: running.then(|| {
                Arc::new(MockTelemetry {
                    fleet: self.fleet.clone(),
                }) as Arc<dyn atlas_driver::TelemetryCapability>
            }),
            logs: running.then(|| {
                Arc::new(MockLogs {
                    fleet: self.fleet.clone(),
                }) as Arc<dyn atlas_driver::LogsCapability>
            }),
            selftest: None,
            status: None,
        }
    }

    async fn children(&self, device: &Identity) -> Result<Vec<Candidate>, DriverError> {
        let link = LinkId(format!("gateway:{}", device.key));
        Ok(self
            .fleet
            .lock()
            .devices
            .values()
            .filter(|child| child.online && child.parent.as_ref() == Some(&device.key))
            .map(|child| Candidate {
                link: link.clone(),
                family: child.key.family.clone(),
                address: child.key.serial.0.clone(),
            })
            .collect())
    }
}

/// Counts an update as running for as long as it is alive.
struct RunningGuard {
    fleet: MockFleet,
    counters: Vec<String>,
}

impl RunningGuard {
    fn start(fleet: &MockFleet, concurrency: &Concurrency) -> Self {
        let mut counters = vec![ALL_UPDATES.to_string()];
        if let Concurrency::Exclusive(resource) = concurrency {
            counters.push(resource.clone());
        }
        {
            let mut state = fleet.lock();
            for counter in &counters {
                let running = state.running.entry(counter.clone()).or_default();
                *running += 1;
                let now = *running;
                let peak = state.peaks.entry(counter.clone()).or_default();
                *peak = (*peak).max(now);
            }
        }
        Self {
            fleet: fleet.clone(),
            counters,
        }
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        let mut state = self.fleet.lock();
        for counter in &self.counters {
            if let Some(running) = state.running.get_mut(counter) {
                *running = running.saturating_sub(1);
            }
        }
    }
}

struct MockUpdater {
    fleet: MockFleet,
}

impl MockUpdater {
    fn set(&self, key: &DeviceKey, change: impl FnOnce(&mut MockDevice)) {
        if let Some(device) = self.fleet.lock().devices.get_mut(key) {
            change(device);
        }
    }
}

#[async_trait]
impl UpdateCapability for MockUpdater {
    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        if release.version.trim().is_empty() {
            return Err(DriverError::Incompatible(
                "the release has no version".into(),
            ));
        }
        let mock = self
            .fleet
            .device(&device.key)
            .ok_or_else(|| DriverError::Unreachable(device.key.to_string()))?;
        let from = device.primary_version().unwrap_or("unknown");
        let summary = match mock.mode {
            DeviceMode::Recovery => format!("Recovery write of {} over USB boot", release.version),
            DeviceMode::Normal => format!("{from} to {}; the device reboots", release.version),
        };
        Ok(UpdatePlan {
            steps: STEPS.to_vec(),
            concurrency: concurrency(&mock),
            summary,
        })
    }

    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let mock = self
            .fleet
            .device(&device.key)
            .filter(|mock| mock.online)
            .ok_or_else(|| DriverError::Unreachable(device.key.to_string()))?;
        let _running = RunningGuard::start(&self.fleet, &concurrency(&mock));
        assert!(
            mock.behavior != MockBehavior::Panics,
            "simulated driver bug"
        );

        for step in STEPS {
            // Apply is the point of no return; cancelling after it is ignored.
            let cancellable = matches!(step, UpdateStep::Preflight | UpdateStep::Transfer);
            if cancellable && cancel.is_cancelled() {
                return Err(DriverError::Cancelled);
            }
            progress.step_started(step);
            let ticks: u32 = if step == UpdateStep::Transfer { 4 } else { 1 };
            for tick in 1..=ticks {
                let pause = tokio::time::sleep(mock.step_time / ticks);
                if cancellable {
                    tokio::select! {
                        () = pause => {}
                        () = cancel.cancelled() => return Err(DriverError::Cancelled),
                    }
                } else {
                    pause.await;
                }
                progress.step_progress(step, tick as f32 / ticks as f32);
            }

            if mock.behavior == MockBehavior::Hangs && step == UpdateStep::Preflight {
                std::future::pending::<()>().await;
            }
            if mock.behavior == MockBehavior::FailsAt(step) {
                return Err(DriverError::StepFailed {
                    step: step.label().into(),
                    message: "simulated failure".into(),
                });
            }
            if step == UpdateStep::Apply && mock.behavior == MockBehavior::BricksAfterApply {
                self.set(&device.key, |mock| mock.mode = DeviceMode::Recovery);
                return Ok(UpdateOutcome::NeedsRecovery {
                    reason: "the device did not boot after writing the image".into(),
                });
            }
            if step == UpdateStep::Confirm && mock.behavior == MockBehavior::NeverConfirms {
                progress.log("no boot confirmation; the device switched back to its previous slot");
                return Ok(UpdateOutcome::RolledBack {
                    reason: "no boot confirmation within the timeout".into(),
                });
            }
        }

        let version = release.version.clone();
        self.set(&device.key, |mock| {
            mock.version = version.clone();
            mock.mode = DeviceMode::Normal;
        });
        progress.log(format!("now running {version}"));
        Ok(UpdateOutcome::Verified { version })
    }
}

struct MockActions {
    fleet: MockFleet,
}

#[async_trait]
impl ActionsCapability for MockActions {
    fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
        vec![
            DeviceAction {
                id: "locate".into(),
                label: "Find it".into(),
                destructive: false,
            },
            DeviceAction {
                id: "reboot".into(),
                label: "Restart".into(),
                destructive: false,
            },
            DeviceAction {
                id: "factory-reset".into(),
                label: "Factory reset".into(),
                destructive: true,
            },
        ]
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        self.fleet
            .device(&device.key)
            .filter(|mock| mock.online)
            .ok_or_else(|| DriverError::Unreachable(device.key.to_string()))?;
        let mut state = self.fleet.lock();
        if action_id == "reboot"
            && let Some(mock) = state.devices.get_mut(&device.key)
        {
            mock.booted_ms = crate::observe::now_ms();
        }
        state
            .actions
            .push((device.key.clone(), action_id.to_string()));
        Ok(())
    }
}
