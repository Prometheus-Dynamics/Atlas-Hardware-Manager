//! Jobs: a job survives an Atlas restart, a stuck job holds up no other, and
//! failing steps are retried until they succeed.
//!
//! These run on real time: the job list is written to disk, and a paused clock
//! would advance past the waits while the writes are in flight.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_driver::DriverError;
use atlas_jobs::{
    Backoff, Event, Events, JobError, JobId, JobKind, JobRecord, JobStatus, JobTiming, Jobs,
};
use atlas_types::DeviceKey;
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokio::time;

/// Records each step it runs. `hang` names a step that never finishes, and
/// `failures` makes the first calls fail.
struct Kind {
    name: &'static str,
    resumable: bool,
    hang: Option<&'static str>,
    failures: Mutex<u32>,
    log: Arc<Mutex<Vec<String>>>,
}

impl Kind {
    fn arc(
        name: &'static str,
        resumable: bool,
        hang: Option<&'static str>,
        failures: u32,
        log: &Arc<Mutex<Vec<String>>>,
    ) -> Arc<dyn JobKind> {
        Arc::new(Self {
            name,
            resumable,
            hang,
            failures: Mutex::new(failures),
            log: Arc::clone(log),
        })
    }
}

#[async_trait]
impl JobKind for Kind {
    fn name(&self) -> &str {
        self.name
    }

    fn resumable(&self) -> bool {
        self.resumable
    }

    async fn run_step(&self, _device: Option<&DeviceKey>, step: &str) -> Result<(), DriverError> {
        self.log.lock().unwrap().push(step.to_owned());
        if self.hang == Some(step) {
            std::future::pending::<()>().await;
        }
        let fail = {
            let mut left = self.failures.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                true
            } else {
                false
            }
        };
        if fail {
            return Err(DriverError::Unreachable(
                "the device is not answering".into(),
            ));
        }
        Ok(())
    }
}

fn steps(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

fn log() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

async fn open(path: &Path, kinds: Vec<Arc<dyn JobKind>>, events: &Events) -> Jobs {
    Jobs::open(path, kinds, events.clone(), JobTiming::default())
        .await
        .expect("the job list opens")
}

/// Waits for a change to job `id` that `wanted` accepts, read from the events
/// (each one is sent after its save).
async fn wait_for_job(
    rx: &mut Receiver<Event>,
    id: JobId,
    wanted: impl Fn(&JobRecord) -> bool,
) -> JobRecord {
    time::timeout(Duration::from_secs(30), async {
        loop {
            match rx.recv().await {
                Ok(Event::Job { job }) if job.id == id && wanted(&job) => return job,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("event stream closed"),
            }
        }
    })
    .await
    .expect("the job did not reach the expected state")
}

#[tokio::test]
async fn a_job_survives_a_restart_and_finished_steps_do_not_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.json");
    let device = DeviceKey::new("sample-soc", "1000abcd");

    // First Atlas: step "a" finishes and is saved; step "b" hangs. Then Atlas stops.
    let events = Events::new(256);
    let mut rx = events.subscribe();
    let first = log();
    let jobs = open(
        &path,
        vec![Kind::arc("resumable-kind", true, Some("b"), 0, &first)],
        &events,
    )
    .await;
    let id = jobs
        .submit(
            "resumable-kind",
            Some(device.clone()),
            steps(&["a", "b", "c"]),
        )
        .await
        .unwrap();
    wait_for_job(&mut rx, id, |job| job.completed == 1).await;
    jobs.shutdown();

    // Second Atlas reads the same file and carries the job on from step "b".
    let events = Events::new(256);
    let mut rx = events.subscribe();
    let second = log();
    let jobs = open(
        &path,
        vec![Kind::arc("resumable-kind", true, None, 0, &second)],
        &events,
    )
    .await;
    let job = wait_for_job(&mut rx, id, |job| job.status == JobStatus::Done).await;

    assert_eq!(job.completed, 3);
    assert_eq!(job.device, Some(device));
    assert_eq!(*second.lock().unwrap(), vec!["b", "c"], "step a ran again");
    assert_eq!(jobs.get(id).unwrap().status, JobStatus::Done);
    jobs.shutdown();
}

#[tokio::test]
async fn a_job_that_cannot_resume_is_marked_interrupted_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.json");

    let events = Events::new(256);
    let mut rx = events.subscribe();
    let first = log();
    let jobs = open(
        &path,
        vec![Kind::arc("one-shot-kind", false, Some("b"), 0, &first)],
        &events,
    )
    .await;
    let id = jobs
        .submit("one-shot-kind", None, steps(&["a", "b", "c"]))
        .await
        .unwrap();
    wait_for_job(&mut rx, id, |job| job.completed == 1).await;
    jobs.shutdown();

    let events = Events::new(256);
    let second = log();
    let jobs = open(
        &path,
        vec![Kind::arc("one-shot-kind", false, None, 0, &second)],
        &events,
    )
    .await;
    let job = jobs.get(id).expect("the interrupted job is still listed");
    match &job.status {
        JobStatus::Interrupted { reason } => {
            assert!(reason.contains("cannot continue"), "{reason}")
        }
        other => panic!("expected interrupted, got {other:?}"),
    }
    assert!(
        second.lock().unwrap().is_empty(),
        "an interrupted job ran again"
    );
}

#[tokio::test]
async fn a_stuck_job_holds_up_no_other_job() {
    let dir = tempfile::tempdir().unwrap();
    let events = Events::new(256);
    let mut rx = events.subscribe();
    let shared = log();
    let jobs = open(
        &dir.path().join("jobs.json"),
        vec![Kind::arc("mixed-kind", true, Some("stuck"), 0, &shared)],
        &events,
    )
    .await;

    let stuck = jobs
        .submit("mixed-kind", None, steps(&["stuck"]))
        .await
        .unwrap();
    let quick = jobs
        .submit("mixed-kind", None, steps(&["go"]))
        .await
        .unwrap();
    wait_for_job(&mut rx, quick, |job| job.status == JobStatus::Done).await;
    assert_eq!(jobs.get(stuck).unwrap().status, JobStatus::Running);

    assert!(jobs.cancel(stuck));
    wait_for_job(&mut rx, stuck, |job| job.status == JobStatus::Cancelled).await;
    jobs.shutdown();
}

#[tokio::test]
async fn a_failing_step_is_retried_until_it_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let events = Events::new(256);
    let mut rx = events.subscribe();
    let calls = log();
    let timing = JobTiming {
        step_timeout: Duration::from_secs(60),
        backoff: Backoff {
            first: Duration::from_millis(1),
            cap: Duration::from_millis(5),
        },
    };
    let jobs = Jobs::open(
        dir.path().join("jobs.json"),
        vec![Kind::arc("flaky-kind", true, None, 3, &calls)],
        events.clone(),
        timing,
    )
    .await
    .unwrap();

    let id = jobs
        .submit("flaky-kind", None, steps(&["only"]))
        .await
        .unwrap();
    let job = wait_for_job(&mut rx, id, |job| job.status == JobStatus::Done).await;
    assert_eq!(job.completed, 1);
    assert_eq!(
        calls.lock().unwrap().len(),
        4,
        "three failures, then one success"
    );
    jobs.shutdown();
}

#[tokio::test]
async fn submit_refuses_an_unknown_kind_and_an_empty_job() {
    let dir = tempfile::tempdir().unwrap();
    let events = Events::new(256);
    let jobs = open(
        &dir.path().join("jobs.json"),
        vec![Kind::arc("known-kind", true, None, 0, &log())],
        &events,
    )
    .await;
    assert_eq!(
        jobs.submit("missing-kind", None, steps(&["a"])).await,
        Err(JobError::UnknownKind("missing-kind".into()))
    );
    assert_eq!(
        jobs.submit("known-kind", None, Vec::new()).await,
        Err(JobError::NoSteps)
    );
    assert!(jobs.list().is_empty());
}
