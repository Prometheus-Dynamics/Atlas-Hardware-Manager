//! The flow against scripted seams, on tokio's paused clock: every wait is exact, and nothing sleeps for real.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_flash_flow::{
    Control, FirstFlash, Flasher, Flow, FlowEnd, FlowView, ImagePlan, Loaded, Loader, Say,
    SlotWatch, StepError, StepKey, StepState, Stream, Target, WritePhase, WriteProgress,
    WriteReport,
};

fn target() -> Target {
    Target {
        key: "rpi:a317bcbee5226d57".into(),
        board: "raze".into(),
        family: "rpi".into(),
        port: "3-4.2.1.4.4.2".into(),
    }
}

fn image() -> ImagePlan {
    ImagePlan {
        file: "raze-base.img.xz".into(),
        board: "raze".into(),
        slot: "raze-base-2026.1.0-dev.1303-58d8b5d4".into(),
        raw_size: 123_731_968,
        sha256: "eb5777b70a37ce02a1314bc87161d2d8e781cc2d0e0b22430f73668a5d3136ae".into(),
        stream: Stream::Xz,
        signature: None,
    }
}

/// Answers in order; the last answer repeats.
#[derive(Default)]
struct Script<T: Clone> {
    answers: Mutex<VecDeque<Result<T, StepError>>>,
    calls: AtomicU32,
}

impl<T: Clone> Script<T> {
    fn new(answers: Vec<Result<T, StepError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            calls: AtomicU32::new(0),
        }
    }

    fn next(&self) -> Result<T, StepError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            answers.front().unwrap().clone()
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

struct FakeLoader(Script<Loaded>);

#[async_trait]
impl Loader for FakeLoader {
    fn family(&self) -> &str {
        "rpi"
    }
    fn label(&self) -> &str {
        "BCM2712 boot ROM"
    }
    async fn load(&self, _: &Target, say: &Say<'_>) -> Result<Loaded, StepError> {
        say("bootcode5.bin sent".into());
        self.0.next()
    }
}

struct FakeBoard {
    writes: Script<WriteReport>,
    reboots: Script<()>,
    /// The write waits here when set, so a test can act in the middle of it.
    hold: Option<Duration>,
}

#[async_trait]
impl FirstFlash for FakeBoard {
    async fn write(
        &self,
        image: &ImagePlan,
        progress: &(dyn Fn(WriteProgress) + Send + Sync),
        _: &Say<'_>,
    ) -> Result<WriteReport, StepError> {
        for phase in [
            WritePhase::Receiving,
            WritePhase::Writing,
            WritePhase::Verifying,
        ] {
            progress(WriteProgress {
                phase,
                done: image.raw_size / 2,
                total: image.raw_size,
            });
            if let Some(hold) = self.hold {
                tokio::time::sleep(hold).await;
            }
        }
        self.writes.next()
    }
    async fn reboot(&self, _: &Say<'_>) -> Result<(), StepError> {
        self.reboots.next()
    }
}

struct FakeFlasher {
    finds: Script<()>,
    board: Arc<FakeBoard>,
}

#[async_trait]
impl Flasher for FakeFlasher {
    async fn find(&self, _: &Target, _: &Say<'_>) -> Result<Arc<dyn FirstFlash>, StepError> {
        self.finds.next()?;
        Ok(self.board.clone())
    }
}

struct FakeWatch(Script<String>);

#[async_trait]
impl SlotWatch for FakeWatch {
    async fn booted(&self, _: &Target, _: &str, _: &Say<'_>) -> Result<String, StepError> {
        self.0.next()
    }
}

fn loaded() -> Result<Loaded, StepError> {
    Ok(Loaded {
        words: "the flasher started from RAM".into(),
        facts: vec![("Serial".into(), "a317bcbee5226d57".into())],
    })
}

fn report() -> Result<WriteReport, StepError> {
    Ok(WriteReport {
        written: 123_731_968,
        verified: 123_731_968,
        signature: "verified".into(),
    })
}

struct Rig {
    loader: Arc<FakeLoader>,
    flasher: Arc<FakeFlasher>,
    board: Arc<FakeBoard>,
    watch: Arc<FakeWatch>,
    seen: Arc<Mutex<Vec<FlowView>>>,
}

impl Rig {
    fn new(
        loads: Vec<Result<Loaded, StepError>>,
        writes: Vec<Result<WriteReport, StepError>>,
        boots: Vec<Result<String, StepError>>,
    ) -> Self {
        let board = Arc::new(FakeBoard {
            writes: Script::new(writes),
            reboots: Script::new(vec![Ok(())]),
            hold: None,
        });
        Self {
            loader: Arc::new(FakeLoader(Script::new(loads))),
            flasher: Arc::new(FakeFlasher {
                finds: Script::new(vec![Ok(())]),
                board: board.clone(),
            }),
            board,
            watch: Arc::new(FakeWatch(Script::new(boots))),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn holding(mut self, hold: Duration) -> Self {
        let board = Arc::new(FakeBoard {
            writes: Script::new(vec![report()]),
            reboots: Script::new(vec![Ok(())]),
            hold: Some(hold),
        });
        self.flasher = Arc::new(FakeFlasher {
            finds: Script::new(vec![Ok(())]),
            board: board.clone(),
        });
        self.board = board;
        self
    }

    fn flow(&self, target: Target, image: ImagePlan) -> Arc<Flow> {
        let seen = self.seen.clone();
        Arc::new(Flow::new(
            target,
            image,
            self.loader.clone(),
            self.flasher.clone(),
            self.watch.clone(),
            Arc::new(move |view: &FlowView| seen.lock().unwrap().push(view.clone())),
        ))
    }

    fn states(&self, key: StepKey) -> Vec<StepState> {
        let mut out: Vec<StepState> = Vec::new();
        for view in self.seen.lock().unwrap().iter() {
            let state = view.step(key).clone();
            if out.last() != Some(&state) {
                out.push(state);
            }
        }
        out
    }
}

fn booted() -> Result<String, StepError> {
    Ok("the node says slot raze-base-2026.1.0-dev.1303-58d8b5d4".into())
}

#[tokio::test(start_paused = true)]
async fn a_board_is_flashed_through_the_six_steps_in_order() {
    let rig = Rig::new(vec![loaded()], vec![report()], vec![booted()]);
    let flow = rig.flow(target(), image());

    let end = flow.run(&Control::new()).await;

    assert_eq!(
        end,
        FlowEnd::Booted {
            slot: "raze-base-2026.1.0-dev.1303-58d8b5d4".into(),
            words: "the node says slot raze-base-2026.1.0-dev.1303-58d8b5d4".into(),
        }
    );
    let view = flow.view();
    for key in StepKey::ALL {
        assert!(
            matches!(view.step(key), StepState::Done { .. }),
            "{key:?}: {:?}",
            view.step(key)
        );
    }
    // The page saw the board's own phases, in order.
    let phases: Vec<WritePhase> = rig
        .states(StepKey::Write)
        .iter()
        .filter_map(|state| match state {
            StepState::Running {
                progress: Some(progress),
                ..
            } => Some(progress.phase),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        [
            WritePhase::Receiving,
            WritePhase::Writing,
            WritePhase::Verifying
        ]
    );
    assert_eq!(view.written.as_ref().unwrap().verified, 123_731_968);
    assert_eq!(view.loaded.as_ref().unwrap().facts[0].1, "a317bcbee5226d57");
    assert_eq!(rig.board.reboots.calls(), 1);
    assert!(
        view.log
            .iter()
            .any(|line| line.step == StepKey::Loader && line.text == "bootcode5.bin sent")
    );
}

#[tokio::test(start_paused = true)]
async fn an_image_for_another_board_is_refused_before_anything_is_sent() {
    let rig = Rig::new(vec![loaded()], vec![report()], vec![booted()]);
    let mut other = image();
    other.board = "radxa-zero-3e".into();
    let flow = rig.flow(target(), other);

    let end = flow.run(&Control::new()).await;

    assert!(
        matches!(&end, FlowEnd::Refused { step: StepKey::Image, words } if words.contains("radxa-zero-3e")),
        "{end:?}"
    );
    assert_eq!(rig.loader.0.calls(), 0, "nothing reaches the board");
    assert!(matches!(
        flow.view().step(StepKey::Loader),
        StepState::Pending
    ));
}

#[tokio::test(start_paused = true)]
async fn a_board_of_another_family_is_refused_before_anything_is_sent() {
    let rig = Rig::new(vec![loaded()], vec![report()], vec![booted()]);
    let mut other = target();
    other.family = "rockchip".into();
    let end = rig.flow(other, image()).run(&Control::new()).await;
    assert!(
        matches!(
            end,
            FlowEnd::Refused {
                step: StepKey::Board,
                ..
            }
        ),
        "{end:?}"
    );
    assert_eq!(rig.loader.0.calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_failure_that_can_change_is_tried_again_after_a_wait_that_doubles_to_five_seconds() {
    let again = || Err(StepError::Again("no answer from the boot ROM".into()));
    let rig = Rig::new(
        vec![again(), again(), again(), again(), again(), loaded()],
        vec![report()],
        vec![booted()],
    );
    let flow = rig.flow(target(), image());
    let start = tokio::time::Instant::now();

    let end = flow.run(&Control::new()).await;

    assert!(matches!(end, FlowEnd::Booted { .. }));
    assert_eq!(rig.loader.0.calls(), 6);
    let waits: Vec<u64> = rig
        .states(StepKey::Loader)
        .iter()
        .filter_map(|state| match state {
            StepState::Retrying { retry_in_ms, .. } => Some(*retry_in_ms),
            _ => None,
        })
        .collect();
    assert_eq!(waits, [1000, 2000, 4000, 5000, 5000]);
    assert_eq!(start.elapsed(), Duration::from_millis(17_000));
}

#[tokio::test(start_paused = true)]
async fn a_refusal_waits_for_the_person_and_is_never_retried_by_itself() {
    let rig = Rig::new(
        vec![loaded()],
        vec![
            Err(StepError::Refused(
                "the board refuses the image: unknown key".into(),
            )),
            report(),
        ],
        vec![booted()],
    );
    let flow = rig.flow(target(), image());
    let control = Control::new();
    let run = tokio::spawn({
        let (flow, control) = (flow.clone(), control.clone());
        async move { flow.run(&control).await }
    });

    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        rig.board.writes.calls(),
        1,
        "ten minutes later, still one try"
    );
    assert!(matches!(
        flow.view().step(StepKey::Write),
        StepState::Refused { attempt: 1, .. }
    ));

    control.retry_now();
    let end = run.await.unwrap();
    assert!(matches!(end, FlowEnd::Booted { .. }));
    assert_eq!(rig.board.writes.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn stopped_retries_wait_for_the_person_then_go_on() {
    let again = || {
        Err(StepError::Again(
            "the flasher's node has not answered".into(),
        ))
    };
    let rig = Rig::new(
        vec![again(), again(), loaded()],
        vec![report()],
        vec![booted()],
    );
    let flow = rig.flow(target(), image());
    let control = Control::new();
    let run = tokio::spawn({
        let (flow, control) = (flow.clone(), control.clone());
        async move { flow.run(&control).await }
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    control.stop();
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert_eq!(rig.loader.0.calls(), 1, "no try while stopped");
    assert!(matches!(
        flow.view().step(StepKey::Loader),
        StepState::Stopped { attempt: 1, .. }
    ));

    control.retry_now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(rig.loader.0.calls(), 2, "try again now tries at once");
    let end = run.await.unwrap();
    assert!(matches!(end, FlowEnd::Booted { .. }));
    assert_eq!(rig.loader.0.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn try_again_now_cuts_the_wait_short() {
    let again = || Err(StepError::Again("no answer".into()));
    let rig = Rig::new(vec![again(), loaded()], vec![report()], vec![booted()]);
    let flow = rig.flow(target(), image());
    let control = Control::new();
    let start = tokio::time::Instant::now();
    let run = tokio::spawn({
        let (flow, control) = (flow.clone(), control.clone());
        async move { flow.run(&control).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    control.retry_now();
    run.await.unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(rig.loader.0.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn cancel_during_the_write_ends_the_flow_there_and_restarts_nothing() {
    let rig =
        Rig::new(vec![loaded()], vec![report()], vec![booted()]).holding(Duration::from_secs(30));
    let flow = rig.flow(target(), image());
    let control = Control::new();
    let run = tokio::spawn({
        let (flow, control) = (flow.clone(), control.clone());
        async move { flow.run(&control).await }
    });
    tokio::time::sleep(Duration::from_secs(40)).await;
    control.cancel();
    let end = run.await.unwrap();
    assert_eq!(
        end,
        FlowEnd::Cancelled {
            step: StepKey::Write
        }
    );
    assert!(matches!(
        flow.view().step(StepKey::Write),
        StepState::Cancelled
    ));
    assert_eq!(
        rig.board.reboots.calls(),
        0,
        "a cancelled write never restarts the board"
    );
}

#[tokio::test(start_paused = true)]
async fn the_restart_is_asked_once_while_the_slot_is_watched_again() {
    let again = || Err(StepError::Again("the board has not come back yet".into()));
    let rig = Rig::new(
        vec![loaded()],
        vec![report()],
        vec![again(), again(), booted()],
    );
    let end = rig.flow(target(), image()).run(&Control::new()).await;
    assert!(matches!(end, FlowEnd::Booted { .. }));
    assert_eq!(rig.board.reboots.calls(), 1);
    assert_eq!(rig.watch.0.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn the_page_reads_the_view_as_json_with_the_step_keys() {
    let rig = Rig::new(vec![loaded()], vec![report()], vec![booted()]);
    let flow = rig.flow(target(), image());
    flow.run(&Control::new()).await;
    let json = serde_json::to_value(flow.view()).unwrap();
    let keys: Vec<&str> = json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        ["image", "board", "loader", "flasher", "write", "boot"]
    );
    assert_eq!(json["steps"][4]["state"], "done");
    assert_eq!(json["end"]["end"], "booted");
    assert_eq!(json["boot_rom"], "BCM2712 boot ROM");
}
