//! Capabilities from more than one source for one device.
//!
//! A device has one owning driver: the one that found and identified it
//! (for a running Raze, the board driver over mDNS). A management agent such as
//! Orion knows the same board by its serial and can add telemetry, actions,
//! or updates, but must not become a second device. A [`CapabilitySource`]
//! does that: Atlas asks every source about each device it identifies and
//! fills in what the owning driver left empty. Without the source, the
//! device keeps exactly what its driver gives it.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use async_trait::async_trait;

use crate::{
    ActionsCapability, Capabilities, DeviceAction, DeviceEvent, DeviceStatus, DriverError,
    EventPage, EventQuery, Identity, PushSink, StatusCapability,
};

/// Extra capabilities for devices other drivers own.
pub trait CapabilitySource: Send + Sync {
    /// What this source adds for `device`; empty when it doesn't know it.
    fn capabilities_for(&self, device: &Identity) -> Capabilities;
}

impl Capabilities {
    /// Fills the capabilities this device lacks from `extra`. Actions are
    /// combined: the owning driver's come first and win on equal ids, unless
    /// `extra`'s are [`preferred`](ActionsCapability::preferred): then those
    /// run, and the driver's only when they can't reach the device.
    pub fn fill_from(&mut self, extra: Capabilities) {
        if self.update.is_none() {
            self.update = extra.update;
        }
        if self.telemetry.is_none() {
            self.telemetry = extra.telemetry;
        }
        if self.logs.is_none() {
            self.logs = extra.logs;
        }
        if self.selftest.is_none() {
            self.selftest = extra.selftest;
        }
        if self.hardware.is_none() {
            self.hardware = extra.hardware;
        }
        self.status = match (self.status.take(), extra.status) {
            (Some(own), Some(more)) => Some(Arc::new(CombinedStatus { own, more })),
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

/// The owner's status, with the hardware readings of the extra source when it
/// has any. Events come from the owner, or from the extra source when the
/// owner can't answer.
struct CombinedStatus {
    own: Arc<dyn StatusCapability>,
    more: Arc<dyn StatusCapability>,
}

#[async_trait]
impl StatusCapability for CombinedStatus {
    async fn status(&self, device: &Identity) -> Result<DeviceStatus, DriverError> {
        match both(self.own.status(device), self.more.status(device)).await {
            (Ok(mut status), Ok(extra)) => {
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
        match self.own.events(device, since, limit).await {
            Ok(events) => Ok(events),
            Err(err) => self
                .more
                .events(device, since, limit)
                .await
                .map_err(|_| err),
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
        match self.own.event_page(device, query).await {
            Ok(page) => Ok(page),
            Err(err) => self.more.event_page(device, query).await.map_err(|_| err),
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
}
