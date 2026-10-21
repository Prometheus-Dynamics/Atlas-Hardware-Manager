//! Status from two sources: one device's status, readings and events
//! merged from its owning driver and a management agent.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use async_trait::async_trait;

use crate::{
    DeviceEvent, DeviceStatus, DriverError, EventPage, EventQuery, Identity, PushSink,
    StatusCapability,
};

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
    status.pstore = status.pstore.take().or(other.pstore);
}

/// The status of both sources: the extra source's hardware readings when it
/// has any; with `data_first`, the extra source's status filled from the
/// owner's, else the owner's. Events come from the first, or from the other
/// when it can't answer.
pub(super) struct CombinedStatus {
    pub(super) own: Arc<dyn StatusCapability>,
    pub(super) more: Arc<dyn StatusCapability>,
    pub(super) data_first: bool,
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

    async fn kernel_record(&self, device: &Identity, boot: &str) -> Result<String, DriverError> {
        let (first, then) = self.order();
        match first.kernel_record(device, boot).await {
            Ok(text) => Ok(text),
            Err(err) => then.kernel_record(device, boot).await.map_err(|_| err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{
        Capabilities, DeviceKey, DeviceMode, EventSource, HardwareDevice, HardwareSnapshot, LinkId,
    };

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
}
