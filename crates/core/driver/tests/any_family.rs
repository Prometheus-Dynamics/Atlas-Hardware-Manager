//! A driver that serves any family, beside drivers that name one. The registry gives each
//! device the driver the rule in `DriverRegistry::resolve` picks.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::{Driver, DriverError, DriverManifest, DriverRegistry, RegisterError, Serves};
use atlas_types::{Candidate, CapabilitySet, Family, Identity, Link, LinkKind};

/// A driver with a manifest and nothing to do: the registry only reads what it declares.
struct Fake {
    manifest: DriverManifest,
    any: bool,
}

impl Fake {
    fn named(family: &str, name: &str, kinds: Vec<LinkKind>, priority: i32) -> Arc<Self> {
        Arc::new(Self {
            manifest: DriverManifest {
                family: Family::new(family),
                name: name.into(),
                version: "1".into(),
                link_kinds: kinds,
                priority,
            },
            any: false,
        })
    }

    fn any(label: &str, name: &str, kinds: Vec<LinkKind>) -> Arc<Self> {
        Arc::new(Self {
            manifest: DriverManifest {
                family: Family::new(label),
                name: name.into(),
                version: "1".into(),
                link_kinds: kinds,
                priority: 0,
            },
            any: true,
        })
    }
}

#[async_trait]
impl Driver for Fake {
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

    async fn discover(&self, _link: &Link) -> Result<Vec<Candidate>, DriverError> {
        Ok(Vec::new())
    }

    async fn identify(&self, _candidate: &Candidate) -> Result<Identity, DriverError> {
        Err(DriverError::Unsupported("not in this test".into()))
    }

    fn capabilities(&self, _device: &Identity) -> CapabilitySet {
        CapabilitySet::new()
    }
}

fn usb() -> LinkKind {
    LinkKind::UsbBoot
}

fn ethernet() -> LinkKind {
    LinkKind::Ethernet
}

fn names(drivers: &[Arc<dyn Driver>]) -> Vec<String> {
    let mut names: Vec<String> = drivers
        .iter()
        .map(|driver| driver.manifest().name.clone())
        .collect();
    names.sort();
    names
}

/// The board-style driver registers beside two family drivers. Asked about a link kind the
/// family drivers do not claim, the registry offers it, and only it.
#[test]
fn the_any_family_driver_is_offered_a_link_the_family_drivers_do_not_claim() {
    let mut registry = DriverRegistry::new();
    registry.register(Fake::named("rpi-like", "Family A", vec![usb()], 0));
    registry.register(Fake::named("rock-like", "Family B", vec![usb()], 0));
    registry.register(Fake::any("board-label", "Board", vec![ethernet()]));

    assert_eq!(
        names(&registry.for_link(&ethernet())),
        vec!["Board".to_string()]
    );
    let resolved = registry
        .resolve(&Family::new("anything"), &ethernet())
        .expect("the any-family driver serves a family on ethernet");
    assert_eq!(resolved.manifest().name, "Board");
    assert!(
        registry.get(&Family::new("board-label")).is_none(),
        "an any-family driver is not a named family's driver"
    );
    assert_eq!(registry.all().len(), 3);
}

/// Where a named driver and the any-family driver could both claim a device on a link kind,
/// the named one serves its family. Otherwise the any-family driver takes the device.
#[test]
fn a_named_driver_wins_for_its_family_and_the_any_family_driver_takes_the_rest() {
    let mut registry = DriverRegistry::new();
    registry.register(Fake::named("rpi-like", "Named", vec![usb()], 0));
    registry.register(Fake::any("board-label", "Board", vec![usb(), ethernet()]));

    let named = registry
        .resolve(&Family::new("rpi-like"), &usb())
        .expect("a driver for the device");
    assert_eq!(named.manifest().name, "Named", "named wins for its family");

    let other = registry
        .resolve(&Family::new("other-family"), &usb())
        .expect("a driver for the device");
    assert_eq!(other.manifest().name, "Board", "any-family takes the rest");

    // The named driver does not handle ethernet, so the any-family driver serves its family there.
    let there = registry
        .resolve(&Family::new("rpi-like"), &ethernet())
        .expect("a driver on ethernet");
    assert_eq!(there.manifest().name, "Board");

    // No driver at all on a link kind nobody handles.
    assert!(
        registry
            .resolve(&Family::new("rpi-like"), &LinkKind::Simulated)
            .is_none()
    );
}

#[test]
fn two_any_family_drivers_on_one_link_kind_are_refused_and_the_first_stays() {
    let mut registry = DriverRegistry::new();
    registry
        .try_register(Fake::any("label-a", "First", vec![ethernet()]))
        .expect("the first is registered");
    let refused = registry
        .try_register(Fake::any("label-b", "Second", vec![usb(), ethernet()]))
        .expect_err("a second on ethernet is refused");
    assert!(matches!(
        refused,
        RegisterError::AnyFamilyTaken { ref holder, .. } if holder == "First"
    ));
    assert_eq!(names(&registry.all()), vec!["First".to_string()]);

    // A different link kind is fine: the second one is accepted when it does not overlap.
    registry
        .try_register(Fake::any("label-c", "Third", vec![usb()]))
        .expect("usb is free");
    assert_eq!(names(&registry.all()).len(), 2);
}

#[test]
fn an_any_family_label_cannot_be_a_named_family_and_the_reverse_is_refused_too() {
    let mut registry = DriverRegistry::new();
    registry.register(Fake::named("rpi-like", "Named", vec![usb()], 0));
    let clash = registry
        .try_register(Fake::any("rpi-like", "Board", vec![ethernet()]))
        .expect_err("the label is a family");
    assert!(matches!(clash, RegisterError::LabelIsFamily { .. }));

    let mut registry = DriverRegistry::new();
    registry
        .try_register(Fake::any("board-label", "Board", vec![ethernet()]))
        .expect("registered");
    let clash = registry
        .try_register(Fake::named("board-label", "Named", vec![usb()], 0))
        .expect_err("the family is a label");
    assert!(matches!(clash, RegisterError::FamilyIsLabel { .. }));
}

#[test]
fn a_named_family_keeps_the_higher_priority_as_before() {
    let mut registry = DriverRegistry::new();
    registry.register(Fake::named("rpi-like", "Low", vec![usb()], 0));
    registry.register(Fake::named("rpi-like", "High", vec![usb()], 5));
    registry.register(Fake::named("rpi-like", "Lower", vec![usb()], 1));
    let kept = registry.get(&Family::new("rpi-like")).expect("one driver");
    assert_eq!(kept.manifest().name, "High");
}
