//! A fake Orion for the driver's tests: one node, its status lane, and a
//! board agent that plays the `update.*` key sequence of an update.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use atlas_driver::{
    Artifact, DeviceKey, DeviceMode, DriverError, Family, Identity, LinkId, ReleaseRef,
};
use atlas_driver_orion::{BundleHost, OrionTransport};
use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, NodeHostFacts, NodeRecord, StatusEntry, StatusQuery,
    StatusSubject, TypedConfigValue,
};
use orion_core::NodeId;

/// How the fake board's update goes.
#[derive(Clone, Copy, PartialEq)]
pub enum Ending {
    Confirms,
    RollsBack,
    /// The download fails its checksum while staging.
    FailsStaging,
    /// Staging never finishes (until `update.cancel`).
    Hangs,
}

/// One step of the board's agent: the `update.*` keys it publishes, and
/// whether the board restarts with it (a new boot id from then on).
pub struct Step {
    pub keys: Vec<(&'static str, TypedConfigValue)>,
    pub rebooted: bool,
}

pub fn text(value: &str) -> TypedConfigValue {
    TypedConfigValue::String(value.into())
}

pub fn step(rebooted: bool, keys: &[(&'static str, TypedConfigValue)]) -> Step {
    Step {
        keys: keys.to_vec(),
        rebooted,
    }
}

pub fn script(ending: Ending) -> VecDeque<Step> {
    let staging = |progress| {
        step(
            false,
            &[
                ("update.state", text("staging")),
                ("update.progress", TypedConfigValue::UInt(progress)),
                ("update.boot_id", text("boot-1")),
            ],
        )
    };
    let mut steps = VecDeque::from([staging(250)]);
    match ending {
        Ending::Hangs => return steps,
        Ending::FailsStaging => {
            steps.push_back(step(
                false,
                &[
                    ("update.state", text("error")),
                    (
                        "update.error",
                        text("the downloaded image failed its SHA-256 check"),
                    ),
                ],
            ));
            return steps;
        }
        Ending::Confirms | Ending::RollsBack => {}
    }
    steps.extend([
        staging(750),
        step(
            false,
            &[
                ("update.state", text("staged")),
                ("update.progress", TypedConfigValue::UInt(1000)),
            ],
        ),
        // The board restarts right after saying so.
        step(true, &[("update.state", text("rebooting"))]),
        // The board is back; the old boot's keys are still on the lane.
        step(false, &[]),
        step(
            false,
            &[
                ("update.state", text("trying")),
                ("update.boot_id", text("boot-2")),
            ],
        ),
    ]);
    steps.push_back(if ending == Ending::Confirms {
        step(
            false,
            &[
                ("update.state", text("confirmed")),
                ("update.version_active", text("2026.4.0")),
            ],
        )
    } else {
        step(
            false,
            &[
                ("update.state", text("rolled-back")),
                ("update.error", text("PhotonVision didn't start")),
            ],
        )
    });
    steps
}

#[derive(Default)]
pub struct State {
    pub nodes: Vec<NodeRecord>,
    pub status: Vec<StatusEntry>,
    pub actions: HashMap<String, ActionResult>,
    pub reject: bool,
    pub ending: Option<Ending>,
    /// What the board's agent does next, one step per update-key query.
    pub steps: VecDeque<Step>,
    /// Every action the board received, by name.
    pub received: Vec<String>,
    /// The last `update` intent, as the board receives it.
    pub update_request: Option<ActionRequest>,
    /// Stage nothing until the board has downloaded the image.
    pub hold_for_download: bool,
    pub downloaded: bool,
}

#[derive(Clone, Default)]
pub struct FakeOrion(pub Arc<Mutex<State>>);

pub const NODE: &str = "raze-node";

pub fn node(boot_id: &str) -> NodeRecord {
    NodeRecord::builder(NodeId::new(NODE))
        .host(NodeHostFacts {
            board_serial: Some("a317bcbee5226d57\0".into()),
            boot_id: Some(boot_id.into()),
            cpu_count: Some(4),
            ..NodeHostFacts::default()
        })
        .build()
}

pub fn status(key: &str, value: TypedConfigValue) -> StatusEntry {
    StatusEntry::new(StatusSubject::Node(NodeId::new(NODE)), key, value)
}

impl FakeOrion {
    pub fn new() -> Self {
        let fake = Self::default();
        fake.0.lock().unwrap().nodes = vec![node("boot-1")];
        fake
    }

    pub fn set_status(&self, key: &str, value: TypedConfigValue) {
        self.0.lock().unwrap().set(key, value);
    }

    pub fn received(&self) -> Vec<String> {
        self.0.lock().unwrap().received.clone()
    }
}

impl State {
    pub fn set(&mut self, key: &str, value: TypedConfigValue) {
        self.status.retain(|entry| entry.key != key);
        self.status.push(status(key, value));
    }

    /// The agent's next step, when the board may go on.
    pub fn advance(&mut self) {
        if self.hold_for_download && !self.downloaded {
            return;
        }
        if let Some(next) = self.steps.pop_front() {
            if next.rebooted {
                self.nodes = vec![node("boot-2")];
            }
            for (key, value) in next.keys {
                self.set(key, value);
            }
        }
    }
}

pub fn result(request: &ActionRequest, state: ActionState) -> ActionResult {
    ActionResult {
        action_id: request.action_id.clone(),
        target: request.target.clone(),
        name: request.name.clone(),
        state,
        output: BTreeMap::new(),
        handled_by: NodeId::new(NODE),
        requested_by: "atlas".into(),
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

#[async_trait]
impl OrionTransport for FakeOrion {
    async fn nodes(&self) -> Result<Vec<NodeRecord>, DriverError> {
        Ok(self.0.lock().unwrap().nodes.clone())
    }

    async fn status(&self, query: StatusQuery) -> Result<Vec<StatusEntry>, DriverError> {
        let prefix = query.key_prefix.unwrap_or_default();
        let mut state = self.0.lock().unwrap();
        if prefix == "update." {
            state.advance();
        }
        Ok(state
            .status
            .iter()
            .filter(|entry| entry.key.starts_with(&prefix))
            .cloned()
            .collect())
    }

    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError> {
        let mut state = self.0.lock().unwrap();
        state.received.push(request.name.clone());
        if state.reject {
            let answer = result(
                &request,
                ActionState::Rejected {
                    reason: "no handler for locate".into(),
                },
            );
            state
                .actions
                .insert(request.action_id.clone(), answer.clone());
            return Ok(answer);
        }
        let mut answer = result(&request, ActionState::Accepted);
        match request.name.as_str() {
            // Asynchronous: the agent starts the stage and says so.
            "update" => {
                state.update_request = Some(request.clone());
                let ending = state.ending.unwrap_or(Ending::Confirms);
                state.steps = script(ending);
                answer.output.insert("phase".into(), text("staging"));
            }
            "update.cancel" => {
                state.steps.clear();
                state.set("update.state", text("cancelled"));
                answer.output.insert("phase".into(), text("cancelled"));
            }
            _ => {}
        }
        state
            .actions
            .insert(request.action_id.clone(), answer.clone());
        Ok(answer)
    }

    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError> {
        let mut state = self.0.lock().unwrap();
        let Some(current) = state.actions.get_mut(action_id) else {
            return Ok(None);
        };
        if current.state == ActionState::Accepted {
            current.state = ActionState::Succeeded;
        }
        Ok(Some(current.clone()))
    }
}

pub struct LocalBundles;

impl BundleHost for LocalBundles {
    fn url_for(&self, artifact: &Artifact, _peer: Option<IpAddr>) -> Result<String, DriverError> {
        Ok(format!("http://atlas.local:7700/bundles/{}", artifact.name))
    }
}

pub fn raze(board_serial: Option<&str>) -> Identity {
    let mut attributes = BTreeMap::new();
    if let Some(serial) = board_serial {
        attributes.insert("board_serial".to_string(), serial.to_string());
    }
    Identity {
        key: DeviceKey::new("raze", "a317bcbee5226d57"),
        model: "Raze Gen 1".into(),
        mode: DeviceMode::Normal,
        versions: BTreeMap::from([("os".to_string(), "2026.3.0".to_string())]),
        name: Some("photonvision".into()),
        link: LinkId("mdns".into()),
        address: "http://172.31.250.1:5899/.well-known/pd-device".into(),
        attributes,
    }
}

pub fn release() -> ReleaseRef {
    ReleaseRef {
        family: Family::new("raze"),
        version: "2026.4.0".into(),
        artifact: Some(Artifact {
            name: "photonvision-2026.4.0-raze.img.xz".into(),
            path: PathBuf::from("/tmp/x.img.xz"),
            sha256: "ab".repeat(32),
            size_bytes: 400_000_000,
        }),
    }
}
