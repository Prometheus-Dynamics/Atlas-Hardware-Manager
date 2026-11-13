//! The scan: a slow driver delays nothing else, and a failing device is retried
//! with capped backoff. Runs on tokio's paused clock, so no real time passes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, DriverRegistry};
use atlas_jobs::{Event, Events, LinkSource, ScanConfig, ScanControl, start_scan};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkId, LinkKind,
};
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio::time::{self, Instant};

/// A driver whose every call waits `delay` first. `missing` makes discovery fail.
struct TestDriver {
    manifest: DriverManifest,
    serial: &'static str,
    delay: Duration,
    missing: bool,
}

impl TestDriver {
    fn new(family: &str, serial: &'static str, delay: Duration, missing: bool) -> Arc<Self> {
        Arc::new(Self {
            manifest: DriverManifest {
                family: Family::new(family),
                name: format!("{family} test driver"),
                version: "1".into(),
                link_kinds: vec![LinkKind::UsbNetwork],
                priority: 0,
            },
            serial,
            delay,
            missing,
        })
    }
}

#[async_trait]
impl Driver for TestDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        time::sleep(self.delay).await;
        if self.missing {
            return Err(DriverError::Unreachable(
                "nothing answers on this link".into(),
            ));
        }
        Ok(vec![Candidate {
            link: link.id.clone(),
            family: self.manifest.family.clone(),
            address: format!("{}-addr", self.serial),
        }])
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        time::sleep(self.delay).await;
        Ok(Identity {
            key: DeviceKey::new(self.manifest.family.as_str(), self.serial),
            model: "test-model".into(),
            state: DeviceState::Running,
            versions: BTreeMap::new(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: BTreeMap::new(),
        })
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds([CapabilityKind::Info])
    }
}

struct FixedLinks(Vec<Link>);

impl LinkSource for FixedLinks {
    fn links(&self) -> Vec<Link> {
        self.0.clone()
    }
}

fn usb_link() -> Link {
    Link {
        id: LinkId::new("usbnet:usb0"),
        kind: LinkKind::UsbNetwork,
        label: "USB network (usb0)".into(),
    }
}

fn start(registry: DriverRegistry, events: &Events) -> ScanControl {
    start_scan(
        registry,
        Arc::new(FixedLinks(vec![usb_link()])),
        events.clone(),
        ScanConfig::default(),
    )
}

/// Waits up to `within` (virtual time) for the first event `wanted` accepts.
async fn next_matching(
    rx: &mut Receiver<Event>,
    within: Duration,
    mut wanted: impl FnMut(&Event) -> bool,
) -> Option<Event> {
    time::timeout(within, async {
        loop {
            match rx.recv().await {
                Ok(event) if wanted(&event) => return Some(event),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

fn identified(family: &'static str) -> impl FnMut(&Event) -> bool {
    move |event| matches!(event, Event::Identified { identity, .. } if identity.key.family.as_str() == family)
}

fn probe_failed(family: &'static str) -> impl FnMut(&Event) -> bool {
    move |event| matches!(event, Event::ProbeFailed { family: f, .. } if f.as_str() == family)
}

#[tokio::test(start_paused = true)]
async fn a_driver_that_sleeps_in_every_call_delays_nothing_else() {
    let mut registry = DriverRegistry::new();
    registry.register(TestDriver::new(
        "slow-family",
        "slow-serial",
        Duration::from_secs(30),
        false,
    ));
    registry.register(TestDriver::new(
        "fast-family",
        "fast-serial",
        Duration::ZERO,
        false,
    ));
    let events = Events::new(512);
    let mut rx = events.subscribe();
    let control = start(registry, &events);

    // The fast device is identified within a second, and again on the next round.
    assert!(
        next_matching(&mut rx, Duration::from_secs(1), identified("fast-family"))
            .await
            .is_some(),
        "the fast device was held up by the slow driver"
    );
    assert!(
        next_matching(&mut rx, Duration::from_secs(5), identified("fast-family"))
            .await
            .is_some(),
        "the fast device was not scanned again"
    );

    // The slow driver's first discovery is cut off at its timeout, not waited on.
    let slow = next_matching(
        &mut rx,
        Duration::from_secs(15),
        probe_failed("slow-family"),
    )
    .await
    .expect("the slow discovery was never cut off");
    assert!(matches!(
        slow,
        Event::ProbeFailed {
            timed_out: true,
            ..
        }
    ));
    control.stop();
}

#[tokio::test(start_paused = true)]
async fn a_missing_device_is_retried_with_capped_backoff_and_never_given_up() {
    let mut registry = DriverRegistry::new();
    registry.register(TestDriver::new(
        "gone-family",
        "gone-serial",
        Duration::ZERO,
        true,
    ));
    let events = Events::new(512);
    let mut rx = events.subscribe();
    let control = start(registry, &events);

    let end = Instant::now() + Duration::from_secs(60);
    let mut failures = Vec::new();
    while let Some(event) = next_matching(
        &mut rx,
        end.saturating_duration_since(Instant::now()),
        probe_failed("gone-family"),
    )
    .await
    {
        failures.push(event);
    }
    control.stop();

    // Backoff 250 ms doubling to a 5 s cap: a minute still holds a dozen retries.
    assert!(
        failures.len() >= 12,
        "only {} retries in a minute",
        failures.len()
    );
    let attempts: Vec<u32> = failures
        .iter()
        .filter_map(|e| match e {
            Event::ProbeFailed { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    let expected: Vec<u32> = (1..=attempts.len() as u32).collect();
    assert_eq!(attempts, expected);
    let delays: Vec<u64> = failures
        .iter()
        .filter_map(|e| match e {
            Event::ProbeFailed { retry_in_ms, .. } => Some(*retry_in_ms),
            _ => None,
        })
        .collect();
    assert_eq!(&delays[..3], &[250, 500, 1000]);
    assert!(
        delays.iter().all(|ms| *ms <= 5000),
        "backoff passed the cap: {delays:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_paused_family_starts_no_calls_until_resumed() {
    let mut registry = DriverRegistry::new();
    registry.register(TestDriver::new(
        "held-family",
        "held-serial",
        Duration::ZERO,
        false,
    ));
    let events = Events::new(512);
    let mut rx = events.subscribe();
    let control = start(registry, &events);
    let family = Family::new("held-family");

    assert!(
        next_matching(&mut rx, Duration::from_secs(1), identified("held-family"))
            .await
            .is_some()
    );
    control.pause_family(&family);
    assert!(control.is_paused(&family));
    assert!(
        next_matching(&mut rx, Duration::from_secs(10), identified("held-family"))
            .await
            .is_none(),
        "a paused family was still scanned"
    );

    control.resume_family(&family);
    assert!(
        next_matching(&mut rx, Duration::from_secs(3), identified("held-family"))
            .await
            .is_some(),
        "the family did not restart after resume"
    );
    control.stop();
}
