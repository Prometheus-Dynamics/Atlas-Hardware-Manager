use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use atlas_types::{Family, LinkKind};

use crate::{Driver, Serves};

/// Why a driver was not registered.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// Another driver already serves every family on a link kind this one handles, and two
    /// any-family drivers on one link kind would not know whose device is whose.
    #[error("{driver} serves any family on {link}, but {holder} already does")]
    AnyFamilyTaken {
        driver: String,
        holder: String,
        link: &'static str,
    },
    /// This driver serves any family under a label that is a named family's.
    #[error("{driver} serves any family under the label {label}, which a named family uses")]
    LabelIsFamily { driver: String, label: Family },
    /// This driver names a family that another any-family driver uses as its label.
    #[error("{driver} names the family {label}, which an any-family driver uses as its label")]
    FamilyIsLabel { driver: String, label: Family },
}

/// The drivers in this build.
///
/// A driver either names one family, or serves any family ([`Serves::Any`]). Named drivers are
/// keyed by family, one per family: when two name the same family, the one with the higher
/// manifest priority stays (on a tie, the first). Any-family drivers are kept beside them, at
/// most one per link kind.
///
/// Which driver serves a device: see [`DriverRegistry::resolve`].
#[derive(Clone, Default)]
pub struct DriverRegistry {
    families: BTreeMap<Family, Arc<dyn Driver>>,
    any: Vec<Arc<dyn Driver>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a driver. A refused driver is dropped: use [`DriverRegistry::try_register`] to learn
    /// why.
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> &mut Self {
        let _ = self.try_register(driver);
        self
    }

    /// Adds a driver, or says why it cannot be added.
    ///
    /// A named driver replaces a named driver of its family only when its priority is higher.
    /// An any-family driver is refused when another any-family driver already handles one of
    /// its link kinds, and when its label is a named family (or a named family's label is its
    /// family).
    ///
    /// # Errors
    ///
    /// The [`RegisterError`] that refuses it.
    pub fn try_register(&mut self, driver: Arc<dyn Driver>) -> Result<(), RegisterError> {
        match driver.serves() {
            Serves::Family(family) => {
                if self.any.iter().any(|held| held.manifest().family == family) {
                    return Err(RegisterError::FamilyIsLabel {
                        driver: driver.manifest().name.clone(),
                        label: family,
                    });
                }
                let keep_existing = self.families.get(&family).is_some_and(|existing| {
                    existing.manifest().priority >= driver.manifest().priority
                });
                if !keep_existing {
                    self.families.insert(family, driver);
                }
                Ok(())
            }
            Serves::Any => {
                let label = driver.manifest().family.clone();
                if self.families.contains_key(&label) {
                    return Err(RegisterError::LabelIsFamily {
                        driver: driver.manifest().name.clone(),
                        label,
                    });
                }
                for kind in &driver.manifest().link_kinds {
                    if let Some(holder) = self.any.iter().find(|held| held.manifest().handles(kind))
                    {
                        return Err(RegisterError::AnyFamilyTaken {
                            driver: driver.manifest().name.clone(),
                            holder: holder.manifest().name.clone(),
                            link: kind.label(),
                        });
                    }
                }
                self.any.push(driver);
                Ok(())
            }
        }
    }

    /// The named driver for `family`, if one is registered. An any-family driver is not returned
    /// here: use [`DriverRegistry::resolve`] for a device.
    pub fn get(&self, family: &Family) -> Option<&Arc<dyn Driver>> {
        self.families.get(family)
    }

    /// The driver that serves a device of `family` found on a link of `kind`.
    ///
    /// The rule: the named driver for `family` if it handles `kind`; otherwise the any-family
    /// driver that handles `kind`, if there is one; otherwise none. So a named driver always wins
    /// for its own family where both could claim the device, and an any-family driver takes what
    /// no named driver claims on that link kind.
    ///
    /// The family is the one the device reports in its identity, so this is asked after identify.
    pub fn resolve(&self, family: &Family, kind: &LinkKind) -> Option<Arc<dyn Driver>> {
        self.families
            .get(family)
            .filter(|driver| driver.manifest().handles(kind))
            .or_else(|| {
                self.any
                    .iter()
                    .find(|driver| driver.manifest().handles(kind))
            })
            .cloned()
    }

    /// Drivers that can discover devices on this kind of link: the named ones first, then the
    /// any-family one, if any.
    pub fn for_link(&self, link: &LinkKind) -> Vec<Arc<dyn Driver>> {
        self.families
            .values()
            .chain(self.any.iter())
            .filter(|driver| driver.manifest().handles(link))
            .cloned()
            .collect()
    }

    /// Every driver: the named ones, then the any-family ones.
    pub fn all(&self) -> Vec<Arc<dyn Driver>> {
        self.families
            .values()
            .chain(self.any.iter())
            .cloned()
            .collect()
    }

    /// The named families, not the any-family drivers' labels.
    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.families.keys()
    }

    pub fn is_empty(&self) -> bool {
        self.families.is_empty() && self.any.is_empty()
    }
}

impl fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("families", &self.families.keys().collect::<Vec<_>>())
            .field(
                "any_family",
                &self
                    .any
                    .iter()
                    .map(|driver| driver.manifest().name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}
