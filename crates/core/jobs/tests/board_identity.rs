//! A job that needs a board's own identity refuses a node key at submit, names what is missing,
//! and queues nothing. A job that needs no identity takes a node key like any other device.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_jobs::{DeviceClaim, DeviceHolds, Events, JobError, JobKind, JobTiming, Jobs};
use atlas_types::DeviceKey;

/// A kind that flashes boards: it needs a board's identity.
struct Flash;

/// A kind that reads a node: it needs no identity, as Orion's own services need none.
struct Inspect;

#[async_trait]
impl JobKind for Flash {
    fn name(&self) -> &str {
        "flash"
    }

    fn resumable(&self) -> bool {
        false
    }

    fn needs_board_identity(&self) -> bool {
        true
    }

    async fn run_step(&self, _claim: &DeviceClaim, _step: &str) -> Result<(), DriverError> {
        Ok(())
    }
}

#[async_trait]
impl JobKind for Inspect {
    fn name(&self) -> &str {
        "inspect"
    }

    fn resumable(&self) -> bool {
        false
    }

    async fn run_step(&self, _claim: &DeviceClaim, _step: &str) -> Result<(), DriverError> {
        Ok(())
    }
}

async fn open(dir: &std::path::Path) -> Jobs {
    let kinds: Vec<Arc<dyn JobKind>> = vec![Arc::new(Flash), Arc::new(Inspect)];
    Jobs::open(
        &dir.join("jobs.json"),
        kinds,
        Events::new(64),
        JobTiming::default(),
        DeviceHolds::new(),
    )
    .await
    .expect("the job list opens")
}

fn steps() -> Vec<String> {
    vec!["write".to_owned()]
}

#[tokio::test]
async fn a_flash_job_refuses_a_node_key_and_names_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = open(dir.path()).await;
    let node = DeviceKey::node("ab:cd").unwrap();

    let refused = jobs
        .submit("flash", DeviceClaim::writes([node.clone()]), steps())
        .await;
    match refused {
        Err(JobError::NeedsBoard { kind, reason }) => {
            assert_eq!(kind, "flash");
            assert_eq!(reason.fingerprint, "ab:cd");
        }
        other => panic!("a node key is refused for a flash: {other:?}"),
    }
    let text = JobError::NeedsBoard {
        kind: "flash".into(),
        reason: node.board_identity().unwrap_err(),
    }
    .to_string();
    assert!(text.starts_with("flash works on a board"), "{text}");
    assert!(text.contains("ab:cd"), "{text}");
    assert!(jobs.list().is_empty(), "a refused job is not queued");
    jobs.shutdown();
}

#[tokio::test]
async fn a_flash_job_takes_a_board_key_and_an_inspect_takes_a_node_key() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = open(dir.path()).await;

    let board = DeviceKey::from_static("sample-soc", "1000abcd");
    let flashed = jobs
        .submit("flash", DeviceClaim::writes([board]), steps())
        .await;
    assert!(flashed.is_ok(), "{flashed:?}");

    let node = DeviceKey::node("ab:cd").unwrap();
    let read = jobs
        .submit("inspect", DeviceClaim::writes([node]), steps())
        .await;
    assert!(read.is_ok(), "an inspect needs no board identity: {read:?}");
    jobs.shutdown();
}
