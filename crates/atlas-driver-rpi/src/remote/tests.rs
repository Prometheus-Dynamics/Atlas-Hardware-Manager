use super::*;

/// An agent on loopback that knows one job, `job-1`, already finished
/// with `done`, and answers 404 for any other.
async fn agent_with_a_finished_job() -> String {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).await.unwrap_or(0);
            let line = String::from_utf8_lossy(&request[..read]).to_string();
            let (status, body) = if line.starts_with("GET /v1/jobs/job-1") {
                let events = JobEvents {
                    id: "job-1".into(),
                    events: vec![
                        Event::Log {
                            text: "writing".into(),
                            rss_kib: None,
                        },
                        Event::Done {
                            restarted: Some("ttyACM0".into()),
                            restart_error: None,
                        },
                    ],
                    next: 2,
                    finished: true,
                };
                ("200 OK", serde_json::to_string(&events).unwrap())
            } else {
                ("404 Not Found", "{\"error\":\"no such job\"}".to_string())
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        }
    });
    address
}

fn board(agent: &str) -> Identity {
    Identity {
        key: DeviceKey::new(RPI_FAMILY, "e5226d57"),
        model: "Raze".into(),
        mode: DeviceMode::Recovery,
        versions: BTreeMap::new(),
        name: None,
        link: atlas_driver::LinkId("usb".into()),
        address: atlas_usb_agent::board_address(agent, "3-1.2"),
        attributes: BTreeMap::new(),
    }
}

fn release() -> ReleaseRef {
    ReleaseRef {
        family: atlas_driver::Family::new(RPI_FAMILY),
        version: "2026.2.0-dev.811".into(),
        artifact: Some(Artifact {
            name: "helios.img.xz".into(),
            path: "/nowhere/helios.img.xz".into(),
            sha256: "ab".repeat(32),
            size_bytes: 1,
        }),
    }
}

#[tokio::test]
async fn a_flash_an_earlier_atlas_started_is_followed_not_started_again() {
    let agent = agent_with_a_finished_job().await;
    let dir = std::env::temp_dir().join(format!("atlas-agent-jobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let flash = RemoteFlash {
        config: Arc::new(RpiConfig {
            agent_jobs: Some(dir.clone()),
            ..RpiConfig::default()
        }),
    };
    let device = board(&agent);
    assert_eq!(flash.resumable(&device), None);

    // Atlas started job-1, then went away; the record stayed.
    remember(&flash.config, &device, "job-1", &release());
    assert_eq!(flash.resumable(&device), Some(release()));
    // The same board on another port isn't that flash.
    let mut elsewhere = device.clone();
    elsewhere.address = atlas_usb_agent::board_address(&agent, "3-1.4");
    assert_eq!(flash.resumable(&elsewhere), None);

    // Started again: it follows job-1 to its end (the image, which is
    // gone, isn't needed) and forgets it.
    let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = lines.clone();
    let progress = ProgressSink::new(move |update| {
        if let atlas_driver::ProgressUpdate::Log { message } = update {
            seen.lock().unwrap().push(message);
        }
    });
    let outcome = flash
        .run(&device, &release(), &progress, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        UpdateOutcome::Verified {
            version: "2026.2.0-dev.811".into()
        }
    );
    let lines = lines.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("an earlier Atlas started here")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("restarted the board")),
        "{lines:?}"
    );
    assert_eq!(flash.resumable(&device), None, "forgotten once it ended");

    // A job the agent no longer has: said so, forgotten, nothing started.
    remember(&flash.config, &device, "job-9", &release());
    let error = flash
        .run(
            &device,
            &release(),
            &ProgressSink::new(|_| {}),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no longer has job job-9"),
        "{error}"
    );
    assert_eq!(flash.resumable(&device), None);
    let _ = std::fs::remove_dir_all(dir);
}

fn usb(port: &str, product: &str, serial: Option<&str>) -> UsbDevice {
    UsbDevice {
        port: port.into(),
        vendor: "0a5c".into(),
        product: product.into(),
        name: None,
        serial: serial.map(str::to_string),
        kind: UsbDevice::kind_of("0a5c", product),
    }
}

#[test]
fn a_board_behind_the_agent_keeps_its_serial_as_its_key() {
    let config = RpiConfig {
        boot_file_dirs: Vec::new(),
        catalog: Arc::new(atlas_devices::DeviceCatalog::default()),
        ssh_keys: Default::default(),
        known_hosts: None,
        agent_jobs: None,
    };
    let candidate = Candidate {
        link: atlas_driver::LinkId("usb".into()),
        family: atlas_driver::Family::new(RPI_FAMILY),
        address: atlas_usb_agent::board_address("172.27.0.1:5809", "3-1.2"),
    };
    let booting = identity(
        &config,
        &candidate,
        "172.27.0.1:5809",
        &usb("3-1.2", "2712", Some("e5226d57")),
    );
    let gadget = identity(
        &config,
        &candidate,
        "172.27.0.1:5809",
        &usb("3-1.2", "0104", Some("a317bcbee5226d57")),
    );
    assert_eq!(booting.key.to_string(), "rpi:e5226d57");
    assert_eq!(gadget.key, booting.key);
    assert_eq!(booting.mode, DeviceMode::Recovery);
    assert_eq!(booting.attributes["behind"], "172.27.0.1");
    assert_eq!(booting.attributes["usb_port"], "3-1.2");
    assert_eq!(booting.attributes["chip"], "BCM2712 (Pi 5 / CM5)");
    assert_eq!(
        booting.name.as_deref(),
        Some("BCM2712 (Pi 5 / CM5) on USB 3-1.2 behind 172.27.0.1")
    );
    let unnamed = identity(
        &config,
        &candidate,
        "172.27.0.1:5809",
        &usb("3-1.2", "2712", None),
    );
    assert_eq!(unnamed.key.to_string(), "rpi:port-172.27.0.1-3-1.2");
}

#[test]
fn plain_image_names_are_kept() {
    let mut artifact = Artifact {
        name: "helios-2026.2.0-dev.801.img.xz".into(),
        path: "/x".into(),
        sha256: "0123456789abcdef".into(),
        size_bytes: 1,
    };
    assert_eq!(stored_name(&artifact), "helios-2026.2.0-dev.801.img.xz");
    artifact.name = "my image (1).img".into();
    assert_eq!(stored_name(&artifact), "image-0123456789ab");
}
