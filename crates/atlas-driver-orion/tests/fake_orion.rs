use std::collections::{BTreeMap, HashMap};
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

/// How the fake device answers an `update` action.
#[derive(Clone, Copy, PartialEq)]
enum Ending {
    Confirms,
    RollsBack,
}

#[derive(Default)]
struct State {
    nodes: Vec<NodeRecord>,
    status: Vec<StatusEntry>,
    actions: HashMap<String, ActionResult>,
    polls: HashMap<String, u32>,
    reject: bool,
    ending: Option<Ending>,
    /// The last `update` intent, as the board receives it.
    update_request: Option<ActionRequest>,
    /// Keep the update running until the board has downloaded the image.
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
        let mut state = self.0.lock().unwrap();
        state.status.retain(|entry| entry.key != key);
        state.status.push(status(key, value));
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
        Ok(self
            .0
            .lock()
            .unwrap()
            .status
            .iter()
            .filter(|entry| entry.key.starts_with(&prefix))
            .cloned()
            .collect())
    }

    async fn run_action(&self, request: ActionRequest) -> Result<ActionResult, DriverError> {
        let mut state = self.0.lock().unwrap();
        let answer = if state.reject {
            result(
                &request,
                ActionState::Rejected {
                    reason: "no handler for locate".into(),
                },
            )
        } else {
            result(
                &request,
                ActionState::Running {
                    progress: Some(250),
                },
            )
        };
        if request.name == "update" {
            state.update_request = Some(request.clone());
        }
        state
            .actions
            .insert(request.action_id.clone(), answer.clone());
        Ok(answer)
    }

    async fn query_action(&self, action_id: &str) -> Result<Option<ActionResult>, DriverError> {
        let mut guard = self.0.lock().unwrap();
        let state = &mut *guard;
        let polls = state.polls.entry(action_id.into()).or_default();
        *polls += 1;
        let Some(current) = state.actions.get_mut(action_id) else {
            return Ok(None);
        };
        if current.name == "update" && state.hold_for_download && !state.downloaded {
            return Ok(Some(current.clone()));
        }
        if *polls >= 2 && matches!(current.state, ActionState::Running { .. }) {
            current.state = ActionState::Succeeded;
            if current.name == "update" {
                current.output.insert(
                    "version_staged".into(),
                    TypedConfigValue::String("2026.4.0".into()),
                );
                // The board reboots into the new slot and reports back.
                state.nodes = vec![node("boot-2")];
                let ending = state.ending.unwrap_or(Ending::Confirms);
                let entries = match ending {
                    Ending::Confirms => vec![
                        status("update.state", TypedConfigValue::String("confirmed".into())),
                        status(
                            "update.version_active",
                            TypedConfigValue::String("2026.4.0".into()),
                        ),
                    ],
                    Ending::RollsBack => vec![
                        status(
                            "update.state",
                            TypedConfigValue::String("rolled-back".into()),
                        ),
                        status(
                            "update.error",
                            TypedConfigValue::String("PhotonVision didn't start".into()),
                        ),
                    ],
                };
                state
                    .status
                    .retain(|entry| !entry.key.starts_with("update."));
                state.status.extend(entries);
            }
        } else if *polls == 1 && matches!(current.state, ActionState::Running { .. }) {
            current.state = ActionState::Running {
                progress: Some(750),
            };
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

async fn update(ending: Ending) -> UpdateOutcome {
    let fake = FakeOrion::new();
    fake.0.lock().unwrap().ending = Some(ending);
    fake.set_status("update.state", TypedConfigValue::String("idle".into()));
    let directory = OrionDirectory::new(Arc::new(fake), Some(Arc::new(LocalBundles)));
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let updater = directory.capabilities_for(&device).update.unwrap();

    let plan = updater.plan(&device, &release()).unwrap();
    assert_eq!(plan.steps.len(), 5);
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let log = log.clone();
        ProgressSink::new(move |update| log.lock().unwrap().push(format!("{update:?}")))
    };
    let outcome = updater
        .run(&device, &release(), &sink, &CancellationToken::new())
        .await
        .unwrap();
    let log = log.lock().unwrap();
    for step in ["Transfer", "Apply", "Reboot", "Confirm"] {
        assert!(
            log.iter().any(|line| line.contains(step)),
            "{step} reported: {log:?}"
        );
    }
    outcome
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
    fake.set_status("update.state", TypedConfigValue::String("idle".into()));
    let directory = OrionDirectory::new(Arc::new(fake.clone()), Some(Arc::new(server.clone())));
    directory.refresh().await.unwrap();
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
