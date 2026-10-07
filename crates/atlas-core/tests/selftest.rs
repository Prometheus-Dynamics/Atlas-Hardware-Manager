//! Self-tests: run by hand, kept per board, and run once by themselves when
//! a board comes back from an update.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_core::{
    ActivityKind, ActivityLevel, Atlas, CoreError, JsonFileStore, ReleaseTarget, SelfTestTrigger,
    StagedRollout, UpdateRequest,
};
use atlas_driver::{
    Capabilities, CapabilityKind, CapabilitySource, CheckStatus, DeviceKey, DriverError, Family,
    Identity, SelfTestCapability, SelfTestCheck, SelfTestReport,
};
use atlas_driver_mock::{MockFleet, SIM_HELIOS};

/// A board's self-test: answers with `ok`, or fails the way `next` says.
#[derive(Default)]
struct FakeTest {
    runs: AtomicUsize,
    next: Mutex<Vec<Result<bool, DriverError>>>,
}

#[async_trait::async_trait]
impl SelfTestCapability for FakeTest {
    async fn run_selftest(&self, _device: &Identity) -> Result<SelfTestReport, DriverError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let ok = self.next.lock().unwrap().pop().unwrap_or(Ok(true))?;
        Ok(SelfTestReport {
            version: 1,
            board_serial: Some("10000000a317bcbe".into()),
            model: Some("cm5".into()),
            package_version: Some("1.0.7".into()),
            at: Some(1),
            interactive: false,
            ok,
            checks: vec![SelfTestCheck {
                id: "fan".into(),
                status: if ok {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Fail
                },
                message: "states 0-4".into(),
                data: serde_json::json!({ "max_state": 4 }),
            }],
        })
    }
}

/// Adds the self-test to one device, the way a driver would.
struct WithTest(DeviceKey, Arc<FakeTest>);

impl CapabilitySource for WithTest {
    fn capabilities_for(&self, device: &Identity) -> Capabilities {
        Capabilities {
            selftest: (device.key == self.0).then(|| self.1.clone() as Arc<dyn SelfTestCapability>),
            ..Capabilities::default()
        }
    }
}

fn atlas_with(fleet: &MockFleet, test: Arc<FakeTest>, path: &std::path::Path) -> Atlas {
    let mut builder = Atlas::builder()
        .link_source(fleet.link_source())
        .capability_source(Arc::new(WithTest(rear(), test)))
        .store(Arc::new(JsonFileStore::new(path)));
    for driver in fleet.drivers() {
        builder = builder.driver(driver);
    }
    builder.build().expect("atlas builds")
}

fn rear() -> DeviceKey {
    DeviceKey::new(SIM_HELIOS, "H-1003")
}

fn temp(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "atlas-selftest-{label}-{}.json",
        std::process::id()
    ))
}

async fn update(atlas: &Atlas, key: &DeviceKey) {
    let job = atlas
        .start_update(UpdateRequest {
            devices: vec![key.clone()],
            releases: BTreeMap::from([(
                Family::new(SIM_HELIOS),
                ReleaseTarget::version("2026.3.1"),
            )]),
            staged: StagedRollout::Off,
        })
        .unwrap();
    atlas.wait_job(job).await.unwrap();
}

/// Waits for spawned self-tests to finish.
async fn settle(test: &FakeTest, runs: usize) {
    for _ in 0..100 {
        if test.runs.load(Ordering::SeqCst) >= runs {
            tokio::time::sleep(Duration::from_millis(20)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_selftest_is_kept_shown_in_history_and_remembered() {
    let path = temp("kept");
    let fleet = MockFleet::demo();
    let test = Arc::new(FakeTest::default());
    {
        let atlas = atlas_with(&fleet, test.clone(), &path);
        atlas.scan().await;
        let record = atlas.device(&rear()).unwrap();
        assert!(record.capabilities.contains(&CapabilityKind::SelfTest));

        let run = atlas.run_selftest(&rear()).await.unwrap();
        assert!(run.passed());
        assert_eq!(run.trigger, SelfTestTrigger::Manual);
        assert_eq!(atlas.selftest(&rear()), Some(run));
        let entry = &atlas.activity(1)[0];
        assert_eq!(entry.kind, ActivityKind::SelfTest);
        assert_eq!(entry.level, ActivityLevel::Success);
        assert!(entry.message.contains("passed its self-test"));

        // A device without one says so.
        let front = DeviceKey::new(SIM_HELIOS, "H-1001");
        assert!(matches!(
            atlas.run_selftest(&front).await,
            Err(CoreError::Unsupported { .. })
        ));
    }
    let atlas = atlas_with(&fleet, test, &path);
    assert!(atlas.selftest(&rear()).is_some_and(|run| run.passed()));
    assert_eq!(atlas.selftests().len(), 1);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn a_failed_run_is_kept_too() {
    let path = temp("failed");
    let fleet = MockFleet::demo();
    let test = Arc::new(FakeTest::default());
    let atlas = atlas_with(&fleet, test.clone(), &path);
    atlas.scan().await;

    test.next.lock().unwrap().push(Ok(false));
    let run = atlas.run_selftest(&rear()).await.unwrap();
    assert!(!run.passed());
    assert_eq!(atlas.activity(1)[0].level, ActivityLevel::Warning);

    test.next.lock().unwrap().push(Err(DriverError::Other(
        "the self-test took too long".into(),
    )));
    let run = atlas.run_selftest(&rear()).await.unwrap();
    assert!(run.report.is_none());
    assert_eq!(run.error.as_deref(), Some("the self-test took too long"));
    assert!(atlas.activity(1)[0].message.contains("couldn't run"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn an_updated_board_is_tested_when_it_is_back() {
    let path = temp("after-update");
    let fleet = MockFleet::demo();
    let test = Arc::new(FakeTest::default());
    let atlas = atlas_with(&fleet, test.clone(), &path);
    atlas.scan().await;

    // Not reachable yet (SSH still starting): nothing is recorded, and the
    // next scan tries again.
    test.next
        .lock()
        .unwrap()
        .push(Err(DriverError::Unreachable("connection refused".into())));
    update(&atlas, &rear()).await;
    atlas.scan().await;
    settle(&test, 1).await;
    assert_eq!(test.runs.load(Ordering::SeqCst), 1);
    assert!(atlas.selftest(&rear()).is_none());

    atlas.scan().await;
    settle(&test, 2).await;
    let run = atlas.selftest(&rear()).expect("the retry ran");
    assert_eq!(run.trigger, SelfTestTrigger::AfterUpdate);
    assert!(run.passed());
    assert!(
        atlas
            .activity(5)
            .iter()
            .any(|entry| entry.message.contains("after its update"))
    );

    // Once is enough: later scans don't run it again.
    atlas.scan().await;
    settle(&test, 3).await;
    assert_eq!(test.runs.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_file(path);
}
