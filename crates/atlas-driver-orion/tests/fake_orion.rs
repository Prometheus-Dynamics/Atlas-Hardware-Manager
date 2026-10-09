use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use atlas_driver::{
    Artifact, CancellationToken, CapabilitySource, DeviceKey, DeviceMode, DriverError, Family,
    Identity, LinkId, LinkSource, ProgressSink, ReleaseRef, UpdateOutcome,
};
use atlas_driver_orion::{BundleHost, OrionDirectory, OrionTransport};
use atlas_image_server::{ImageServer, ImageServerConfig};
use orion_control_plane::{
    ActionRequest, ActionResult, ActionState, NodeHostFacts, NodeRecord, StatusEntry, StatusQuery,
    StatusSubject, TypedConfigValue,
};
use orion_core::NodeId;
use sha2::{Digest, Sha256};

/// How the fake board's update goes.
#[derive(Clone, Copy, PartialEq)]
enum Ending {
    Confirms,
    RollsBack,
    /// The download fails its checksum while staging.
    FailsStaging,
    /// Staging never finishes (until `update.cancel`).
    Hangs,
}

/// One step of the board's agent: the `update.*` keys it publishes, and
/// whether the board restarts with it (a new boot id from then on).
struct Step {
    keys: Vec<(&'static str, TypedConfigValue)>,
    rebooted: bool,
}

fn text(value: &str) -> TypedConfigValue {
    TypedConfigValue::String(value.into())
}

fn step(rebooted: bool, keys: &[(&'static str, TypedConfigValue)]) -> Step {
    Step {
        keys: keys.to_vec(),
        rebooted,
    }
}

fn script(ending: Ending) -> VecDeque<Step> {
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
struct State {
    nodes: Vec<NodeRecord>,
    status: Vec<StatusEntry>,
    actions: HashMap<String, ActionResult>,
    reject: bool,
    ending: Option<Ending>,
    /// What the board's agent does next, one step per update-key query.
    steps: VecDeque<Step>,
    /// Every action the board received, by name.
    received: Vec<String>,
    /// The last `update` intent, as the board receives it.
    update_request: Option<ActionRequest>,
    /// Stage nothing until the board has downloaded the image.
    hold_for_download: bool,
    downloaded: bool,
}

#[derive(Clone, Default)]
struct FakeOrion(Arc<Mutex<State>>);

const NODE: &str = "raze-node";

fn node(boot_id: &str) -> NodeRecord {
    NodeRecord::builder(NodeId::new(NODE))
        .host(NodeHostFacts {
            board_serial: Some("a317bcbee5226d57\0".into()),
            boot_id: Some(boot_id.into()),
            cpu_count: Some(4),
            ..NodeHostFacts::default()
        })
        .build()
}

fn status(key: &str, value: TypedConfigValue) -> StatusEntry {
    StatusEntry::new(StatusSubject::Node(NodeId::new(NODE)), key, value)
}

impl FakeOrion {
    fn new() -> Self {
        let fake = Self::default();
        fake.0.lock().unwrap().nodes = vec![node("boot-1")];
        fake
    }

    fn set_status(&self, key: &str, value: TypedConfigValue) {
        self.0.lock().unwrap().set(key, value);
    }

    fn received(&self) -> Vec<String> {
        self.0.lock().unwrap().received.clone()
    }
}

impl State {
    fn set(&mut self, key: &str, value: TypedConfigValue) {
        self.status.retain(|entry| entry.key != key);
        self.status.push(status(key, value));
    }

    /// The agent's next step, when the board may go on.
    fn advance(&mut self) {
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

fn result(request: &ActionRequest, state: ActionState) -> ActionResult {
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

struct LocalBundles;

impl BundleHost for LocalBundles {
    fn url_for(&self, artifact: &Artifact, _peer: Option<IpAddr>) -> Result<String, DriverError> {
        Ok(format!("http://atlas.local:7700/bundles/{}", artifact.name))
    }
}

fn raze(board_serial: Option<&str>) -> Identity {
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

fn release() -> ReleaseRef {
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

#[tokio::test]
async fn orion_adds_to_the_device_with_the_same_board_serial() {
    let fake = FakeOrion::new();
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    assert_eq!(directory.refresh().await.unwrap(), 1);

    let matched = directory.capabilities_for(&raze(Some("e5226d57")));
    assert!(matched.telemetry.is_some());
    assert!(matched.actions.is_some());
    assert!(matched.update.is_none(), "no bundle host, no updates");

    assert_eq!(
        directory
            .capabilities_for(&raze(Some("00000000")))
            .kinds()
            .len(),
        1
    );
    assert_eq!(directory.capabilities_for(&raze(None)).kinds().len(), 1);
}

#[tokio::test]
async fn host_metrics_become_readings() {
    let fake = FakeOrion::new();
    fake.set_status(
        "host.temperature.cpu_thermal",
        TypedConfigValue::Int(61_500),
    );
    fake.set_status("host.temperature.rp1_adc", TypedConfigValue::Int(40_000));
    fake.set_status("host.load1_milli", TypedConfigValue::UInt(2_000));
    fake.set_status("host.memory_total_bytes", TypedConfigValue::UInt(4_000));
    fake.set_status("host.memory_available_bytes", TypedConfigValue::UInt(1_000));
    fake.set_status("host.uptime_seconds", TypedConfigValue::UInt(3_600));
    let directory = OrionDirectory::new(Arc::new(fake), None);
    directory.refresh().await.unwrap();

    let telemetry = directory
        .capabilities_for(&raze(Some("e5226d57")))
        .telemetry
        .unwrap();
    let metrics = telemetry.read(&raze(Some("e5226d57"))).await.unwrap();
    let get = |id: &str| metrics.iter().find(|m| m.id == id).map(|m| m.value);
    assert_eq!(get("temp"), Some(61.5));
    assert_eq!(get("cpu"), Some(50.0), "load 2.0 on 4 cores");
    assert_eq!(get("memory"), Some(75.0));
    assert_eq!(get("uptime"), Some(3_600.0));
    assert_eq!(get("temp.rp1_adc"), Some(40.0));
}

#[tokio::test(start_paused = true)]
async fn locate_waits_for_the_device_and_reports_rejections() {
    let fake = FakeOrion::new();
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let actions = directory.capabilities_for(&device).actions.unwrap();

    actions.run_action(&device, "locate").await.unwrap();

    fake.0.lock().unwrap().reject = true;
    let error = actions.run_action(&device, "locate").await.unwrap_err();
    assert!(error.to_string().contains("no handler"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn cancel_and_rollback_are_offered_when_the_agent_handles_updates() {
    let fake = FakeOrion::new();
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let ids = |directory: &OrionDirectory| -> Vec<String> {
        let actions = directory.capabilities_for(&device).actions.unwrap();
        actions.actions(&device).into_iter().map(|a| a.id).collect()
    };
    assert_eq!(
        ids(&directory),
        ["locate", "reboot"],
        "no agent, no update actions"
    );
    let actions = directory.capabilities_for(&device).actions.unwrap();
    assert!(
        actions
            .run_action(&device, "update.rollback")
            .await
            .is_err()
    );

    // The agent publishes update.state: it holds update and its siblings.
    fake.set_status("update.state", text("confirmed"));
    directory.refresh().await.unwrap();
    assert_eq!(
        ids(&directory),
        ["locate", "reboot", "update.cancel", "update.rollback"]
    );
    let actions = directory.capabilities_for(&device).actions.unwrap();
    let rollback = actions
        .actions(&device)
        .into_iter()
        .find(|a| a.id == "update.rollback")
        .unwrap();
    assert!(rollback.destructive, "going back needs a confirm");
    actions
        .run_action(&device, "update.rollback")
        .await
        .unwrap();
    actions.run_action(&device, "update.cancel").await.unwrap();
    assert_eq!(fake.received(), ["update.rollback", "update.cancel"]);
}

async fn update_with(
    ending: Ending,
    cancel: CancellationToken,
) -> (Result<UpdateOutcome, DriverError>, Vec<String>, FakeOrion) {
    let fake = FakeOrion::new();
    fake.0.lock().unwrap().ending = Some(ending);
    fake.set_status("update.state", text("idle"));
    let directory = OrionDirectory::new(Arc::new(fake.clone()), Some(Arc::new(LocalBundles)));
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let updater = directory.capabilities_for(&device).update.unwrap();

    let plan = updater.plan(&device, &release()).unwrap();
    assert_eq!(plan.steps.len(), 5);
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let log = log.clone();
        let cancel = cancel.clone();
        let hangs = ending == Ending::Hangs;
        ProgressSink::new(move |update| {
            let line = format!("{update:?}");
            // Atlas's cancel button, once the transfer shows progress.
            if hangs && line.contains("StepProgress") {
                cancel.cancel();
            }
            log.lock().unwrap().push(line);
        })
    };
    let outcome = updater.run(&device, &release(), &sink, &cancel).await;
    let log = log.lock().unwrap().clone();
    (outcome, log, fake)
}

async fn update(ending: Ending) -> UpdateOutcome {
    let (outcome, log, _) = update_with(ending, CancellationToken::new()).await;
    for step in ["Transfer", "Apply", "Reboot", "Confirm"] {
        assert!(
            log.iter().any(|line| line.contains(step)),
            "{step} reported: {log:?}"
        );
    }
    outcome.unwrap()
}

#[tokio::test(start_paused = true)]
async fn an_update_is_verified_from_durable_state_after_the_reboot() {
    assert_eq!(
        update(Ending::Confirms).await,
        UpdateOutcome::Verified {
            version: "2026.4.0".into()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_rolled_back_update_says_why() {
    assert_eq!(
        update(Ending::RollsBack).await,
        UpdateOutcome::RolledBack {
            reason: "PhotonVision didn't start".into()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_stage_that_fails_on_the_board_fails_the_update() {
    let (outcome, log, _) = update_with(Ending::FailsStaging, CancellationToken::new()).await;
    let error = outcome.unwrap_err();
    assert!(error.to_string().contains("SHA-256"), "{error}");
    assert!(!log.iter().any(|line| line.contains("Reboot")), "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn cancelling_while_staging_cancels_on_the_board() {
    let (outcome, _, fake) = update_with(Ending::Hangs, CancellationToken::new()).await;
    assert!(
        matches!(outcome, Err(DriverError::Cancelled)),
        "{outcome:?}"
    );
    assert_eq!(fake.received(), ["update", "update.cancel"]);
}

#[tokio::test(start_paused = true)]
async fn an_update_already_running_is_refused() {
    let fake = FakeOrion::new();
    fake.set_status("update.state", text("staging"));
    let directory = OrionDirectory::new(Arc::new(fake.clone()), Some(Arc::new(LocalBundles)));
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let updater = directory.capabilities_for(&device).update.unwrap();
    let error = updater
        .run(
            &device,
            &release(),
            &ProgressSink::new(|_| {}),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("already updating"), "{error}");
    assert!(fake.received().is_empty());
}

fn arg(request: &ActionRequest, key: &str) -> TypedConfigValue {
    request
        .args
        .get(key)
        .cloned()
        .unwrap_or_else(|| panic!("no {key} in {request:?}"))
}

/// The whole path: OrionUpdate asks the real image server for a URL, Orion
/// carries it to the board, and the board downloads the image and checks
/// its size and SHA-256 before the update goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_board_downloads_and_checks_the_image_atlas_serves() {
    let dir = std::env::temp_dir().join(format!("atlas-orion-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
    let path = dir.join("photonvision-2026.4.0-raze.img.xz");
    std::fs::write(&path, &image).unwrap();
    let sum: String = Sha256::digest(&image)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let release = ReleaseRef {
        artifact: Some(Artifact {
            name: "photonvision-2026.4.0-raze.img.xz".into(),
            path: path.clone(),
            sha256: sum.clone(),
            size_bytes: image.len() as u64,
        }),
        ..release()
    };

    // Every interface, as in the app: the URL must name the address that
    // routes to the board (loopback here).
    let server = ImageServer::new(ImageServerConfig {
        bind: "0.0.0.0:0".parse().unwrap(),
        ..ImageServerConfig::default()
    });
    let fake = FakeOrion::new();
    fake.0.lock().unwrap().hold_for_download = true;
    fake.set_status("update.state", text("idle"));
    let directory = OrionDirectory::new(Arc::new(fake.clone()), Some(Arc::new(server.clone())));
    directory.refresh().await.unwrap();
    directory.set_update_poll(std::time::Duration::from_millis(20));
    let mut device = raze(Some("e5226d57"));
    device.address = "http://127.0.0.1:5899/.well-known/pd-device".into();
    let updater = directory.capabilities_for(&device).update.unwrap();

    let board = tokio::spawn({
        let fake = fake.clone();
        async move {
            let request = loop {
                if let Some(request) = fake.0.lock().unwrap().update_request.clone() {
                    break request;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            let TypedConfigValue::String(url) = arg(&request, "image_url") else {
                panic!("image_url is not a string");
            };
            let TypedConfigValue::String(expected) = arg(&request, "sha256") else {
                panic!("sha256 is not a string");
            };
            let TypedConfigValue::UInt(size) = arg(&request, "size") else {
                panic!("size is not a number");
            };
            let mut response = reqwest::get(&url).await.unwrap();
            assert!(response.status().is_success(), "{}", response.status());
            let mut hasher = Sha256::new();
            let mut got = 0u64;
            while let Some(chunk) = response.chunk().await.unwrap() {
                got += chunk.len() as u64;
                hasher.update(&chunk);
            }
            let actual: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(got, size);
            assert_eq!(actual, expected);
            fake.0.lock().unwrap().downloaded = true;
            url
        }
    });

    let outcome = updater
        .run(
            &device,
            &release,
            &ProgressSink::new(|_| {}),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        UpdateOutcome::Verified {
            version: "2026.4.0".into()
        }
    );
    let url = board.await.unwrap();
    let port = server.local_addr().unwrap().port();
    assert!(
        url.starts_with(&format!("http://127.0.0.1:{port}/images/")),
        "{url}"
    );
    assert!(url.ends_with("/photonvision-2026.4.0-raze.img.xz"), "{url}");
    assert!(
        directory
            .health()
            .await
            .iter()
            .any(|check| check.id == "orion.image-server")
    );
    let _ = std::fs::remove_dir_all(dir);
}
