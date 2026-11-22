//! The Orion side against the simulated node: real pushes on a real session, and the flasher finder.
//!
//! The simulated comm core (`orion-stubs`) says `state = running` always and commits a file with no words, so the
//! write here goes as far as a node without Lemnos's first-flash sink takes it: both files land where the bridge
//! takes them, and the commit is refused for having no fields. The fields and the flasher state are the board
//! step's (`docs/prs/D6.md`, "The board step").
#![cfg(target_os = "linux")]

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_files::PushKind;
use atlas_first_flash::{BridgeFlash, OrionBoard, OrionFlasher, Reach};
use atlas_flash_flow::{FirstFlash, Flasher, ImagePlan, Say, StepError, Stream, Target};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobKind, JobTiming, Jobs};
use atlas_session::{Handle, Pause, Presence, Sessions};
use atlas_sim_node::SimNode;
use orion::Address;

const DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug)]
struct ShortPause;

impl Pause for ShortPause {
    fn pause(&self, delay: Duration, stop: &std::sync::atomic::AtomicBool) {
        let end = Instant::now() + delay.min(Duration::from_millis(10));
        while !stop.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

struct Rig {
    node: SimNode,
    handle: Handle,
    jobs: Jobs,
    kind: Arc<PushKind>,
    _sessions: Sessions,
    _scratch: tempfile::TempDir,
}

async fn rig(name: &str) -> Rig {
    let dir =
        atlas_sim_node::socket::node_dir(name).unwrap_or_else(|error| panic!("{name}: {error}"));
    let _ = std::fs::remove_dir_all(&dir);
    let node = SimNode::start(&dir).expect("the simulated node starts");
    let sessions = Sessions::new(Arc::new(ShortPause));
    let address = Address::from_str(&format!("orion+tcp://{}", node.network())).unwrap();
    let handle = sessions.handle(&address).expect("a handle");
    tokio::time::timeout(
        DEADLINE,
        handle
            .watch()
            .wait_for(|presence| matches!(presence, Presence::Found(_))),
    )
    .await
    .expect("the node is connected")
    .expect("the monitor runs");
    let scratch = tempfile::Builder::new()
        .prefix("jobs-")
        .tempdir_in(Path::new(env!("CARGO_TARGET_TMPDIR")))
        .expect("a scratch directory");
    let kind = PushKind::new();
    let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
    let timing = JobTiming {
        step_timeout: Duration::from_secs(24 * 60 * 60),
        backoff: Backoff {
            first: Duration::from_millis(10),
            cap: Duration::from_secs(5),
        },
    };
    let jobs = Jobs::open(
        scratch.path().join("jobs.json"),
        kinds,
        Events::new(1024),
        timing,
        DeviceHolds::new(),
    )
    .await
    .expect("the jobs open");
    Rig {
        node,
        handle,
        jobs,
        kind,
        _sessions: sessions,
        _scratch: scratch,
    }
}

struct Given(Handle);

#[async_trait]
impl Reach for Given {
    async fn reach(&self, _target: &Target, _say: &Say<'_>) -> Result<Handle, StepError> {
        Ok(self.0.clone())
    }
}

fn target(serial: &str) -> Target {
    Target {
        key: format!("simulated:{serial}"),
        board: "simulated".into(),
        family: "simulated".into(),
        port: "1-1".into(),
    }
}

#[tokio::test]
async fn a_running_node_is_waited_for_and_never_taken_as_the_flasher() {
    let rig = rig("ff-find").await;
    let flasher = OrionFlasher::new(
        Given(rig.handle.clone()),
        rig.kind.clone(),
        rig.jobs.clone(),
    );
    match flasher.find(&target("0123456789abcdef"), &|_| {}).await {
        Err(StepError::Again(words)) => assert!(words.contains("running"), "{words}"),
        Err(other) => panic!("a running node of the target is waited for: {other:?}"),
        Ok(_) => panic!("a running node of the target is waited for, not taken"),
    }
    match flasher.find(&target("fedcba9876543210"), &|_| {}).await {
        Err(StepError::Refused(words)) => assert!(words.contains("0123456789abcdef"), "{words}"),
        Err(other) => panic!("another board is refused: {other:?}"),
        Ok(_) => panic!("another board is refused, not taken"),
    }
}

#[tokio::test]
async fn the_manifest_and_the_image_land_where_the_bridge_takes_them_and_a_bare_commit_is_refused()
{
    let rig = rig("ff-write").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("image.img.xz");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&file, &bytes).unwrap();
    let image = ImagePlan {
        file: file.display().to_string(),
        board: "simulated".into(),
        slot: "simulated-1".into(),
        raw_size: 1 << 20,
        sha256: "5e".repeat(32),
        stream: Stream::Xz,
        signature: None,
    };
    let flash = BridgeFlash::new(OrionBoard::new(
        rig.handle.clone(),
        rig.kind.clone(),
        rig.jobs.clone(),
    ));
    let answer = tokio::time::timeout(DEADLINE, flash.write(&image, &|_| {}, &|_| {}))
        .await
        .expect("the write ends");
    match answer {
        Err(StepError::Refused(words)) => assert!(words.contains("FLASH-BCM2712"), "{words}"),
        other => panic!("a commit with no fields is refused: {other:?}"),
    }
    let committed = rig.node.shelf().committed.lock().unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        committed
            .get("lemnos/flash/image.img.xz.json")
            .expect("the manifest went first"),
    )
    .unwrap();
    assert_eq!(manifest["board"], "simulated");
    assert_eq!(
        committed.get("lemnos/flash/image.img.xz"),
        Some(&bytes),
        "the image is the file, whole"
    );
}
