//! TEST ONLY: the D6 write step against a flasher reached at a fixed address, for Gaia's board QEMU profile
//! (`docs/board-qemu.md`, "The whole flash"). There the PC reaches the flasher only through the stock host's
//! forward (`127.0.0.1:<port>`), so the board's own address (`SerialReach`) is not routable. This example's `Fixed`
//! reach stands in for it. It is an example, never linked into Atlas: the product path finds a board by its serial
//! only.
//!
//!   cargo run -p atlas-first-flash --example emulated_flash -- \
//!     --address orion+tcp://127.0.0.1:19201 --serial <board serial> --board <board> --slot <slot id> \
//!     --image <raw image> --raw-size <bytes> --sha256 <hex>
//!
//! It finds the flasher by serial (hello.state = flasher, the serial fact), pushes the manifest and the image, reads
//! the write's and the verify's counts and the signature outcome, asks for the restart once, and then watches the
//! same address with a fresh session until a node that is not the flasher answers there (the slot's first boot).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use atlas_files::PushKind;
use atlas_first_flash::{FLASH_DEVICE, FlasherBoard, OrionBoard, OrionFlasher, Reach};
use atlas_flash_flow::{Flasher, ImagePlan, Say, StepError, Stream, Target};
use atlas_jobs::{Backoff, DeviceHolds, Events, JobKind, JobTiming, Jobs};
use atlas_session::{Handle, Pause, Presence, Sessions};
use orion::Address;

#[derive(Debug)]
struct Sleep;

impl Pause for Sleep {
    fn pause(&self, delay: Duration, stop: &std::sync::atomic::AtomicBool) {
        let end = Instant::now() + delay;
        while !stop.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The test-only reach: always the session at the given address.
struct Fixed(Handle);

#[async_trait]
impl Reach for Fixed {
    async fn reach(&self, _target: &Target, _say: &Say<'_>) -> Result<Handle, StepError> {
        Ok(self.0.clone())
    }
}

struct Args {
    address: String,
    serial: String,
    board: String,
    slot: String,
    image: PathBuf,
    raw_size: u64,
    sha256: String,
    scratch: PathBuf,
    call_wait: Option<Duration>,
}

fn args() -> Result<Args, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(key) = it.next() {
        let value = it.next().ok_or_else(|| format!("{key} needs a value"))?;
        map.insert(key, value);
    }
    let mut take = |key: &str| map.remove(key).ok_or_else(|| format!("{key} is required"));
    Ok(Args {
        address: take("--address")?,
        serial: take("--serial")?,
        board: take("--board")?,
        slot: take("--slot")?,
        image: PathBuf::from(take("--image")?),
        raw_size: take("--raw-size")?
            .parse()
            .map_err(|e| format!("--raw-size: {e}"))?,
        sha256: take("--sha256")?,
        scratch: PathBuf::from(take("--scratch")?),
        // The session's call wait, scaled for a guest slower than the board (the runner's measured scale); the
        // product's is atlas_session::CALL_WAIT.
        call_wait: map
            .remove("--call-wait-ms")
            .map(|ms| {
                ms.parse()
                    .map(Duration::from_millis)
                    .map_err(|e| format!("--call-wait-ms: {e}"))
            })
            .transpose()?,
    })
}

fn stamp(start: Instant) -> String {
    format!("[{:7.1} s]", start.elapsed().as_secs_f64())
}

async fn found(handle: &Handle, start: Instant) -> orion::node::Node {
    let mut watch = handle.watch();
    loop {
        match watch.borrow_and_update().clone() {
            Presence::Found(node) => return node,
            other => println!("{} waiting for the node: {other:?}", stamp(start)),
        }
        let _ = tokio::time::timeout(Duration::from_secs(10), watch.changed()).await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let args = args()?;
    let start = Instant::now();
    let say = move |words: String| println!("{} say: {words}", stamp(start));
    let address = Address::from_str(&args.address).map_err(|e| format!("--address: {e}"))?;
    let sessions = match args.call_wait {
        Some(wait) => Sessions::with_call_wait(Arc::new(Sleep), wait),
        None => Sessions::new(Arc::new(Sleep)),
    };
    let handle = sessions.handle(&address).ok_or("no handle")?;
    let node = found(&handle, start).await;
    println!(
        "{} connected: {} state {:?}",
        stamp(start),
        node.name,
        node.state()
    );
    // Every change of the session's presence, with its time: a lapse during the write shows here.
    let mut presence = handle.watch();
    tokio::spawn(async move {
        while presence.changed().await.is_ok() {
            let now = match &*presence.borrow_and_update() {
                Presence::Found(node) => format!("found {}", node.name),
                other => format!("{other:?}"),
            };
            println!("{} PRESENCE: {now}", stamp(start));
        }
    });

    std::fs::create_dir_all(&args.scratch).map_err(|e| e.to_string())?;
    let kind = PushKind::new();
    let kinds: Vec<Arc<dyn JobKind>> = vec![kind.clone()];
    let timing = JobTiming {
        step_timeout: Duration::from_secs(24 * 60 * 60),
        backoff: Backoff {
            first: Duration::from_secs(1),
            cap: Duration::from_secs(5),
        },
    };
    let jobs = Jobs::open(
        args.scratch.join("jobs.json"),
        kinds,
        Events::new(1024),
        timing,
        DeviceHolds::new(),
    )
    .await
    .map_err(|e| format!("jobs: {e}"))?;
    let target = Target {
        key: format!("emulated:{}", args.serial),
        board: args.board.clone(),
        family: "emulated".into(),
        port: "usb-host-forward".into(),
    };
    let board = OrionBoard::new(handle.clone(), Arc::clone(&kind), jobs.clone());
    let info = async || match board.call(FLASH_DEVICE, &["info"], false, &|_, _| {}).await {
        Ok(out) => String::from_utf8_lossy(&out).into_owned(),
        Err(error) => format!("info failed: {error:?}"),
    };
    println!("{} INFO before: {}", stamp(start), info().await);
    let finder = OrionFlasher::new(Fixed(handle.clone()), kind, jobs);
    let mut wait = Duration::from_secs(1);
    let flash = loop {
        match finder.find(&target, &say).await {
            Ok(flash) => break flash,
            Err(StepError::Again(words)) => {
                println!("{} find again in {wait:?}: {words}", stamp(start));
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(5));
            }
            Err(other) => return Err(format!("find: {other:?}")),
        }
    };
    let image = ImagePlan {
        file: args.image.display().to_string(),
        board: args.board.clone(),
        slot: args.slot.clone(),
        raw_size: args.raw_size,
        sha256: args.sha256.clone(),
        stream: Stream::Raw,
        signature: None,
    };
    let progress =
        move |p: atlas_flash_flow::WriteProgress| println!("{} progress: {p:?}", stamp(start));
    let report = match flash.write(&image, &progress, &say).await {
        Ok(report) => report,
        Err(error) => {
            println!("{} INFO after the failure: {}", stamp(start), info().await);
            return Err(format!("write: {error:?}"));
        }
    };
    println!(
        "{} WRITE: written {} verified {} signature: {}",
        stamp(start),
        report.written,
        report.verified,
        report.signature
    );
    flash
        .reboot(&say)
        .await
        .map_err(|e| format!("reboot: {e:?}"))?;
    println!("{} REBOOT asked once", stamp(start));
    drop(flash);
    drop(finder);
    drop(handle);
    drop(sessions);

    // The slot's node has its own key, so it is watched with a fresh session (no pin to the flasher's key).
    let after = match args.call_wait {
        Some(wait) => Sessions::with_call_wait(Arc::new(Sleep), wait),
        None => Sessions::new(Arc::new(Sleep)),
    };
    let deadline = Instant::now() + Duration::from_secs(1800);
    loop {
        let handle = after.handle(&address).ok_or("no handle")?;
        let node = tokio::time::timeout(Duration::from_secs(60), found(&handle, start)).await;
        if let Ok(node) = node {
            let facts: Vec<String> = node
                .facts
                .iter()
                .map(|f| match std::str::from_utf8(&f.value) {
                    Ok(text) => format!("{}={text}", f.name),
                    Err(_) => format!("{}=<{} bytes>", f.name, f.value.len()),
                })
                .collect();
            println!(
                "{} AFTER: {} state {:?} facts {facts:?}",
                stamp(start),
                node.name,
                node.state()
            );
            if node.state() != Some("flasher") {
                return Ok(());
            }
        }
        if Instant::now() > deadline {
            return Err("no slot node answered within 30 min".into());
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
