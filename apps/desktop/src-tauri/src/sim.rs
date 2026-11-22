//! The simulated devices (feature `sim`): what the app shows with no hardware.
//!
//! [`World::install`] fills in the app's parts with simulated devices behind the real drivers
//! wherever a driver has a seam for it, so the list shows what those drivers really report:
//!
//! - a running board: the simulated Orion node (`atlas-sim-node`) behind the real board driver,
//!   seeded on a USB network link and on the network link, so it is one record with two links;
//! - a board in its boot ROM: a device on the simulated USB bus (`atlas_usb::sim`) behind the
//!   real boot ROM driver;
//! - a removable disk: a made-up disk list behind the real disk driver;
//! - a robot's controller that does not run Orion yet: a made-up robot network ([`SimNet`])
//!   behind the real robot drivers, for the team of the simulated robot;
//! - a board in recovery, and a board that was seen and is gone: the mock fleet (`atlas-mock`).
//!   The simulated node cannot be put in recovery (see its `Mode`), so the mock stands in.
//!
//! Nothing on the computer is looked at: no USB, no disks, no network beyond the node's own
//! port on 127.0.0.1. The `sim.*` calls change the simulated world the way the world changes
//! real devices (a board is unplugged, comes back), so the click tests can show every state.
//! They exist only in a build with this feature.

use std::collections::{BTreeSet, VecDeque};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use atlas_board::BoardDriver;
use atlas_disk::{Disk, DiskError, DiskSource, Guard, Platform, Proof, RawDisk, ResumePolicy};
use atlas_driver_disk::DiskDriver;
use atlas_driver_rockchip::RockchipDriver;
use atlas_inventory::Robot;
use atlas_mock::{MockDevice, MockFleet, SIM_FAMILY, SIM_LINK};
use atlas_robot::{Answer, RADIO_FAMILY, Reach, SYSTEMCORE_FAMILY};
use atlas_rockchip::sim::{Fault, SimRockchip};
use atlas_session::{RealPause, Seed, Sessions};
use atlas_sim_node::socket::NodeDir;
use atlas_sim_node::{Hello, Kind, SimNode};
use atlas_types::{DeviceKey, DeviceState, Link, LinkId, LinkKind};
use atlas_usb::{DeviceSummary, OpenOptions, Timeouts, UsbBus, UsbError, UsbId, UsbIo};
use serde::Deserialize;
use serde_json::Value;

pub mod feed;

use crate::call::{CallError, CallResult, args};
use crate::core::{Core, Parts};
use crate::drivers::TeamList;

/// The simulated robot, its team, and where its controller answers.
pub const ROBOT: &str = "Practice field";
const TEAM: u16 = 9999;
const CONTROLLER: Ipv4Addr = Ipv4Addr::new(10, 99, 99, 2);
/// The controller's own API port and path (what a controller without Orion answers on).
const CONTROLLER_API: (u16, &str) = (4800, "/api/team");

/// The simulated boot ROM device: where it is plugged in and what it shows.
const ROM_PORT: &str = "1-3";
const ROM_ID: UsbId = UsbId {
    vendor: 0x2207,
    product: 0x350b,
};

const USB_LINK: &str = "usb-1-2";
const NETWORK_LINK: &str = "lan";
const BOOT_LINK: &str = "usb";
const DISK_LINK: &str = "disks";

/// Which simulated world to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// One device in every state the device list shows.
    Demo,
    /// The same drivers and links with nothing plugged in.
    Empty,
    /// The demo, with the running board a plain Orion node: no Lemnos, so no slots, readings
    /// or actions of a board.
    Plain,
}

impl Scene {
    /// The scene a word names: `empty`, or anything else for the demo.
    #[must_use]
    pub fn named(word: &str) -> Self {
        match word {
            "empty" => Self::Empty,
            "plain" => Self::Plain,
            _ => Self::Demo,
        }
    }
}

/// A made-up robot network: one controller that answers its own API and does not run Orion.
#[derive(Debug)]
pub struct SimNet {
    controller_up: AtomicBool,
    radio_up: AtomicBool,
}

/// Where the simulated robot's radio answers, and what its status page says.
const RADIO: Ipv4Addr = Ipv4Addr::new(10, 99, 99, 1);
const RADIO_WEB: (u16, &str) = (80, "/status");

fn radio_status() -> String {
    serde_json::json!({
        "mode": "ROBOT_RADIO",
        "channel": "53",
        "teamNumber": TEAM,
        "status": "ACTIVE",
        "version": "SIM-RADIO_1.2.0",
        "networkStatus6": {
            "ssid": TEAM.to_string(), "isEnabled": true, "isLinked": true,
            "signalDbm": -58, "connectionQuality": "good"
        },
        "networkStatus24": { "ssid": TEAM.to_string(), "isEnabled": false }
    })
    .to_string()
}

#[async_trait]
impl Reach for SimNet {
    async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer> {
        if !self.open(ip, port).await {
            return None;
        }
        let body = if ip == CONTROLLER && path == CONTROLLER_API.1 {
            format!(r#"{{"team":"{TEAM}","team_number":{TEAM}}}"#)
        } else if ip == RADIO && path == RADIO_WEB.1 {
            radio_status()
        } else {
            return None;
        };
        Some(Answer {
            status: 200,
            server: None,
            body,
        })
    }

    async fn open(&self, ip: Ipv4Addr, port: u16) -> bool {
        (self.controller_up.load(Ordering::Relaxed) && ip == CONTROLLER && port == CONTROLLER_API.0)
            || (self.radio_up.load(Ordering::Relaxed) && ip == RADIO && port == RADIO_WEB.0)
    }

    async fn resolve(&self, _name: &str) -> Vec<Ipv4Addr> {
        Vec::new()
    }
}

/// A made-up disk list, and what a write to one of its disks lands in: a file under the app's
/// own data directory. Nothing here opens a disk of this computer.
#[derive(Debug, Default)]
pub struct SimDisks {
    disks: Mutex<Vec<Disk>>,
    /// While set, a write to the card waits: how a test looks at a write in progress.
    hold: Arc<AtomicBool>,
    /// Set while a write is waiting on `hold`: the write has opened the card and is at a write.
    held: AtomicBool,
    /// Where the card's bytes are kept.
    backing: Mutex<Option<PathBuf>>,
}

impl SimDisks {
    fn present(&self, disk: &Disk) -> bool {
        self.disks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|listed| listed.path == disk.path)
    }
}

fn disk_io(path: &std::path::Path, error: &std::io::Error) -> DiskError {
    DiskError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

/// The simulated card, opened: a file, and a card that can be pulled while it is written.
struct SimCard {
    file: std::fs::File,
    disk: Disk,
    disks: Arc<SimDisks>,
}

impl SimCard {
    fn check(&self) -> std::io::Result<()> {
        if self.disks.present(&self.disk) {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "the card was removed",
            ))
        }
    }
}

impl RawDisk for SimCard {
    fn write_at(&mut self, offset: u64, data: &[u8]) -> std::io::Result<()> {
        use std::os::unix::fs::FileExt;
        while self.disks.hold.load(Ordering::Relaxed) && self.disks.present(&self.disk) {
            self.disks.held.store(true, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.disks.held.store(false, Ordering::Relaxed);
        self.check()?;
        self.file.write_all_at(data, offset)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        use std::os::unix::fs::FileExt;
        self.check()?;
        self.file.read_exact_at(buf, offset)
    }

    fn sync(&mut self) -> std::io::Result<()> {
        self.check()?;
        self.file.sync_data()
    }
}

/// The simulated disks as a platform for `atlas-disk`'s write: the real pipeline (the fresh
/// listing, the checks, the segments, the read-back, the reattach after a drop) on a file.
#[derive(Debug)]
struct SimPlatform(Arc<SimDisks>);

impl DiskSource for SimPlatform {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        self.0.disks()
    }
}

impl Platform for SimPlatform {
    fn name(&self) -> &'static str {
        "the simulated computer"
    }

    fn proof(&self) -> Proof {
        Proof::Proven
    }

    fn prepare(&self, _disk: &Disk) -> Result<Guard, DiskError> {
        Ok(Box::new(()))
    }

    fn open(&self, disk: &Disk) -> Result<Box<dyn RawDisk>, DiskError> {
        if !self.0.present(disk) {
            return Err(DiskError::NotFound(disk.path.display().to_string()));
        }
        let backing = self
            .0
            .backing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                DiskError::NotFound("the simulated card has nowhere to keep its bytes".into())
            })?;
        if let Some(dir) = backing.parent() {
            std::fs::create_dir_all(dir).map_err(|error| disk_io(&backing, &error))?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&backing)
            .map_err(|error| disk_io(&backing, &error))?;
        Ok(Box::new(SimCard {
            file,
            disk: disk.clone(),
            disks: Arc::clone(&self.0),
        }))
    }

    fn finish(&self, _disk: &Disk, mut raw: Box<dyn RawDisk>) -> Result<(), DiskError> {
        raw.sync()
            .map_err(|error| disk_io(std::path::Path::new("the simulated card"), &error))
    }
}

/// The simulated USB bus of the boot ROM driver: one simulated board in its boot ROM
/// (`atlas_rockchip::sim`), which can be unplugged and plugged in again as a fresh board.
#[derive(Debug, Default)]
pub struct SimRom {
    board: Mutex<Option<SimRockchip>>,
}

impl SimRom {
    fn board(&self) -> Option<SimRockchip> {
        self.board.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Plugs a board in, in its boot ROM, in place of whatever was there.
    fn plug(&self) {
        *self.board.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(SimRockchip::maskrom(ROM_PORT, ROM_ID, "3588", 122_142_720));
    }

    fn unplug(&self) {
        *self.board.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[async_trait]
impl UsbBus for SimRom {
    async fn devices(&self) -> Result<Vec<DeviceSummary>, UsbError> {
        match self.board() {
            Some(board) => board.devices().await,
            None => Ok(Vec::new()),
        }
    }

    async fn open(
        &self,
        location: &str,
        options: &OpenOptions,
    ) -> Result<Box<dyn UsbIo>, UsbError> {
        match self.board() {
            Some(board) => board.open(location, options).await,
            None => Err(UsbError::NotFound(location.to_string())),
        }
    }
}

/// A loader file for the simulated board, laid out as the chip vendor's tool lays one out. Its
/// stages are filler: the simulated boot ROM checks their framing, not what they would run.
fn sample_loader() -> Vec<u8> {
    use atlas_rockchip::loader::{EntryKind, build_loader};
    let ddr = vec![0x11u8; 5000];
    let usbplug = vec![0x22u8; 4094];
    build_loader(
        b"8853",
        &[
            (EntryKind::Ddr, "ddr", &ddr, 0),
            (EntryKind::Usbplug, "usbplug", &usbplug, 0),
        ],
        atlas_rockchip::crc::LOADER_POLYNOMIALS[0],
    )
}

/// A first-flash image for the simulated world: a GPT disk whose one partition, `data`, is the
/// image reader's own fixture (two slots built for one board), so `atlas-image` reads it as it
/// reads a real image.
fn sample_image() -> Vec<u8> {
    const DATA: &[u8] = include_bytes!("../../../../crates/flash/image/tests/fixtures/data.ext4");
    const FIRST_LBA: u64 = 2048;
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }
    let start = (FIRST_LBA * 512) as usize;
    let end = start + DATA.len();
    let mut disk = vec![0u8; end];
    disk[510] = 0x55;
    disk[511] = 0xAA;
    let mut entries = vec![0u8; 128 * 128];
    entries[0..16].copy_from_slice(&[0x11; 16]);
    entries[16..32].copy_from_slice(&[1; 16]);
    let last = FIRST_LBA + (DATA.len() as u64).div_ceil(512) - 1;
    entries[32..40].copy_from_slice(&FIRST_LBA.to_le_bytes());
    entries[40..48].copy_from_slice(&last.to_le_bytes());
    for (index, unit) in "data".encode_utf16().enumerate() {
        entries[56 + index * 2..58 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    let mut header = vec![0u8; 92];
    header[0..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    header[12..16].copy_from_slice(&92u32.to_le_bytes());
    header[24..32].copy_from_slice(&1u64.to_le_bytes());
    header[40..48].copy_from_slice(&34u64.to_le_bytes());
    header[48..56].copy_from_slice(&(end as u64 / 512 - 34).to_le_bytes());
    header[56..72].copy_from_slice(&[0x22; 16]);
    header[72..80].copy_from_slice(&2u64.to_le_bytes());
    header[80..84].copy_from_slice(&128u32.to_le_bytes());
    header[84..88].copy_from_slice(&128u32.to_le_bytes());
    header[88..92].copy_from_slice(&crc32(&entries).to_le_bytes());
    let header_crc = crc32(&header);
    header[16..20].copy_from_slice(&header_crc.to_le_bytes());
    disk[512..512 + 92].copy_from_slice(&header);
    disk[1024..1024 + entries.len()].copy_from_slice(&entries);
    disk[start..end].copy_from_slice(DATA);
    disk
}

impl DiskSource for SimDisks {
    fn disks(&self) -> Result<Vec<Disk>, DiskError> {
        Ok(self.disks.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

fn card() -> Disk {
    Disk {
        path: "/dev/sim-card".into(),
        name: "sim-card".into(),
        size_bytes: 64_000_000_000,
        vendor: None,
        model: Some("SD card".into()),
        serial: Some("SIM-CARD-1".into()),
        removable: true,
        usb: true,
        usb_port: Some("1-5".into()),
        system: false,
        mount_points: Vec::new(),
    }
}

/// The simulated Orion node, and what is needed to start it again where it was.
struct Node {
    dir: NodeDir,
    kind: Kind,
    port: u16,
    running: Mutex<Option<SimNode>>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// A name the simulated world gives a record once it is found, and the robot it belongs to.
/// These are the user's own label and robot, set through the inventory as a person would.
struct Tag {
    id: String,
    label: Option<&'static str>,
    robot: Option<&'static str>,
}

/// The simulated devices of one app. Cheap to clone; clones share one world.
#[derive(Clone, Debug)]
pub struct World {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    scene: Scene,
    fleet: MockFleet,
    rom: Arc<SimRom>,
    disks: Arc<SimDisks>,
    net: Arc<SimNet>,
    node: Option<Node>,
    /// The key of the simulated node's board, as the board driver reports it.
    node_id: String,
    picks: Arc<Picks>,
    feed: Arc<feed::SimFeed>,
}

/// The files the simulated person will choose, in order: what stands in for the file dialog
/// where there is no window. A test queues one with `sim.pick_bundle`; with none queued the
/// person chose nothing.
#[derive(Debug, Default)]
pub struct Picks {
    queued: Mutex<VecDeque<PathBuf>>,
}

impl crate::board::Picker for Picks {
    fn pick(&self, _title: &str, _extensions: &[&str]) -> Option<PathBuf> {
        self.queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
    }
}

/// The slots the simulated board starts with: the running default, the one before it, an older
/// starred one and an older plain one. A board that was just started holds only its first slot.
fn seed_slots(node: &SimNode) {
    use atlas_sim_node::slots::{Entry, Good, Signed};
    let entry = |id: &str, version: &str, seq: u64, pinned: bool| Entry {
        id: id.into(),
        product: "demo-os".into(),
        board: SIM_FAMILY.into(),
        version: version.into(),
        variant: String::new(),
        kernel: String::new(),
        display_name: format!("demo-os {version}"),
        size: 2_100_000_000,
        verity: true,
        signed: Signed::Unsigned,
        good: Good::Yes,
        failed: false,
        failures: 0,
        fail_reason: String::new(),
        pinned,
        seq,
    };
    let mut model = node.model();
    model.slots.entries = vec![
        entry("slot-1", "2026.9.12", 1, false),
        entry("slot-2", "2026.9.30", 2, true),
        entry("slot-3", "2026.10.20", 3, false),
        entry("slot-4", "2026.11.2", 4, false),
    ];
    model.slots.active = Some("slot-4".into());
    model.slots.default = Some("slot-4".into());
    model.slots.previous = Some("slot-3".into());
    model.slots.keep = 3;
}

fn mock_key(serial: &'static str) -> DeviceKey {
    DeviceKey::from_static(SIM_FAMILY, serial)
}

const OLD_BOARD: &str = "SIM-OLD-1";
const RECOVERY_BOARD: &str = "SIM-REC-1";

impl World {
    /// Adds the simulated drivers and links to the app's parts. What cannot be started (the
    /// simulated node's port or socket) is left out and returned as a line for the log.
    pub fn install(parts: &mut Parts, scene: Scene, teams: &TeamList) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let demo = scene != Scene::Empty;
        for (id, kind, label) in [
            (USB_LINK, LinkKind::UsbNetwork, "USB 1-2"),
            (NETWORK_LINK, LinkKind::Ethernet, "Network"),
            (BOOT_LINK, LinkKind::UsbBoot, "USB"),
            (DISK_LINK, LinkKind::LocalDisk, "card reader"),
            (SIM_LINK, LinkKind::Simulated, "Simulated devices"),
        ] {
            parts.links.push(Link {
                id: LinkId::new(id),
                kind,
                label: label.into(),
            });
        }

        // The mock fleet: a board in recovery, and one that will be unplugged once it was seen.
        let fleet = if demo {
            MockFleet::of([
                MockDevice::new(RECOVERY_BOARD, DeviceState::Recovery).named("bench-c"),
                MockDevice::new(OLD_BOARD, DeviceState::Running)
                    .named("old-bench")
                    .with_version("product", "demo-os")
                    .with_version("os", "2026.10.20"),
            ])
        } else {
            MockFleet::of([])
        };
        parts.registry.register(Arc::new(fleet.driver()));

        // The boot ROM driver on a simulated bus.
        let rom = Arc::new(SimRom::default());
        if demo {
            rom.plug();
        }
        parts
            .registry
            .register(Arc::new(RockchipDriver::new(rom.clone(), vec![ROM_ID])));
        // Its loader: the real one, with transfer limits short enough for a failed try to be
        // seen and tried again while a person watches.
        let limits = OpenOptions {
            interface: None,
            timeouts: Timeouts {
                control: std::time::Duration::from_millis(400),
                bulk: std::time::Duration::from_millis(400),
            },
        };
        match crate::flash::RockchipRom::new(
            atlas_driver_rockchip::FAMILY,
            rom.clone(),
            sample_loader(),
        ) {
            Ok(loader) => parts.roms.push(Arc::new(
                loader.with_limits(limits, std::time::Duration::from_secs(3)),
            )),
            Err(error) => problems.push(format!("the simulated loader file: {error}")),
        }

        // The disk driver on a made-up disk list.
        let disks = Arc::new(SimDisks::default());
        if demo {
            disks
                .disks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(card());
        }
        parts
            .registry
            .register(Arc::new(DiskDriver::new(disks.clone())));
        // Its writer: the real write pipeline on the simulated platform, in segments small
        // enough for the sample image to take several.
        parts.disk_writer = Some(Arc::new(crate::flash::PlatformWriter::new(
            Arc::new(SimPlatform(disks.clone())),
            ResumePolicy {
                segment_bytes: 256 << 10,
                max_backoff: std::time::Duration::from_millis(500),
            },
        )));

        // The robot drivers on a made-up robot network.
        let net = Arc::new(SimNet {
            controller_up: AtomicBool::new(demo),
            radio_up: AtomicBool::new(demo),
        });
        for driver in atlas_robot::drivers(net.clone(), Arc::new(teams.clone())) {
            parts.registry.register(driver);
        }

        // The board driver against the simulated Orion node, on two links.
        let hello = Hello {
            product: "demo-os".into(),
            os_version: "2026.11.2".into(),
            slot: "slot-4".into(),
            ..Hello::default()
        };
        let node_id = format!("{}:{}", hello.family, hello.serial);
        let mut seeds = Vec::new();
        let kind = if scene == Scene::Plain {
            Kind::Plain
        } else {
            Kind::WithLemnos(hello)
        };
        let node = demo
            .then(|| start_node(kind))
            .and_then(|started| started.map_err(|error| problems.push(error)).ok());
        if let Some(node) = &node {
            let address = format!("orion+tcp://127.0.0.1:{}", node.port);
            for link in [USB_LINK, NETWORK_LINK] {
                match Seed::new(LinkId::new(link), &address) {
                    Ok(seed) => seeds.push(seed),
                    Err(error) => problems.push(format!("simulated node on {link}: {error}")),
                }
            }
        }
        let sessions = Arc::new(Sessions::new(Arc::new(RealPause)));
        parts.registry.register(Arc::new(BoardDriver::with_sessions(
            seeds,
            sessions.clone(),
        )));
        parts.sessions = Some(sessions);

        let sim_feed = Arc::new(feed::SimFeed::default());
        parts.fetch = Some(sim_feed.clone());
        let world = Self {
            inner: Arc::new(Inner {
                scene,
                fleet,
                rom,
                disks,
                net,
                node,
                node_id,
                picks: Arc::new(Picks::default()),
                feed: Arc::clone(&sim_feed),
            }),
        };
        (world, problems)
    }

    /// The names and the robot the simulated world gives its records.
    fn tags(&self) -> Vec<Tag> {
        let tag = |id: String, label, robot| Tag { id, label, robot };
        vec![
            tag(self.inner.node_id.clone(), Some("bench-a"), None),
            tag(
                format!("{}:usb-{ROM_PORT}", atlas_driver_rockchip::FAMILY),
                Some("bench-b"),
                None,
            ),
            tag(
                format!("{SYSTEMCORE_FAMILY}:{TEAM}"),
                Some("sc-practice"),
                Some(ROBOT),
            ),
            tag(
                format!("{RADIO_FAMILY}:{TEAM}"),
                Some("field-radio"),
                Some(ROBOT),
            ),
        ]
    }

    /// Follows the app and does what a person would have done before the picture: names the
    /// boards, puts the controller in its robot, and unplugs the old board once it was seen, so
    /// the list holds an offline record. Called once, when the core is open.
    pub fn settle(&self, core: &Core) {
        *self
            .inner
            .disks
            .backing
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = core
            .dirs()
            .map(|dirs| dirs.data.join("sim-files").join("sim-card.bin"));
        if self.inner.scene == Scene::Empty {
            return;
        }
        core.change_inventory(|inventory, _| {
            inventory.put_robot(Robot {
                name: ROBOT.into(),
                team: Some(TEAM.to_string()),
                notes: None,
            });
        });
        let (world, core) = (self.clone(), core.clone());
        tokio::spawn(async move {
            let mut updates = core.subscribe();
            let mut unplugged = false;
            loop {
                for tag in world.tags() {
                    let Some(key) = DeviceKey::parse(&tag.id) else {
                        continue;
                    };
                    let wanted = tag.label.map(str::to_string);
                    let (known, labelled, placed) = core.with_inventory(|inventory| {
                        let record = inventory.get(&key);
                        (
                            record.is_some(),
                            record.is_some_and(|record| record.label == wanted),
                            record.is_some_and(|record| record.robot.as_deref() == tag.robot),
                        )
                    });
                    if known && !(labelled && placed) {
                        core.change_inventory(|inventory, now| {
                            if !labelled {
                                inventory.set_label(&key, wanted.clone(), now);
                            }
                            if !placed {
                                inventory.set_device_robot(
                                    &key,
                                    tag.robot.map(str::to_string),
                                    now,
                                );
                            }
                        });
                    }
                }
                let old = mock_key(OLD_BOARD);
                if !unplugged && core.with_inventory(|inventory| inventory.get(&old).is_some()) {
                    world.inner.fleet.set_online(&old, false);
                    unplugged = true;
                }
                match updates.recv().await {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Takes a simulated device off its link, or puts it back, whichever driver finds it.
    fn set_online(&self, id: &str, online: bool) -> Result<(), CallError> {
        let inner = &self.inner;
        if id == inner.node_id {
            let node = inner
                .node
                .as_ref()
                .ok_or_else(|| CallError::from("the simulated node did not start"))?;
            let mut running = node.running.lock().unwrap_or_else(|e| e.into_inner());
            match (online, running.is_some()) {
                (false, _) => *running = None,
                (true, true) => {}
                (true, false) => {
                    let started = SimNode::start_with(&node.dir, &node.kind, Some(node.port))
                        .map_err(|error| CallError(format!("the simulated node: {error}")))?;
                    if node.kind != Kind::Plain {
                        seed_slots(&started);
                    }
                    *running = Some(started);
                }
            }
            return Ok(());
        }
        if id == format!("{}:usb-{ROM_PORT}", atlas_driver_rockchip::FAMILY) {
            if online {
                inner.rom.plug();
            } else {
                inner.rom.unplug();
            }
            return Ok(());
        }
        if id == format!("{SYSTEMCORE_FAMILY}:{TEAM}") {
            inner.net.controller_up.store(online, Ordering::Relaxed);
            return Ok(());
        }
        if id == format!("{RADIO_FAMILY}:{TEAM}") {
            inner.net.radio_up.store(online, Ordering::Relaxed);
            return Ok(());
        }
        if id == format!("{}:SIM-CARD-1", atlas_driver_disk::FAMILY) {
            let mut disks = inner.disks.disks.lock().unwrap_or_else(|e| e.into_inner());
            disks.clear();
            if online {
                disks.push(card());
            }
            return Ok(());
        }
        let key =
            DeviceKey::parse(id).ok_or_else(|| CallError(format!("`{id}` is not a device key")))?;
        if inner.fleet.set_online(&key, online) {
            Ok(())
        } else {
            Err(CallError(format!("no simulated device {id}")))
        }
    }

    /// Queues a file as the next one the simulated person chooses.
    pub fn queue_pick(&self, path: PathBuf) {
        self.inner
            .picks
            .queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(path);
    }

    /// The simulated release feed.
    #[must_use]
    pub fn feed(&self) -> &Arc<feed::SimFeed> {
        &self.inner.feed
    }

    /// What stands in for the file dialog.
    #[must_use]
    pub fn picker(&self) -> Arc<dyn crate::board::Picker> {
        self.inner.picks.clone()
    }

    /// Runs `with` on the simulated Orion node, when it is running.
    pub fn with_node<T>(&self, with: impl FnOnce(&SimNode) -> T) -> Option<T> {
        let node = self.inner.node.as_ref()?;
        let running = node.running.lock().unwrap_or_else(|e| e.into_inner());
        running.as_ref().map(with)
    }

    /// The simulated USB bus the boot ROM driver and its loader look at.
    #[must_use]
    pub fn rom(&self) -> &Arc<SimRom> {
        &self.inner.rom
    }

    /// Writes the sample first-flash image under `dir` and queues it as the next file the
    /// person chooses.
    fn pick_image(&self, dir: &std::path::Path, name: &str) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let path = dir.join(name);
        std::fs::write(&path, sample_image()).map_err(|error| error.to_string())?;
        self.inner
            .picks
            .queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(path.clone());
        Ok(path)
    }

    /// The id of the simulated node's board.
    #[must_use]
    pub fn node_id(&self) -> &str {
        &self.inner.node_id
    }

    /// Writes a slot bundle for the simulated board under `dir` and queues it as the next file
    /// the person chooses. `board` overrides the board the bundle says it is for.
    fn pick_bundle(
        &self,
        dir: &std::path::Path,
        id: &str,
        version: &str,
        board: Option<&str>,
    ) -> Result<PathBuf, String> {
        let rootfs = format!("rootfs of {id} at {version}\n")
            .repeat(40_000)
            .into_bytes();
        let boot: [(&str, &[u8]); 2] = [
            ("cmdline.txt", b"console=ttyS0 root=/dev/dm-0\n"),
            ("kernel.img", b"kernel bytes"),
        ];
        let mut manifest = atlas_sim_node::bundle::manifest(id, version, &rootfs, &boot);
        manifest["product"] = Value::String("demo-os".into());
        manifest["display_name"] = Value::String(format!("demo-os {version}"));
        if let Some(board) = board {
            manifest["board"] = Value::String(board.into());
        }
        let bytes = atlas_sim_node::bundle::build(&manifest, None, None, &rootfs, &boot);
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let path = dir.join(format!("demo-os-{version}.tar"));
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        self.inner
            .picks
            .queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(path.clone());
        Ok(path)
    }

    /// The ids of every simulated device, for a test that wants to know what there is.
    #[must_use]
    pub fn ids(&self) -> BTreeSet<String> {
        if self.inner.scene == Scene::Empty {
            return BTreeSet::new();
        }
        let mut ids: BTreeSet<String> = self.tags().into_iter().map(|tag| tag.id).collect();
        ids.insert(format!("{}:SIM-CARD-1", atlas_driver_disk::FAMILY));
        ids.extend(
            self.inner
                .fleet
                .devices()
                .into_iter()
                .map(|device| device.key.to_string()),
        );
        if self.inner.node.is_none() {
            ids.remove(&self.inner.node_id);
        }
        ids
    }
}

fn start_node(kind: Kind) -> Result<Node, String> {
    let dir = atlas_sim_node::socket::node_dir("atlas-app-sim")
        .map_err(|error| format!("the simulated node has no place for its socket: {error}"))?;
    let node = SimNode::start_with(&dir, &kind, None)
        .map_err(|error| format!("the simulated node did not start: {error}"))?;
    if kind != Kind::Plain {
        seed_slots(&node);
    }
    Ok(Node {
        port: node.network().port(),
        dir,
        kind,
        running: Mutex::new(Some(node)),
    })
}

/// The `sim.*` calls.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    let world = core
        .sim()
        .ok_or_else(|| CallError::from("the app is not running the simulated devices"))?
        .clone();
    match name {
        // Takes a simulated device off its link, or puts it back.
        "sim.set_online" => {
            #[derive(Deserialize)]
            struct SetOnline {
                id: String,
                online: bool,
            }
            let SetOnline { id, online } = args(name, arguments)?;
            // Stopping and starting the simulated node joins its threads: off the async threads.
            tokio::task::spawn_blocking(move || world.set_online(&id, online))
                .await
                .map_err(|error| CallError(error.to_string()))??;
            Ok(Value::Null)
        }
        // Queues a slot bundle as the next file the person chooses in a file dialog.
        "sim.pick_bundle" => {
            #[derive(Deserialize)]
            struct Bundle {
                slot: String,
                version: String,
                board: Option<String>,
            }
            let Bundle {
                slot,
                version,
                board,
            } = args(name, arguments)?;
            let dir = core
                .dirs()
                .map(|dirs| dirs.data.join("sim-files"))
                .ok_or_else(|| CallError::from("the app has no data directory"))?;
            let path = tokio::task::spawn_blocking(move || {
                world.pick_bundle(&dir, &slot, &version, board.as_deref())
            })
            .await
            .map_err(|error| CallError(error.to_string()))?
            .map_err(CallError)?;
            Ok(Value::String(path.display().to_string()))
        }
        // The simulated node's test hooks: each changes the board's state at once.
        "sim.node" => {
            #[derive(Deserialize)]
            struct Hook {
                /// Seconds to move the board's slot clock on.
                clock: Option<u64>,
                healthy: Option<bool>,
                hold_installs: Option<bool>,
                log: Option<String>,
                /// Answers how many runs of this action the node took.
                runs: Option<String>,
                lemnos: Option<bool>,
            }
            let hook: Hook = args(name, arguments)?;
            tokio::task::spawn_blocking(move || {
                world.with_node(|node| {
                    if let Some(healthy) = hook.healthy {
                        node.slot_set_healthy(healthy);
                    }
                    if let Some(hold) = hook.hold_installs {
                        node.hold_installs(hold);
                    }
                    if let Some(secs) = hook.clock {
                        node.slot_clock_advance(secs);
                    }
                    if let Some(line) = &hook.log {
                        node.model().log(line);
                    }
                    match hook.lemnos {
                        Some(true) => node.resume_lemnos(),
                        Some(false) => node.stop_lemnos(),
                        None => {}
                    }
                    hook.runs
                        .as_deref()
                        .map_or(Value::Null, |action| Value::from(node.runs_started(action)))
                })
            })
            .await
            .map_err(|error| CallError(error.to_string()))?
            .ok_or_else(|| CallError::from("the simulated node is not running"))
        }
        // Queues a first-flash image as the next file the person chooses. `broken` queues a
        // file that is not an image.
        "sim.pick_image" => {
            #[derive(Deserialize)]
            struct Pick {
                name: Option<String>,
                #[serde(default)]
                broken: bool,
            }
            let Pick { name, broken } = args(name, arguments)?;
            let dir = core
                .dirs()
                .map(|dirs| dirs.data.join("sim-files"))
                .ok_or_else(|| CallError::from("the app has no data directory"))?;
            let name = name.unwrap_or_else(|| "demo-os-2026.11.2.img".into());
            let path = tokio::task::spawn_blocking(move || {
                let path = world.pick_image(&dir, &name)?;
                if broken {
                    std::fs::write(&path, b"this is not a disk image\n".repeat(64))
                        .map_err(|error| error.to_string())?;
                }
                Ok::<_, String>(path)
            })
            .await
            .map_err(|error| CallError(error.to_string()))?
            .map_err(CallError)?;
            Ok(Value::String(path.display().to_string()))
        }
        // The simulated board in its boot ROM: `silent` makes its next transfers go unanswered
        // (each try then fails and is tried again); `fresh` plugs in a board that answers.
        "sim.rom" => {
            #[derive(Deserialize)]
            struct Rom {
                silent: Option<u32>,
                #[serde(default)]
                fresh: bool,
            }
            let Rom { silent, fresh } = args(name, arguments)?;
            if fresh {
                world.rom().plug();
            }
            if let (Some(count), Some(board)) = (silent, world.rom().board()) {
                for _ in 0..count {
                    board.inject(Fault::Timeout);
                }
            }
            Ok(Value::Null)
        }
        // The simulated disks: `hold` makes a write wait until it is let go; `stick` plugs a second
        // disk in or takes it out.
        "sim.disk" => {
            #[derive(Deserialize)]
            struct Card {
                hold: Option<bool>,
                /// Plugs a second removable disk in (a USB stick), or takes it out.
                stick: Option<bool>,
                /// Answers once a write is waiting on the hold: the card is open and being
                /// written, so pulling it now is pulling it mid-write.
                #[serde(default)]
                wait_held: bool,
            }
            let Card {
                hold,
                stick,
                wait_held,
            } = args(name, arguments)?;
            if wait_held {
                let waiting = async {
                    while !world.inner.disks.held.load(Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                };
                tokio::time::timeout(std::time::Duration::from_secs(20), waiting)
                    .await
                    .map_err(|_| CallError::from("no write reached the held card"))?;
            }
            if let Some(hold) = hold {
                world.inner.disks.hold.store(hold, Ordering::Relaxed);
            }
            if let Some(plugged) = stick {
                let mut disks = world
                    .inner
                    .disks
                    .disks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                disks.retain(|disk| disk.name != "sim-stick");
                if plugged {
                    disks.push(Disk {
                        path: "/dev/sim-stick".into(),
                        name: "sim-stick".into(),
                        size_bytes: 16_000_000_000,
                        model: Some("USB stick".into()),
                        serial: Some("SIM-STICK-1".into()),
                        usb_port: Some("2-1".into()),
                        ..card()
                    });
                }
            }
            Ok(Value::Null)
        }
        // Queues a trusted-key file as the next file the person chooses: a sample key made for
        // this (the Ed25519 base point; nothing is signed with it), or a file that is not one.
        "sim.pick_key" => {
            #[derive(Deserialize)]
            struct Pick {
                #[serde(default)]
                broken: bool,
            }
            let Pick { broken } = args(name, arguments)?;
            let dir = core
                .dirs()
                .map(|dirs| dirs.data.join("sim-files"))
                .ok_or_else(|| CallError::from("the app has no data directory"))?;
            let text = if broken {
                "{ this is not a key file".to_string()
            } else {
                serde_json::json!({ "keys": [{
                    "name": "Lab bench (local)",
                    "kind": "minisign",
                    "key_id": "0807060504030201",
                    "public_key": "RWQBAgMEBQYHCFhmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZm",
                    "owner": "This computer's operator",
                    "contact": "bench@example.invalid",
                    "purpose": "dev",
                    "created": "2026-11-01",
                    "expires": null,
                    "note": "Lab test images only."
                }]})
                .to_string()
            };
            let path = dir.join(if broken {
                "notes.json"
            } else {
                "lab-bench-key.json"
            });
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&path, text))
                .map_err(|error| CallError(error.to_string()))?;
            world.queue_pick(path.clone());
            Ok(Value::String(path.display().to_string()))
        }
        // Sets what the simulated release feed holds (`nothing`, `newer`, `stranger`, `down`),
        // or takes the feed away (`off`); how this copy says it was installed; and whether an
        // installer download is held after its first pieces. The feed's key joins this
        // world's own trusted list the first time a feed is set.
        "sim.feed" => {
            #[derive(Deserialize)]
            struct Feed {
                #[serde(default)]
                state: Option<String>,
                #[serde(default)]
                hold: Option<bool>,
                #[serde(default)]
                kind: Option<String>,
            }
            let Feed { state, hold, kind } = args(name, arguments)?;
            let dir = core
                .dirs()
                .map(|dirs| dirs.data.join("sim-files"))
                .ok_or_else(|| CallError::from("the app has no data directory"))?;
            std::fs::create_dir_all(&dir).map_err(|error| CallError(error.to_string()))?;
            if let Some(hold) = hold {
                world.feed().set_hold(hold);
            }
            if let Some(kind) = kind {
                let app_image = dir.join("Atlas.AppImage");
                let install = match kind.as_str() {
                    "deb" => (atlas_release::InstallKind::Deb, None),
                    "rpm" => (atlas_release::InstallKind::Rpm, None),
                    "source" => (atlas_release::InstallKind::Source, None),
                    "app-image" => {
                        std::fs::write(&app_image, b"the old program")
                            .map_err(|error| CallError(error.to_string()))?;
                        (atlas_release::InstallKind::AppImage, Some(app_image))
                    }
                    other => return Err(CallError(format!("no install kind `{other}`"))),
                };
                core.app_update().set_install(crate::update::Install {
                    kind: install.0,
                    app_image: install.1,
                });
            }
            if let Some(state) = state {
                let holds = match state.as_str() {
                    "off" => None,
                    "nothing" => Some(feed::Holds::NothingNewer),
                    "newer" => Some(feed::Holds::Newer),
                    "stranger" => Some(feed::Holds::NewerByAStranger),
                    "down" => Some(feed::Holds::Down),
                    other => return Err(CallError(format!("no feed state `{other}`"))),
                };
                if let Some(holds) = holds {
                    world.feed().set(holds);
                    let key = dir.join("sim-release-key.json");
                    std::fs::write(&key, feed::SimFeed::key_file())
                        .map_err(|error| CallError(error.to_string()))?;
                    // Already trusted from an earlier call: nothing to add.
                    if core.trust().add_file(&key).is_ok() {
                        crate::trust::publish(core);
                    }
                }
                core.app_update()
                    .set_feed(holds.map(|_| feed::FEED_URL.to_owned()));
            }
            Ok(Value::Null)
        }
        // What the app was asked to open outside itself (a folder, a page), where there is no
        // window to open it.
        "sim.opened" => Ok(serde_json::json!(core.opened())),
        "sim.ids" => {
            serde_json::to_value(world.ids()).map_err(|error| CallError(error.to_string()))
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}
