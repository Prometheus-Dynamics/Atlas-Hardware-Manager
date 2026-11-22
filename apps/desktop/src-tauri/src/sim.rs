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
use atlas_disk::{Disk, DiskError, DiskSource};
use atlas_driver_disk::DiskDriver;
use atlas_driver_rockchip::RockchipDriver;
use atlas_inventory::Robot;
use atlas_mock::{MockDevice, MockFleet, SIM_FAMILY, SIM_LINK};
use atlas_robot::{Answer, Reach, SYSTEMCORE_FAMILY};
use atlas_session::{RealPause, Seed, Sessions};
use atlas_sim_node::socket::NodeDir;
use atlas_sim_node::{Hello, Kind, SimNode};
use atlas_types::{DeviceKey, DeviceState, Link, LinkId, LinkKind};
use atlas_usb::sim::SimBus;
use atlas_usb::{DeviceSummary, UsbId};
use serde::Deserialize;
use serde_json::Value;

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
}

#[async_trait]
impl Reach for SimNet {
    async fn get(&self, ip: Ipv4Addr, port: u16, path: &str) -> Option<Answer> {
        (self.open(ip, port).await && path == CONTROLLER_API.1).then(|| Answer {
            status: 200,
            server: None,
            body: format!(r#"{{"team":"{TEAM}","team_number":{TEAM}}}"#),
        })
    }

    async fn open(&self, ip: Ipv4Addr, port: u16) -> bool {
        self.controller_up.load(Ordering::Relaxed) && ip == CONTROLLER && port == CONTROLLER_API.0
    }

    async fn resolve(&self, _name: &str) -> Vec<Ipv4Addr> {
        Vec::new()
    }
}

/// A made-up disk list.
#[derive(Debug, Default)]
pub struct SimDisks {
    disks: Mutex<Vec<Disk>>,
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
    bus: Arc<SimBus>,
    disks: Arc<SimDisks>,
    net: Arc<SimNet>,
    node: Option<Node>,
    /// The key of the simulated node's board, as the board driver reports it.
    node_id: String,
    picks: Arc<Picks>,
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
        let bus = Arc::new(SimBus::new());
        if demo {
            bus.attach(DeviceSummary::new(ROM_PORT, ROM_ID.vendor, ROM_ID.product));
        }
        parts
            .registry
            .register(Arc::new(RockchipDriver::new(bus.clone(), vec![ROM_ID])));

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

        // The robot drivers on a made-up robot network.
        let net = Arc::new(SimNet {
            controller_up: AtomicBool::new(demo),
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

        let world = Self {
            inner: Arc::new(Inner {
                scene,
                fleet,
                bus,
                disks,
                net,
                node,
                node_id,
                picks: Arc::new(Picks::default()),
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
        ]
    }

    /// Follows the app and does what a person would have done before the picture: names the
    /// boards, puts the controller in its robot, and unplugs the old board once it was seen, so
    /// the list holds an offline record. Called once, when the core is open.
    pub fn settle(&self, core: &Core) {
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
                inner
                    .bus
                    .attach(DeviceSummary::new(ROM_PORT, ROM_ID.vendor, ROM_ID.product));
            } else {
                inner.bus.detach(ROM_PORT);
            }
            return Ok(());
        }
        if id == format!("{SYSTEMCORE_FAMILY}:{TEAM}") {
            inner.net.controller_up.store(online, Ordering::Relaxed);
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

    /// The simulated USB bus the boot ROM driver looks at.
    #[must_use]
    pub fn bus(&self) -> &Arc<SimBus> {
        &self.inner.bus
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
        "sim.ids" => {
            serde_json::to_value(world.ids()).map_err(|error| CallError(error.to_string()))
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}
