//! The driver contract, end to end: a driver is registered, found by its
//! link kind, identified, and answers its capabilities for each device state.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, DriverRegistry, HealthCheck};
use atlas_types::{
    Candidate, CapabilityKind, CapabilitySet, DeviceKey, DeviceState, Family, Identity, Link,
    LinkId, LinkKind,
};

/// Runs a future that completes without waiting on anything.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test future must not wait"),
    }
}

struct SampleDriver {
    manifest: DriverManifest,
}

impl SampleDriver {
    fn new(priority: i32) -> Self {
        Self {
            manifest: DriverManifest {
                family: Family::new("sample-soc"),
                name: "Sample SoC".into(),
                version: "1.0.0".into(),
                link_kinds: vec![LinkKind::UsbBoot, LinkKind::Ethernet],
                priority,
            },
        }
    }
}

#[async_trait]
impl Driver for SampleDriver {
    fn manifest(&self) -> &DriverManifest {
        &self.manifest
    }

    async fn discover(&self, link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(vec![Candidate {
            link: link.id.clone(),
            family: self.manifest.family.clone(),
            address: "10.0.0.2".into(),
        }])
    }

    async fn identify(&self, candidate: &Candidate) -> Result<Identity, DriverError> {
        Ok(Identity {
            key: DeviceKey::new("sample-soc", "0001"),
            model: "sample-board".into(),
            state: DeviceState::Running,
            lemnos: atlas_types::LemnosState::Absent,
            versions: [("os".to_string(), "2026.3.1".to_string())].into(),
            name: None,
            link: candidate.link.clone(),
            address: candidate.address.clone(),
            attributes: Default::default(),
        })
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::from_kinds([
            CapabilityKind::Update,
            CapabilityKind::Logs,
            CapabilityKind::Status,
        ])
    }

    async fn health(&self) -> Vec<HealthCheck> {
        vec![HealthCheck::ok(
            "sample.ready",
            "Sample ready",
            "nothing to fix",
        )]
    }
}

fn ethernet_link() -> Link {
    Link {
        id: LinkId::new("eth0"),
        kind: LinkKind::Ethernet,
        label: "Ethernet (eth0)".into(),
    }
}

#[test]
fn driver_registers_and_is_found_by_link_kind() {
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(SampleDriver::new(0)));

    assert!(!registry.is_empty());
    assert_eq!(registry.for_link(&LinkKind::Ethernet).len(), 1);
    assert!(registry.for_link(&LinkKind::Simulated).is_empty());
    assert!(registry.get(&Family::new("sample-soc")).is_some());
}

#[test]
fn registered_driver_identifies_and_answers_capabilities() {
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(SampleDriver::new(0)));
    let driver = registry
        .get(&Family::new("sample-soc"))
        .expect("driver registered");

    let candidates = block_on(driver.discover(&ethernet_link())).expect("discover");
    assert_eq!(candidates.len(), 1);
    let identity = block_on(driver.identify(&candidates[0])).expect("identify");
    assert_eq!(identity.key.to_string(), "sample-soc:0001");
    assert_eq!(identity.primary_version(), Some("2026.3.1"));

    let granted = driver.capabilities(&identity);
    assert_eq!(
        granted.shown(DeviceState::Running),
        vec![
            CapabilityKind::Info,
            CapabilityKind::Update,
            CapabilityKind::Logs,
            CapabilityKind::Status,
        ]
    );
    assert_eq!(
        granted.shown(DeviceState::BootRom),
        vec![CapabilityKind::Info]
    );

    let health = block_on(driver.health());
    assert_eq!(health[0].id, "sample.ready");
}

#[test]
fn fix_without_a_named_fix_is_unsupported() {
    let driver = SampleDriver::new(0);
    let error = block_on(driver.fix("reset")).expect_err("no fixes offered");
    assert!(matches!(error, DriverError::Unsupported(_)));
}
