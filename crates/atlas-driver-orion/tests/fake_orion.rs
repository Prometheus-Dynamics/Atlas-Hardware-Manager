mod support;

use std::sync::{Arc, Mutex};

use atlas_driver::{
    Artifact, CancellationToken, CapabilitySource, DriverError, HardwareDevice, HardwareReading,
    LinkSource, ProgressSink, ReleaseRef, UpdateOutcome,
};
use atlas_driver_orion::OrionDirectory;
use atlas_image_server::{ImageServer, ImageServerConfig};
use orion_control_plane::{ActionRequest, AvailabilityState, HealthState, TypedConfigValue};
use sha2::{Digest, Sha256};
use support::{Ending, FakeOrion, LocalBundles, NODE, device_resource, raze, release, text};

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

#[tokio::test(start_paused = true)]
async fn the_clock_is_set_through_orion_when_the_agent_claims_it() {
    let fake = FakeOrion::new();
    // An agent from before `action.claimed`: update and its siblings only.
    fake.set_status("update.state", text("idle"));
    let directory = OrionDirectory::new(Arc::new(fake.clone()), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let ids = |directory: &OrionDirectory| -> Vec<String> {
        let actions = directory.capabilities_for(&device).actions.unwrap();
        actions.actions(&device).into_iter().map(|a| a.id).collect()
    };
    assert!(!ids(&directory).contains(&"set-clock".to_owned()));

    // board-agent lists what it claims.
    fake.set_status(
        "action.claimed",
        text("update,update.cancel,update.rollback,reboot,locate,clock.set"),
    );
    directory.refresh().await.unwrap();
    assert_eq!(
        ids(&directory),
        [
            "locate",
            "reboot",
            "update.cancel",
            "update.rollback",
            "set-clock"
        ]
    );
    let actions = directory.capabilities_for(&device).actions.unwrap();
    assert!(
        actions.preferred(),
        "Orion goes before SSH for shared actions"
    );
    actions.run_action(&device, "set-clock").await.unwrap();
    assert_eq!(fake.received(), ["clock.set"]);
    let args = fake.0.lock().unwrap().received_args[0].clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    match args.get("unix") {
        Some(TypedConfigValue::Int(sent)) => assert!((sent - now).abs() < 5, "{sent} vs {now}"),
        other => panic!("clock.set needs `unix` as Int: {other:?}"),
    }
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

#[tokio::test]
async fn the_board_hardware_comes_from_the_devices_on_this_node() {
    let fake = FakeOrion::new();
    let labels = |device: &str, class: &str, model: &str| {
        vec![
            "lemnos.board=raze".to_string(),
            format!("lemnos.class={class}"),
            format!("lemnos.model={model}"),
            format!("lemnos.driver={device}"),
        ]
    };
    let mut imu = labels("imu", "imu", "bmi088");
    imu.extend([
        "lemnos.unit.accel_x=m/s²".into(),
        "lemnos.unit.gyro_z=rad/s".into(),
    ]);
    let imu: Vec<&str> = imu.iter().map(String::as_str).collect();
    fake.add_resource(
        device_resource(
            "imu",
            AvailabilityState::Available,
            HealthState::Healthy,
            &imu,
        ),
        NODE,
        &[
            ("status", text("available")),
            ("accel_x", TypedConfigValue::F64(0.12)),
            ("gyro_z", TypedConfigValue::F64(-0.5)),
            ("read_us", TypedConfigValue::UInt(950)),
        ],
    );
    fake.add_resource(
        device_resource(
            "magnetometer",
            AvailabilityState::Unavailable,
            HealthState::Failed,
            &[
                "lemnos.board=raze",
                "lemnos.class=magnetometer",
                "lemnos.model=qmc5883l",
            ],
        ),
        NODE,
        &[
            ("status", text("faulted")),
            ("reason", text("no response on 0x0d")),
        ],
    );
    let mut fan = labels("fan", "fan", "pwmfan");
    fan.extend([
        "lemnos.unit.rpm=rpm".into(),
        "lemnos.control.duty=0..1".into(),
    ]);
    let fan: Vec<&str> = fan.iter().map(String::as_str).collect();
    fake.add_resource(
        device_resource(
            "fan",
            AvailabilityState::Available,
            HealthState::Healthy,
            &fan,
        ),
        NODE,
        &[
            ("status", text("available")),
            ("rpm", TypedConfigValue::UInt(4200)),
            ("control.duty", TypedConfigValue::F64(0.83)),
        ],
    );
    // Another board's device: its provider is on another node.
    fake.add_resource(
        device_resource(
            "gps",
            AvailabilityState::Available,
            HealthState::Healthy,
            &["lemnos.board=raze", "lemnos.class=gps"],
        ),
        "other-node",
        &[("status", text("available"))],
    );

    let directory = OrionDirectory::new(Arc::new(fake), None);
    directory.refresh().await.unwrap();
    let device = raze(Some("e5226d57"));
    let status = directory
        .capabilities_for(&device)
        .status
        .unwrap()
        .status(&device)
        .await
        .unwrap();

    let hardware = status.hardware.expect("the board has hardware");
    assert!(hardware.at > 0);
    let reading = |name: &str, value: f64, unit: &str| HardwareReading {
        name: name.into(),
        value: Some(value),
        unit: unit.into(),
    };
    assert_eq!(
        hardware.devices,
        vec![
            HardwareDevice {
                id: "fan".into(),
                class: "fan".into(),
                model: "pwmfan".into(),
                status: "available".into(),
                reason: None,
                readings: vec![reading("rpm", 4200.0, "rpm")],
                controls: vec!["duty".into()],
            },
            HardwareDevice {
                id: "imu".into(),
                class: "imu".into(),
                model: "bmi088".into(),
                status: "available".into(),
                reason: None,
                readings: vec![
                    reading("accel_x", 0.12, "m/s²"),
                    reading("gyro_z", -0.5, "rad/s"),
                ],
                controls: vec![],
            },
            HardwareDevice {
                id: "magnetometer".into(),
                class: "magnetometer".into(),
                model: "qmc5883l".into(),
                status: "faulted".into(),
                reason: Some("no response on 0x0d".into()),
                readings: vec![],
                controls: vec![],
            },
        ]
    );
}
