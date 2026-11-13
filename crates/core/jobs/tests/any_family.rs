//! The scan with a driver that serves any family, beside a driver that names one. Each device
//! is identified under the family its identity reports; the any-family driver's own label is
//! only the name of its discovery work.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, DriverRegistry, Serves};
use atlas_jobs::{DeviceHolds, Event, Events, LinkSource, ScanConfig, ScanControl, start_scan};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkId, LinkKind,
};
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio::time;

/// One device on a USB network link. `any` makes the driver serve every family; the device then
/// reports `family` in its identity, and the driver's manifest label is `label`.
struct Device {
    manifest: DriverManifest,
    any: bool,
    family: &'static str,
    serial: &'static str,
}

impl Device {
    fn named(family: &'static str, serial: &'static str) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(family),
                name: format!("{family} driver"),
                version: "1".into(),
                link_kinds: vec![LinkKind::UsbNetwork],
                priority: 0,
            },
            any: false,
            family,
            serial,
        }
    }

    fn any(label: &'static str, family: &'static str, serial: &'static str) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new(label),
                name: format!("{label} driver"),
                version: "1".into(),
                link_kinds: vec![LinkKind::UsbNetwork],
                priority: 0,
            },
            any: true,
            family,
            serial,
        }
    }

    fn arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

#[async_trait]
impl Driver for Device {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    fn serves(&self) -> Serves {
        if self.any {
            Serves::Any
        } else {
            Serves::Family(self.manifest.family.clone())
        }
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(vec![Candidate {
            link: link.id.clone(),
            family: self.manifest.family.clone(),
            address: format!("{}-addr", self.serial),
        }])
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        Ok(Identity {
            key: DeviceKey::from_static(self.family, self.serial),
            model: "test-model".into(),
            state: DeviceState::Running,
            lemnos: atlas_types::LemnosState::Absent,
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

#[tokio::test(start_paused = true)]
async fn an_any_family_driver_beside_a_named_one_is_identified_by_each_devices_own_family() {
    let mut registry = DriverRegistry::new();
    registry.register(Device::named("fast-family", "fast-serial").arc());
    registry.register(Device::any("board-label", "widget-family", "widget-serial").arc());

    let events = Events::new(512);
    let mut rx = events.subscribe();
    let links: Arc<dyn LinkSource> = Arc::new(FixedLinks(vec![usb_link()]));
    let control: ScanControl = start_scan(
        registry,
        links,
        events.clone(),
        ScanConfig::default(),
        DeviceHolds::new(),
    );

    // Everything the scan reports over a few seconds, in order.
    let mut seen = Vec::new();
    let end = time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = end.saturating_duration_since(time::Instant::now());
        if left.is_zero() {
            break;
        }
        match next_matching(&mut rx, left, |_| true).await {
            Some(event) => seen.push(event),
            None => break,
        }
    }
    control.stop();

    // The named device is identified as its family, as before.
    assert!(
        seen.iter()
            .any(|event| matches!(event, Event::Identified { identity, .. }
            if identity.key == DeviceKey::from_static("fast-family", "fast-serial"))),
        "the named device is identified"
    );
    // The any-family device is identified under the family it reports, not under the driver's
    // label, so the inventory files it with its own family.
    assert!(
        seen.iter()
            .any(|event| matches!(event, Event::Identified { identity, .. }
            if identity.key == DeviceKey::from_static("widget-family", "widget-serial"))),
        "the any-family device is identified by its own family"
    );
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, Event::Identified { identity, .. }
            if identity.key.family().map(Family::as_str) == Some("board-label"))),
        "the driver's label is never a device's family"
    );
    // Its discovery is reported under the driver's label: the scan's name for that work.
    assert!(
        seen.iter().any(|event| matches!(
            event,
            Event::Discovered { family, .. } if family.as_str() == "board-label"
        )),
        "the any-family driver's discovery is scanned"
    );
}

#[tokio::test(start_paused = true)]
async fn a_device_two_drivers_see_is_reported_once_by_the_driver_the_registry_names() {
    // A named driver and an any-family driver both find the same device on the same link.
    let mut registry = DriverRegistry::new();
    registry.register(Device::named("fast-family", "fast-serial").arc());
    registry.register(Device::any("board-label", "fast-family", "fast-serial").arc());

    let events = Events::new(512);
    let mut rx = events.subscribe();
    let links: Arc<dyn LinkSource> = Arc::new(FixedLinks(vec![usb_link()]));
    let control: ScanControl = start_scan(
        registry,
        links,
        events.clone(),
        ScanConfig::default(),
        DeviceHolds::new(),
    );

    let mut identified = 0;
    let end = time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = end.saturating_duration_since(time::Instant::now());
        if left.is_zero() {
            break;
        }
        match next_matching(&mut rx, left, |_| true).await {
            Some(Event::Identified { identity, .. })
                if identity.key == DeviceKey::from_static("fast-family", "fast-serial") =>
            {
                identified += 1;
            }
            Some(_) => {}
            None => break,
        }
    }
    control.stop();

    assert_eq!(
        identified, 1,
        "the device is reported once, not once per driver"
    );
}
