use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{Driver, Family, LinkKind};

/// The set of drivers in this build, one per family.
#[derive(Clone, Default)]
pub struct DriverRegistry {
    drivers: BTreeMap<Family, Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a driver. When two drivers claim the same family, the one with
    /// the higher manifest priority stays.
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> &mut Self {
        let family = driver.manifest().family.clone();
        let keep_existing = self
            .drivers
            .get(&family)
            .is_some_and(|existing| existing.manifest().priority >= driver.manifest().priority);
        if !keep_existing {
            self.drivers.insert(family, driver);
        }
        self
    }

    pub fn get(&self, family: &Family) -> Option<&Arc<dyn Driver>> {
        self.drivers.get(family)
    }

    /// Drivers that can discover devices on this kind of link.
    pub fn for_link(&self, link: &LinkKind) -> Vec<Arc<dyn Driver>> {
        self.drivers
            .values()
            .filter(|driver| driver.manifest().handles(link))
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.drivers.keys()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::{Candidate, Capabilities, DriverError, DriverManifest, Identity, Link};

    struct Stub(DriverManifest);

    #[async_trait]
    impl Driver for Stub {
        fn manifest(&self) -> &DriverManifest {
            &self.0
        }
        async fn discover(&self, _link: &Link) -> Result<Vec<Candidate>, DriverError> {
            Ok(Vec::new())
        }
        async fn identify(&self, _candidate: &Candidate) -> Result<Identity, DriverError> {
            Err(DriverError::Unsupported("stub".into()))
        }
        fn capabilities(&self, _device: &Identity) -> Capabilities {
            Capabilities::default()
        }
    }

    fn stub(name: &str, priority: i32, link_kinds: Vec<LinkKind>) -> Arc<dyn Driver> {
        Arc::new(Stub(DriverManifest {
            family: Family::new("helios"),
            name: name.into(),
            version: "0".into(),
            link_kinds,
            priority,
        }))
    }

    #[test]
    fn higher_priority_driver_wins_its_family() {
        let mut registry = DriverRegistry::new();
        registry.register(stub("low", 0, vec![LinkKind::Ethernet]));
        registry.register(stub("high", 5, vec![LinkKind::Ethernet]));
        registry.register(stub("lower", 1, vec![LinkKind::Ethernet]));

        let family = Family::new("helios");
        assert_eq!(registry.get(&family).unwrap().manifest().name, "high");
    }

    #[test]
    fn drivers_are_matched_by_link_kind() {
        let mut registry = DriverRegistry::new();
        registry.register(stub(
            "net",
            0,
            vec![LinkKind::Ethernet, LinkKind::UsbNetwork],
        ));

        assert_eq!(registry.for_link(&LinkKind::UsbNetwork).len(), 1);
        assert!(registry.for_link(&LinkKind::UsbBoot).is_empty());
    }
}
