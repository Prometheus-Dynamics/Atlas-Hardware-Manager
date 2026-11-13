use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use atlas_types::{Family, LinkKind};

use crate::Driver;

/// The drivers in this build, one per family.
#[derive(Clone, Default)]
pub struct DriverRegistry {
    drivers: BTreeMap<Family, Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a driver. When two drivers claim the same family, the one with
    /// the higher manifest priority stays; on a tie the first one stays.
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

    pub fn all(&self) -> Vec<Arc<dyn Driver>> {
        self.drivers.values().cloned().collect()
    }

    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.drivers.keys()
    }

    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }
}

impl fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.drivers.keys()).finish()
    }
}
