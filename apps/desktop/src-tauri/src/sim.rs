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

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
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
use atlas_session::Seed;
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
}

impl Scene {
    /// The scene a word names: `empty`, or anything else for the demo.
    #[must_use]
    pub fn named(word: &str) -> Self {
        if word == "empty" {
            Self::Empty
        } else {
            Self::Demo
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
        let demo = scene == Scene::Demo;
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
            ..Hello::default()
        };
        let node_id = format!("{}:{}", hello.family, hello.serial);
        let mut seeds = Vec::new();
        let node = demo
            .then(|| start_node(Kind::WithLemnos(hello)))
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
        parts.registry.register(Arc::new(BoardDriver::new(seeds)));

        let world = Self {
            inner: Arc::new(Inner {
                scene,
                fleet,
                bus,
                disks,
                net,
                node,
                node_id,
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
        if self.inner.scene != Scene::Demo {
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
                    *running = Some(
                        SimNode::start_with(&node.dir, &node.kind, Some(node.port))
                            .map_err(|error| CallError(format!("the simulated node: {error}")))?,
                    );
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

    /// The ids of every simulated device, for a test that wants to know what there is.
    #[must_use]
    pub fn ids(&self) -> BTreeSet<String> {
        if self.inner.scene != Scene::Demo {
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
        "sim.ids" => {
            serde_json::to_value(world.ids()).map_err(|error| CallError(error.to_string()))
        }
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}
