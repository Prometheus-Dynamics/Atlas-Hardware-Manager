//! The scan: a slow driver delays nothing else, a failing device is retried
//! with capped backoff, and a settled answer is not asked again until the
//! device changes or the user asks. Runs on tokio's paused clock, so no real
//! time passes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// One device of one family. Every call waits `delay` first. The device is
/// listed while `present` is set; `discover_error` and `identify_error` replace
/// the answer. Counters show how often the scan called each.
struct TestDriver {
    manifest: DriverManifest,
    serial: &'static str,
    delay: Duration,
    /// Makes discovery fail as unreachable.
    missing: bool,
    discover_error: Option<DriverError>,
    identify_error: Option<DriverError>,
    present: Arc<AtomicBool>,
    discover_calls: Arc<AtomicU32>,
    identify_calls: Arc<AtomicU32>,
}

impl TestDriver {
    fn new(family: &str, serial: &'static str, delay: Duration, missing: bool) -> Self {
        Self {
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
            discover_error: None,
            identify_error: None,
            present: Arc::new(AtomicBool::new(true)),
            discover_calls: Arc::new(AtomicU32::new(0)),
            identify_calls: Arc::new(AtomicU32::new(0)),
        }
    }

    fn arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

#[async_trait]
impl Driver for TestDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        self.discover_calls.fetch_add(1, Ordering::SeqCst);
        time::sleep(self.delay).await;
        if self.missing {
            return Err(DriverError::Unreachable(
                "nothing answers on this link".into(),
            ));
        }
        if let Some(error) = &self.discover_error {
            return Err(error.clone());
        }
        if !self.present.load(Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        Ok(vec![Candidate {
            link: link.id.clone(),
            family: self.manifest.family.clone(),
            address: format!("{}-addr", self.serial),
        }])
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        self.identify_calls.fetch_add(1, Ordering::SeqCst);
        time::sleep(self.delay).await;
        if let Some(error) = &self.identify_error {
            return Err(error.clone());
        }
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

/// Counts the events `wanted` accepts over `within` (virtual time).
async fn count_matching(
    rx: &mut Receiver<Event>,
    within: Duration,
    mut wanted: impl FnMut(&Event) -> bool,
) -> usize {
    let end = Instant::now() + within;
    let mut count = 0;
    while next_matching(
        rx,
        end.saturating_duration_since(Instant::now()),
        &mut wanted,
    )
    .await
    .is_some()
    {
        count += 1;
    }
    count
}

fn identified(family: &'static str) -> impl FnMut(&Event) -> bool {
    move |event| matches!(event, Event::Identified { identity, .. } if identity.key.family.as_str() == family)
}

fn probe_failed(family: &'static str) -> impl FnMut(&Event) -> bool {
    move |event| matches!(event, Event::ProbeFailed { family: f, .. } if f.as_str() == family)
}

fn settled(family: &'static str) -> impl FnMut(&Event) -> bool {
    move |event| matches!(event, Event::Settled { family: f, .. } if f.as_str() == family)
}

#[tokio::test(start_paused = true)]
async fn a_driver_that_sleeps_in_every_call_delays_nothing_else() {
    let mut registry = DriverRegistry::new();
    registry.register(
        TestDriver::new("slow-family", "slow-serial", Duration::from_secs(30), false).arc(),
    );
    registry.register(TestDriver::new("fast-family", "fast-serial", Duration::ZERO, false).arc());
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
    registry.register(TestDriver::new("gone-family", "gone-serial", Duration::ZERO, true).arc());
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
    registry.register(TestDriver::new("held-family", "held-serial", Duration::ZERO, false).arc());
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

#[tokio::test(start_paused = true)]
async fn a_settled_identify_is_asked_once_and_again_only_on_request_or_return() {
    let mut driver = TestDriver::new("refused-family", "refused-serial", Duration::ZERO, false);
    driver.identify_error = Some(DriverError::Incompatible(
        "this board is not supported".into(),
    ));
    let present = Arc::clone(&driver.present);
    let identify_calls = Arc::clone(&driver.identify_calls);
    let mut registry = DriverRegistry::new();
    registry.register(driver.arc());
    let events = Events::new(512);
    let mut rx = events.subscribe();
    let control = start(registry, &events);
    let family = Family::new("refused-family");

    assert!(
        next_matching(&mut rx, Duration::from_secs(1), settled("refused-family"))
            .await
            .is_some()
    );
    assert_eq!(
        count_matching(&mut rx, Duration::from_secs(30), settled("refused-family")).await,
        0
    );
    assert_eq!(
        identify_calls.load(Ordering::SeqCst),
        1,
        "a settled answer was asked again"
    );

    control.ask_again(&family);
    assert!(
        next_matching(&mut rx, Duration::from_secs(3), settled("refused-family"))
            .await
            .is_some()
    );
    assert_eq!(identify_calls.load(Ordering::SeqCst), 2);

    // Disappears, then returns: that is a change, so it is asked again.
    present.store(false, Ordering::SeqCst);
    time::sleep(Duration::from_secs(5)).await;
    present.store(true, Ordering::SeqCst);
    assert!(
        next_matching(&mut rx, Duration::from_secs(5), settled("refused-family"))
            .await
            .is_some(),
        "a returning device was not asked again"
    );
    assert_eq!(identify_calls.load(Ordering::SeqCst), 3);
    control.stop();
}

#[tokio::test(start_paused = true)]
async fn a_settled_discovery_is_reported_once_and_not_retried() {
    let mut driver = TestDriver::new(
        "unsupported-family",
        "unsupported-serial",
        Duration::ZERO,
        false,
    );
    driver.discover_error = Some(DriverError::Unsupported(
        "this link cannot carry the family".into(),
    ));
    let discover_calls = Arc::clone(&driver.discover_calls);
    let mut registry = DriverRegistry::new();
    registry.register(driver.arc());
    let events = Events::new(512);
    let mut rx = events.subscribe();
    let control = start(registry, &events);

    assert!(
        next_matching(
            &mut rx,
            Duration::from_secs(1),
            settled("unsupported-family")
        )
        .await
        .is_some()
    );
    assert_eq!(
        count_matching(
            &mut rx,
            Duration::from_secs(60),
            probe_failed("unsupported-family")
        )
        .await,
        0,
        "a settled answer went to the backoff schedule"
    );
    assert_eq!(discover_calls.load(Ordering::SeqCst), 1);
    control.stop();
}
