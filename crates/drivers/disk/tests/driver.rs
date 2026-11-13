//! The removable-disk driver against a made-up disk listing: what it finds,
//! what it says each one is, and what it grants.

use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use atlas_disk::{Disk, DiskError, DiskSource};
use atlas_driver::{Driver, DriverError};
use atlas_driver_disk::{DiskDriver, FAMILY};
use atlas_types::{CapabilityKind, DeviceState, Link, LinkId, LinkKind};

/// Runs a future that completes without waiting on anything.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test future must not wait"),
    }
}

struct Listing(Vec<Disk>);

impl DiskSource for Listing {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        Ok(self.0.clone())
    }
}

struct Broken;

impl DiskSource for Broken {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        Err(DiskError::List("no sysfs here".into()))
    }
}

fn card(name: &str, serial: Option<&str>) -> Disk {
    Disk {
        path: PathBuf::from(format!("/dev/{name}")),
        name: name.into(),
        size_bytes: 16_000_000_000,
        vendor: Some("Reader".into()),
        model: Some("SD".into()),
        serial: serial.map(str::to_string),
        removable: true,
        usb: true,
        usb_port: Some("1-4.2".into()),
        system: false,
        mount_points: Vec::new(),
    }
}

fn system_disk() -> Disk {
    let mut disk = card("nvme0n1", None);
    disk.removable = false;
    disk.usb = false;
    disk.system = true;
    disk.mount_points = vec!["/".into()];
    disk
}

fn fixed_usb_drive() -> Disk {
    let mut disk = card("sdc", None);
    disk.removable = false;
    disk
}

fn empty_reader() -> Disk {
    let mut disk = card("sde", None);
    disk.size_bytes = 0;
    disk
}

fn link() -> Link {
    Link {
        id: LinkId::new("disks"),
        kind: LinkKind::Simulated,
        label: "Local disks".into(),
    }
}

fn driver(disks: Vec<Disk>) -> DiskDriver {
    DiskDriver::new(Arc::new(Listing(disks)))
}

#[test]
fn only_cards_that_may_be_written_are_candidates() {
    let driver = driver(vec![
        card("sdz", Some("A1")),
        system_disk(),
        fixed_usb_drive(),
        empty_reader(),
    ]);

    let candidates = block_on(driver.discover(&link())).unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].address, "/dev/sdz");
    assert_eq!(candidates[0].family.as_str(), FAMILY);
}

#[test]
fn a_system_disk_never_becomes_a_device() {
    let driver = driver(vec![system_disk()]);
    assert!(block_on(driver.discover(&link())).unwrap().is_empty());
}

#[test]
fn a_card_is_identified_as_a_removable_disk_with_a_stable_key() {
    let driver = driver(vec![card("sdz", Some("A1B2"))]);
    let candidates = block_on(driver.discover(&link())).unwrap();

    let identity = block_on(driver.identify(&candidates[0])).unwrap();

    assert_eq!(identity.key.to_string(), format!("{FAMILY}:A1B2"));
    assert_eq!(identity.state, DeviceState::RemovableDisk);
    assert_eq!(identity.model, "Reader SD");
    assert_eq!(identity.address, "/dev/sdz");
    assert_eq!(
        identity.attributes.get("usb-port").map(String::as_str),
        Some("1-4.2")
    );
}

#[test]
fn a_card_without_a_serial_is_keyed_by_its_usb_port() {
    let driver = driver(vec![card("sdz", None)]);
    let candidates = block_on(driver.discover(&link())).unwrap();
    let identity = block_on(driver.identify(&candidates[0])).unwrap();
    assert_eq!(identity.key.to_string(), format!("{FAMILY}:usb-1-4.2"));
}

#[test]
fn identify_refuses_a_disk_that_is_gone_or_no_longer_writable() {
    let driver = driver(vec![card("sdz", Some("A1"))]);
    let candidates = block_on(driver.discover(&link())).unwrap();

    let gone = driver_without_disks();
    let error = block_on(gone.identify(&candidates[0])).unwrap_err();
    assert!(matches!(error, DriverError::Unreachable(message) if message.contains("unplugged")));
}

fn driver_without_disks() -> DiskDriver {
    driver(Vec::new())
}

#[test]
fn a_listing_that_fails_is_unreachable_not_empty() {
    let driver = DiskDriver::new(Arc::new(Broken));
    let error = block_on(driver.discover(&link())).unwrap_err();
    assert!(matches!(error, DriverError::Unreachable(_)));
}

#[test]
fn the_driver_grants_flash_for_a_removable_disk_and_nothing_else() {
    let driver = driver(vec![card("sdz", Some("A1"))]);
    let candidates = block_on(driver.discover(&link())).unwrap();
    let identity = block_on(driver.identify(&candidates[0])).unwrap();

    let granted = driver.capabilities(&identity);

    assert_eq!(
        granted.shown(DeviceState::RemovableDisk),
        vec![CapabilityKind::Info, CapabilityKind::Flash]
    );
    assert_eq!(
        granted.shown(DeviceState::Running),
        vec![CapabilityKind::Info]
    );
}

#[test]
fn the_manifest_names_the_family_and_no_link_kind_yet() {
    let driver = driver(Vec::new());
    let manifest = driver.manifest();
    assert_eq!(manifest.family.as_str(), FAMILY);
    assert!(manifest.link_kinds.is_empty());
}
