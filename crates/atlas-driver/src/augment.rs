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

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use async_trait::async_trait;

use crate::{
    ActionsCapability, CancellationToken, Capabilities, DeviceAction, DeviceEvent, DeviceStatus,
    DriverError, EventPage, EventQuery, Identity, LogLine, LogsCapability, Metric, ProgressSink,
    PushSink, ReleaseRef, SlotCommand, SlotInstall, SlotSet, SlotsCapability, StatusCapability,
    TelemetryCapability, UpdateCapability, UpdateOutcome, UpdatePlan, UpdateStep,
};

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
                FallbackUpdate {
                    first: more,
                    then: own,
                }
            } else {
                FallbackUpdate {
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
            (Some(own), Some(more)) => Some(Arc::new(CombinedStatus {
                own,
                more,
                data_first,
            })),
            (own, more) => own.or(more),
        };
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
}

/// Two ways to update one device: `first`, and `then` when `first` can't
/// start (it can't reach the device, the device won't let it in, or its
/// checks fail). Only before anything changed, so nothing is written twice:
/// an Orion update falls back to SSH, or SSH to Orion, never midway.
struct FallbackUpdate {
    first: Arc<dyn UpdateCapability>,
    then: Arc<dyn UpdateCapability>,
}

/// Whether an update failed before it changed anything.
fn failed_to_start(error: &DriverError) -> bool {
    match error {
        DriverError::Unreachable(_)
        | DriverError::Incompatible(_)
        | DriverError::Unsupported(_) => true,
        DriverError::StepFailed { step, .. } => step == UpdateStep::Preflight.label(),
        DriverError::Cancelled | DriverError::Other(_) => false,
    }
}

#[async_trait]
impl UpdateCapability for FallbackUpdate {
    fn preferred(&self) -> bool {
        self.first.preferred()
    }

    fn plan(&self, device: &Identity, release: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
        self.first
            .plan(device, release)
            .or_else(|error| self.then.plan(device, release).map_err(|_| error))
    }

    async fn run(
        &self,
        device: &Identity,
        release: &ReleaseRef,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<UpdateOutcome, DriverError> {
        let first = match self.first.plan(device, release) {
            Ok(_) => self.first.run(device, release, progress, cancel).await,
            Err(error) => Err(error),
        };
        match first {
            Err(error) if failed_to_start(&error) && !cancel.is_cancelled() => {
                if self.then.plan(device, release).is_err() {
                    return Err(error);
                }
                progress.log(format!("{error}; trying the other way in"));
                self.then
                    .run(device, release, progress, cancel)
                    .await
                    .map_err(|second| match second {
                        // Both failed to start: say why for each.
                        DriverError::Incompatible(why) | DriverError::Unreachable(why)
                            if failed_to_start(&error) =>
                        {
                            DriverError::Incompatible(format!("{error}; and {why}"))
                        }
                        other => other,
                    })
            }
            result => result,
        }
    }
}

/// Runs two futures to completion, interleaved on the current task.
async fn both<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut ra, mut rb) = (None, None);
    poll_fn(|cx| {
        if ra.is_none()
            && let Poll::Ready(value) = a.as_mut().poll(cx)
        {
            ra = Some(value);
        }
        if rb.is_none()
            && let Poll::Ready(value) = b.as_mut().poll(cx)
        {
            rb = Some(value);
        }
        match (ra.take(), rb.take()) {
            (Some(x), Some(y)) => Poll::Ready((x, y)),
            (x, y) => {
                ra = x;
                rb = y;
                Poll::Pending
            }
        }
    })
    .await
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

/// What `status` lacks, from `other`.
fn fill_missing(status: &mut DeviceStatus, other: DeviceStatus) {
    status.time = status.time.or(other.time);
    // Field by field: Orion's host facts lack the board's boot record
    // (count, how the boot before ended).
    status.boot = match (status.boot.take(), other.boot) {
        (Some(mut boot), Some(more)) => {
            boot.fill_from(more);
            Some(boot)
        }
        (boot, more) => boot.or(more),
    };
    status.failed_units = status.failed_units.take().or(other.failed_units);
    if status.temperatures.is_empty() {
        status.temperatures = other.temperatures;
    }
    status.fan = status.fan.take().or(other.fan);
    status.update = status.update.take().or(other.update);
    status.clock_offset_s = status.clock_offset_s.or(other.clock_offset_s);
    status.ntp_synchronized = status.ntp_synchronized.or(other.ntp_synchronized);
    status.drift = status.drift.take().or(other.drift);
    status.hardware = status.hardware.take().or(other.hardware);
}

/// The status of both sources: the extra source's hardware readings when it
/// has any; with `data_first`, the extra source's status filled from the
/// owner's, else the owner's. Events come from the first, or from the other
/// when it can't answer.
struct CombinedStatus {
    own: Arc<dyn StatusCapability>,
    more: Arc<dyn StatusCapability>,
    data_first: bool,
}

impl CombinedStatus {
    /// The source asked first, and the other, for events.
    fn order(&self) -> (&Arc<dyn StatusCapability>, &Arc<dyn StatusCapability>) {
        if self.data_first {
            (&self.more, &self.own)
        } else {
            (&self.own, &self.more)
        }
    }
}

#[async_trait]
impl StatusCapability for CombinedStatus {
    async fn status(&self, device: &Identity) -> Result<DeviceStatus, DriverError> {
        match both(self.own.status(device), self.more.status(device)).await {
            (Ok(mut status), Ok(mut extra)) => {
                if self.data_first {
                    fill_missing(&mut extra, status);
                    return Ok(extra);
                }
                if extra.hardware.is_some() {
                    status.hardware = extra.hardware;
                }
                Ok(status)
            }
            (Ok(status), Err(_)) => Ok(status),
            (Err(_), Ok(extra)) => Ok(extra),
            (Err(err), Err(_)) => Err(err),
        }
    }

    async fn events(
        &self,
        device: &Identity,
        since: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DeviceEvent>, DriverError> {
        let (first, then) = self.order();
        match first.events(device, since, limit).await {
            Ok(events) => Ok(events),
            Err(err) => then.events(device, since, limit).await.map_err(|_| err),
        }
    }

    /// The owner's live stream, else the extra source's.
    async fn stream_hardware(
        &self,
        device: &Identity,
        wanted: &[(String, u32)],
        sink: crate::FrameSink,
    ) -> Result<bool, DriverError> {
        match self.own.stream_hardware(device, wanted, sink.clone()).await {
            Ok(false) => self.more.stream_hardware(device, wanted, sink).await,
            other => other,
        }
    }

    /// The owner's push channel, else the extra source's.
    async fn watch(&self, device: &Identity, sink: PushSink) -> Result<bool, DriverError> {
        match self.own.watch(device, sink.clone()).await {
            Ok(false) => self.more.watch(device, sink).await,
            other => other,
        }
    }

    async fn event_page(
        &self,
        device: &Identity,
        query: EventQuery,
    ) -> Result<EventPage, DriverError> {
        let (first, then) = self.order();
        match first.event_page(device, query).await {
            Ok(page) => Ok(page),
            Err(err) => then.event_page(device, query).await.map_err(|_| err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{DeviceKey, DeviceMode, EventSource, HardwareDevice, HardwareSnapshot, LinkId};

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

    /// A status source that answers with what it was given, or fails when
    /// it was given nothing.
    #[derive(Default)]
    struct Reports {
        status: Option<DeviceStatus>,
        events: Option<Vec<DeviceEvent>>,
        asked: Mutex<u32>,
    }

    fn down() -> DriverError {
        DriverError::Unreachable("no answer".into())
    }

    #[async_trait]
    impl StatusCapability for Reports {
        async fn status(&self, _device: &Identity) -> Result<DeviceStatus, DriverError> {
            *self.asked.lock().unwrap() += 1;
            self.status.clone().ok_or_else(down)
        }

        async fn events(
            &self,
            _device: &Identity,
            _since: Option<i64>,
            _limit: usize,
        ) -> Result<Vec<DeviceEvent>, DriverError> {
            self.events.clone().ok_or_else(down)
        }
    }

    fn at(time: i64) -> DeviceStatus {
        DeviceStatus {
            time: Some(time),
            ..DeviceStatus::default()
        }
    }

    fn snapshot(id: &str) -> HardwareSnapshot {
        HardwareSnapshot {
            at: 1,
            devices: vec![HardwareDevice {
                id: id.into(),
                status: "available".into(),
                ..HardwareDevice::default()
            }],
        }
    }

    fn event(t: i64) -> DeviceEvent {
        DeviceEvent {
            t,
            boot_id: String::new(),
            kind: "boot".into(),
            source: EventSource::Local,
            message: String::new(),
            seq: None,
            uptime_s: None,
            at_ms: None,
            data: Default::default(),
        }
    }

    fn merged(own: Arc<Reports>, more: Arc<Reports>) -> Arc<dyn StatusCapability> {
        let mut caps = Capabilities {
            status: Some(own),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            status: Some(more),
            ..Capabilities::default()
        });
        caps.status.unwrap()
    }

    #[tokio::test]
    async fn status_takes_the_hardware_of_the_extra_source() {
        let own = Arc::new(Reports {
            status: Some(at(7)),
            ..Reports::default()
        });
        let more = Arc::new(Reports {
            status: Some(DeviceStatus {
                hardware: Some(snapshot("imu")),
                ..at(9)
            }),
            ..Reports::default()
        });
        let status = merged(own.clone(), more.clone())
            .status(&device())
            .await
            .unwrap();
        // The owner's status, with the extra source's hardware.
        assert_eq!(status.time, Some(7));
        assert_eq!(status.hardware, Some(snapshot("imu")));
        assert_eq!(
            (*own.asked.lock().unwrap(), *more.asked.lock().unwrap()),
            (1, 1)
        );

        // Without hardware from the extra source, the owner's own stays.
        let own = Arc::new(Reports {
            status: Some(DeviceStatus {
                hardware: Some(snapshot("fan")),
                ..at(7)
            }),
            ..Reports::default()
        });
        let more = Arc::new(Reports {
            status: Some(at(9)),
            ..Reports::default()
        });
        let status = merged(own, more).status(&device()).await.unwrap();
        assert_eq!(status.hardware, Some(snapshot("fan")));
    }

    /// Orion first (data_first): its status, filled from the board's; its
    /// events, the board's when it can't answer.
    #[tokio::test]
    async fn a_data_first_source_is_read_first_and_filled_from_the_owner() {
        let own = Arc::new(Reports {
            status: Some(DeviceStatus {
                fan: Some(crate::FanState::default()),
                drift: None,
                ..at(7)
            }),
            events: Some(vec![event(1)]),
            ..Reports::default()
        });
        let more = Arc::new(Reports {
            status: Some(DeviceStatus {
                hardware: Some(snapshot("imu")),
                ..at(9)
            }),
            events: None,
            ..Reports::default()
        });
        let mut caps = Capabilities {
            status: Some(own),
            ..Capabilities::default()
        };
        caps.fill_from_source(
            Capabilities {
                status: Some(more),
                ..Capabilities::default()
            },
            true,
        );
        let merged = caps.status.unwrap();
        let status = merged.status(&device()).await.unwrap();
        assert_eq!(status.time, Some(9), "Orion's own");
        assert!(status.fan.is_some(), "what Orion lacks, from the board");
        assert_eq!(status.hardware, Some(snapshot("imu")));
        // Orion has no events here: the board's.
        assert_eq!(
            merged.events(&device(), None, 10).await.unwrap(),
            vec![event(1)]
        );
    }

    #[tokio::test]
    async fn status_survives_a_failing_source() {
        // The extra source fails: the owner's status, unchanged.
        let owner = at(7);
        let own = Arc::new(Reports {
            status: Some(owner.clone()),
            ..Reports::default()
        });
        let status = merged(own, Arc::new(Reports::default()))
            .status(&device())
            .await
            .unwrap();
        assert_eq!(status, owner);

        // The owner fails: the extra source's status.
        let own = Arc::new(Reports::default());
        let more = Arc::new(Reports {
            status: Some(at(9)),
            ..Reports::default()
        });
        let status = merged(own, more).status(&device()).await.unwrap();
        assert_eq!(status.time, Some(9));

        // Both fail.
        let failing = merged(Arc::new(Reports::default()), Arc::new(Reports::default()));
        assert!(failing.status(&device()).await.is_err());
    }

    #[tokio::test]
    async fn events_come_from_the_owner_and_fall_back_to_the_extra_source() {
        let own = Arc::new(Reports {
            events: Some(vec![event(1)]),
            ..Reports::default()
        });
        let more = Arc::new(Reports {
            events: Some(vec![event(2)]),
            ..Reports::default()
        });
        let caps = merged(own, more.clone());
        assert_eq!(
            caps.events(&device(), None, 10).await.unwrap(),
            vec![event(1)]
        );

        let caps = merged(Arc::new(Reports::default()), more);
        assert_eq!(
            caps.events(&device(), None, 10).await.unwrap(),
            vec![event(2)]
        );

        let caps = merged(Arc::new(Reports::default()), Arc::new(Reports::default()));
        assert!(caps.events(&device(), None, 10).await.is_err());
    }

    /// An update that ends as `outcome`, counting its runs.
    struct Updates {
        preferred: bool,
        outcome: fn() -> Result<UpdateOutcome, DriverError>,
        runs: Mutex<usize>,
    }

    #[async_trait]
    impl UpdateCapability for Updates {
        fn preferred(&self) -> bool {
            self.preferred
        }
        fn plan(&self, _: &Identity, _: &ReleaseRef) -> Result<UpdatePlan, DriverError> {
            Ok(UpdatePlan {
                steps: vec![UpdateStep::Apply],
                concurrency: crate::Concurrency::Parallel,
                summary: String::new(),
            })
        }
        async fn run(
            &self,
            _: &Identity,
            _: &ReleaseRef,
            _: &ProgressSink,
            _: &CancellationToken,
        ) -> Result<UpdateOutcome, DriverError> {
            *self.runs.lock().unwrap() += 1;
            (self.outcome)()
        }
    }

    fn updates(
        preferred: bool,
        outcome: fn() -> Result<UpdateOutcome, DriverError>,
    ) -> Arc<Updates> {
        Arc::new(Updates {
            preferred,
            outcome,
            runs: Mutex::new(0),
        })
    }

    async fn update_with(
        own: &Arc<Updates>,
        more: &Arc<Updates>,
    ) -> Result<UpdateOutcome, DriverError> {
        let mut caps = Capabilities {
            update: Some(own.clone()),
            ..Capabilities::default()
        };
        caps.fill_from(Capabilities {
            update: Some(more.clone()),
            ..Capabilities::default()
        });
        let release = ReleaseRef {
            family: crate::Family::new("raze"),
            version: "2".into(),
            artifact: None,
        };
        caps.update
            .unwrap()
            .run(
                &device(),
                &release,
                &ProgressSink::discard(),
                &CancellationToken::new(),
            )
            .await
    }

    fn verified() -> Result<UpdateOutcome, DriverError> {
        Ok(UpdateOutcome::Verified {
            version: "2".into(),
        })
    }

    #[tokio::test]
    async fn orion_updates_first_and_ssh_is_not_touched() {
        let ssh = updates(false, || {
            Err(DriverError::Incompatible(
                "the board doesn't accept your SSH key".into(),
            ))
        });
        let orion = updates(true, verified);
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (0, 1)
        );
    }

    #[tokio::test]
    async fn a_rejected_ssh_key_falls_back_to_orion() {
        let ssh = updates(false, || {
            Err(DriverError::Incompatible(
                "the board doesn't accept your SSH key".into(),
            ))
        });
        let orion = updates(false, verified);
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn an_unreachable_orion_falls_back_to_ssh() {
        let ssh = updates(false, verified);
        let orion = updates(true, || Err(DriverError::Unreachable("no node".into())));
        assert!(update_with(&ssh, &orion).await.is_ok());
        assert_eq!(
            (*ssh.runs.lock().unwrap(), *orion.runs.lock().unwrap()),
            (1, 1)
        );
    }

    #[tokio::test]
    async fn a_failure_midway_is_never_retried_the_other_way() {
        let ssh = updates(false, verified);
        let orion = updates(true, || {
            Err(DriverError::StepFailed {
                step: "apply".into(),
                message: "x".into(),
            })
        });
        assert!(update_with(&ssh, &orion).await.is_err());
        assert_eq!(*ssh.runs.lock().unwrap(), 0);
    }
}
