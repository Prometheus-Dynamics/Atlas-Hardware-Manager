//! Capabilities from more than one source for one device.
//!
//! A device has one owning driver: the one that found and identified it
//! (for a running Raze, the board driver over mDNS). A management agent such as
//! Orion knows the same board by its serial and can add telemetry, actions,
//! or updates, but must not become a second device. A [`CapabilitySource`]
//! does that: Atlas asks every source about each device it identifies and
//! fills in what the owning driver left empty. Without the source, the
//! device keeps exactly what its driver gives it.
//!
//! A source can ask to come first for a device's data ([`CapabilitySource::data_first`]:
//! Orion, which reaches a board the same way whatever link Atlas found it
//! on): its status, telemetry, logs and events are then read first, the
//! owner's filling what it lacks and standing in when it fails. The live
//! hardware stream and push channel stay the owner's first: the board's own
//! stream runs at device rate.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    ActionsCapability, Capabilities, DeviceAction, DriverError, Identity, LayerPush, LayerPushed,
    LogLine, LogsCapability, Metric, ProgressSink, SlotCommand, SlotInstall, SlotSet,
    SlotsCapability, TelemetryCapability,
};

mod status;
mod update;

/// Extra capabilities for devices other drivers own.
pub trait CapabilitySource: Send + Sync {
    /// What this source adds for `device`; empty when it doesn't know it.
    fn capabilities_for(&self, device: &Identity) -> Capabilities;

    /// Whether this source's data comes before the owning driver's (see the
    /// module notes).
    fn data_first(&self) -> bool {
        false
    }
}

impl Capabilities {
    /// Fills the capabilities this device lacks from `extra`. Actions are
    /// combined: the owning driver's come first and win on equal ids, unless
    /// `extra`'s are [`preferred`](ActionsCapability::preferred): then those
    /// run, and the driver's only when they can't reach the device.
    pub fn fill_from(&mut self, extra: Capabilities) {
        self.fill_from_source(extra, false);
    }

    /// [`fill_from`](Self::fill_from), with `extra`'s data first when
    /// `data_first` (status, telemetry, logs, events).
    pub fn fill_from_source(&mut self, extra: Capabilities, data_first: bool) {
        self.update = match (self.update.take(), extra.update) {
            (Some(own), Some(more)) => Some(Arc::new(if more.preferred() {
                update::FallbackUpdate {
                    first: more,
                    then: own,
                }
            } else {
                update::FallbackUpdate {
                    first: own,
                    then: more,
                }
            })),
            (own, more) => own.or(more),
        };
        self.telemetry = match (self.telemetry.take(), extra.telemetry) {
            (Some(own), Some(more)) if data_first => Some(Arc::new(FallbackTelemetry {
                first: more,
                then: own,
            })),
            (own, more) => own.or(more),
        };
        self.logs = match (self.logs.take(), extra.logs) {
            (Some(own), Some(more)) if data_first => Some(Arc::new(FallbackLogs {
                first: more,
                then: own,
            })),
            (own, more) => own.or(more),
        };
        if self.selftest.is_none() {
            self.selftest = extra.selftest;
        }
        if self.hardware.is_none() {
            self.hardware = extra.hardware;
        }
        self.slots = match (self.slots.take(), extra.slots) {
            (Some(own), Some(more)) => Some(Arc::new(if more.preferred() {
                FallbackSlots {
                    first: more,
                    then: own,
                }
            } else {
                FallbackSlots {
                    first: own,
                    then: more,
                }
            })),
            (own, more) => own.or(more),
        };
        self.status = match (self.status.take(), extra.status) {
            (Some(own), Some(more)) => Some(Arc::new(status::CombinedStatus {
                own,
                more,
                data_first,
            })),
            (own, more) => own.or(more),
        };
        // Install methods add up: the owner's first, then the source's.
        self.install.extend(extra.install);
        self.actions = match (self.actions.take(), extra.actions) {
            (Some(own), Some(more)) => Some(Arc::new(CombinedActions { own, more })),
            (own, more) => own.or(more),
        };
    }
}

struct CombinedActions {
    own: Arc<dyn ActionsCapability>,
    more: Arc<dyn ActionsCapability>,
}

#[async_trait]
impl ActionsCapability for CombinedActions {
    fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
        let mut list = self.own.actions(device);
        for action in self.more.actions(device) {
            if !list.iter().any(|existing| existing.id == action.id) {
                list.push(action);
            }
        }
        list
    }

    async fn run_action(&self, device: &Identity, action_id: &str) -> Result<(), DriverError> {
        let offers = |source: &Arc<dyn ActionsCapability>| {
            source
                .actions(device)
                .iter()
                .any(|action| action.id == action_id)
        };
        let (own, more) = (offers(&self.own), offers(&self.more));
        if more && (self.more.preferred() || !own) {
            match self.more.run_action(device, action_id).await {
                // Only when it never reached the device, so nothing runs twice.
                Err(DriverError::Unreachable(_)) if own => {
                    self.own.run_action(device, action_id).await
                }
                result => result,
            }
        } else {
            self.own.run_action(device, action_id).await
        }
    }
}

/// Two ways to a board's slots: `first`, and `then` when `first` can't
/// reach the board (or doesn't know its slots), so nothing runs twice.
struct FallbackSlots {
    first: Arc<dyn SlotsCapability>,
    then: Arc<dyn SlotsCapability>,
}

#[async_trait]
impl SlotsCapability for FallbackSlots {
    fn preferred(&self) -> bool {
        self.first.preferred()
    }

    async fn slots(&self, device: &Identity) -> Result<Option<SlotSet>, DriverError> {
        match self.first.slots(device).await {
            Ok(Some(set)) => Ok(Some(set)),
            Ok(None) => self.then.slots(device).await.or(Ok(None)),
            Err(error @ DriverError::Unreachable(_)) => {
                self.then.slots(device).await.map_err(|_| error)
            }
            Err(error) => Err(error),
        }
    }

    async fn command(&self, device: &Identity, command: &SlotCommand) -> Result<(), DriverError> {
        match self.first.command(device, command).await {
            Err(DriverError::Unreachable(_) | DriverError::Unsupported(_)) => {
                self.then.command(device, command).await
            }
            result => result,
        }
    }

    async fn install(
        &self,
        device: &Identity,
        install: &SlotInstall,
        progress: &ProgressSink,
    ) -> Result<(), DriverError> {
        match self.first.install(device, install, progress).await {
            Err(error @ (DriverError::Unreachable(_) | DriverError::Unsupported(_))) => {
                progress.log(format!("{error}; trying the other way in"));
                self.then.install(device, install, progress).await
            }
            result => result,
        }
    }

    async fn push_layer(
        &self,
        device: &Identity,
        push: &LayerPush,
        progress: &ProgressSink,
    ) -> Result<LayerPushed, DriverError> {
        match self.first.push_layer(device, push, progress).await {
            Err(DriverError::Unreachable(_) | DriverError::Unsupported(_)) => {
                self.then.push_layer(device, push, progress).await
            }
            result => result,
        }
    }
}

/// Telemetry from `first`, else `then`.
struct FallbackTelemetry {
    first: Arc<dyn TelemetryCapability>,
    then: Arc<dyn TelemetryCapability>,
}

#[async_trait]
impl TelemetryCapability for FallbackTelemetry {
    async fn read(&self, device: &Identity) -> Result<Vec<Metric>, DriverError> {
        match self.first.read(device).await {
            Ok(metrics) if !metrics.is_empty() => Ok(metrics),
            first => match self.then.read(device).await {
                Ok(metrics) => Ok(metrics),
                Err(err) => first.and(Err(err)),
            },
        }
    }
}

/// Logs from `first`, else `then`.
struct FallbackLogs {
    first: Arc<dyn LogsCapability>,
    then: Arc<dyn LogsCapability>,
}

#[async_trait]
impl LogsCapability for FallbackLogs {
    async fn tail(&self, device: &Identity, lines: usize) -> Result<Vec<LogLine>, DriverError> {
        match self.first.tail(device, lines).await {
            Ok(tail) => Ok(tail),
            Err(err) => self.then.tail(device, lines).await.map_err(|_| err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{DeviceKey, DeviceMode, LinkId};

    struct Named(&'static [&'static str], Mutex<Vec<String>>);

    #[async_trait]
    impl ActionsCapability for Named {
        fn actions(&self, _device: &Identity) -> Vec<DeviceAction> {
            self.0
                .iter()
                .map(|id| DeviceAction {
                    id: (*id).into(),
                    label: (*id).into(),
                    destructive: false,
                })
                .collect()
        }

        async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
            self.1.lock().unwrap().push(action_id.into());
            Ok(())
        }
    }

    fn device() -> Identity {
        Identity {
            key: DeviceKey::new("raze", "a1"),
            model: "Raze".into(),
            mode: DeviceMode::Normal,
            versions: BTreeMap::new(),
            name: None,
            link: LinkId("mdns".into()),
            address: "x".into(),
            attributes: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn actions_combine_and_the_owner_wins() {
        let own = Arc::new(Named(&["locate"], Mutex::default()));
        let more = Arc::new(Named(&["locate", "reboot"], Mutex::default()));
        let mut caps = Capabilities {
            actions: Some(own.clone()),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            actions: Some(more.clone()),
            ..Capabilities::default()
        });
        let combined = caps.actions.unwrap();
        let ids: Vec<String> = combined
            .actions(&device())
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, vec!["locate", "reboot"]);

        combined.run_action(&device(), "locate").await.unwrap();
        combined.run_action(&device(), "reboot").await.unwrap();
        assert_eq!(*own.1.lock().unwrap(), vec!["locate"]);
        assert_eq!(*more.1.lock().unwrap(), vec!["reboot"]);
    }

    /// A preferred source (Orion) whose runs fail with `outcome`.
    struct Agent(
        &'static [&'static str],
        Mutex<Vec<String>>,
        fn() -> DriverError,
    );

    #[async_trait]
    impl ActionsCapability for Agent {
        fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
            Named(self.0, Mutex::default()).actions(device)
        }

        fn preferred(&self) -> bool {
            true
        }

        async fn run_action(&self, _device: &Identity, action_id: &str) -> Result<(), DriverError> {
            self.1.lock().unwrap().push(action_id.into());
            Err((self.2)())
        }
    }

    fn combined(own: Arc<Named>, more: Arc<dyn ActionsCapability>) -> Arc<dyn ActionsCapability> {
        let mut caps = Capabilities {
            actions: Some(own),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            actions: Some(more),
            ..Capabilities::default()
        });
        caps.actions.unwrap()
    }

    #[tokio::test]
    async fn a_preferred_source_runs_shared_actions_and_falls_back_only_when_unreached() {
        // Orion reachable: it runs the shared action, the driver doesn't.
        let own = Arc::new(Named(
            &["locate", "set-clock", "power-off"],
            Mutex::default(),
        ));
        let more = Arc::new(Named(&["locate", "set-clock"], Mutex::default()));
        struct Preferred(Arc<Named>);
        #[async_trait]
        impl ActionsCapability for Preferred {
            fn actions(&self, device: &Identity) -> Vec<DeviceAction> {
                self.0.actions(device)
            }
            fn preferred(&self) -> bool {
                true
            }
            async fn run_action(&self, device: &Identity, id: &str) -> Result<(), DriverError> {
                self.0.run_action(device, id).await
            }
        }
        let actions = combined(own.clone(), Arc::new(Preferred(more.clone())));
        actions.run_action(&device(), "set-clock").await.unwrap();
        actions.run_action(&device(), "power-off").await.unwrap();
        assert_eq!(*more.1.lock().unwrap(), vec!["set-clock"]);
        assert_eq!(*own.1.lock().unwrap(), vec!["power-off"]);

        // Never reached: the driver runs it instead.
        let own = Arc::new(Named(&["locate"], Mutex::default()));
        let down = Arc::new(Agent(&["locate"], Mutex::default(), || {
            DriverError::Unreachable("no Orion".into())
        }));
        let actions = combined(own.clone(), down.clone());
        actions.run_action(&device(), "locate").await.unwrap();
        assert_eq!(*down.1.lock().unwrap(), vec!["locate"]);
        assert_eq!(*own.1.lock().unwrap(), vec!["locate"]);

        // Sent but failed: no second run.
        let own = Arc::new(Named(&["reboot"], Mutex::default()));
        let failed = Arc::new(Agent(&["reboot"], Mutex::default(), || {
            DriverError::Other("lost track of it".into())
        }));
        let actions = combined(own.clone(), failed);
        assert!(actions.run_action(&device(), "reboot").await.is_err());
        assert!(own.1.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_from_the_source_changes_nothing() {
        let mut caps = Capabilities::default();
        caps.fill_from(Capabilities::default());
        assert!(caps.kinds().len() == 1);
    }
}
