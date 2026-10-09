use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

fn package() -> DevicePackage {
    DevicePackage {
        manifest: serde_json::from_str(
            r#"{ "contract": 1, "model": "raze", "display_name": "Raze" }"#,
        )
        .unwrap(),
        compat: Default::default(),
        dir: std::path::PathBuf::from("/nonexistent"),
    }
}

/// Serves one HTTP response with `body`, then closes.
async fn serve_once(body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    format!("http://{address}/.well-known/pd-device")
}

#[tokio::test]
async fn identify_reads_the_identity_endpoint() {
    let url = serve_once(
        r#"{ "contract": 1, "model": "Raze", "serial": "ABC", "os": { "name": "helios", "version": "2026.3.1" } }"#,
    )
    .await;
    let driver = BoardDriver::new(package());

    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();

    assert_eq!(identity.key.to_string(), "raze:abc");
    assert_eq!(identity.model, "Raze");
    assert_eq!(identity.attributes["os"], "helios");
}

#[tokio::test]
async fn listed_endpoints_become_capabilities() {
    let url = serve_once(
        r#"{ "model": "raze", "serial": "1",
             "endpoints": { "metrics": "/api/metrics", "actions": "/api/actions" },
             "actions": [ { "id": "locate", "label": "Find it" } ],
             "camera_stream": "/stream.mjpg" }"#,
    )
    .await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url.clone(),
        })
        .await
        .unwrap();

    let capabilities = driver.capabilities(&identity);
    assert!(capabilities.telemetry.is_some());
    assert!(capabilities.logs.is_none());
    let actions = capabilities.actions.unwrap().actions(&identity);
    assert_eq!(actions[0].label, "Find it");
    assert!(identity.attributes["camera_stream"].ends_with("/stream.mjpg"));
}

#[tokio::test]
async fn a_plain_device_offers_no_extras() {
    let url = serve_once(r#"{ "model": "raze", "serial": "2" }"#).await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let capabilities = driver.capabilities(&identity);
    assert_eq!(
        capabilities.kinds(),
        vec![atlas_driver::CapabilityKind::Info]
    );
}

#[tokio::test]
async fn an_ab_board_updates_over_ssh() {
    let url = serve_once(
        r#"{ "model": "raze", "serial": "3", "update_methods": ["image-write", "ab-tryboot"] }"#,
    )
    .await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let update = driver.capabilities(&identity).update.unwrap();
    let release = |name: &str| atlas_driver::ReleaseRef {
        family: Family::new("raze"),
        version: "2.0".into(),
        artifact: Some(atlas_driver::Artifact {
            name: name.into(),
            path: name.into(),
            sha256: String::new(),
            size_bytes: 1,
        }),
    };
    assert!(update.plan(&identity, &release("pv-raze.img.xz")).is_ok());
    assert!(matches!(
        update.plan(&identity, &release("pv.pdupdate")),
        Err(DriverError::Incompatible(_))
    ));
}

#[tokio::test]
async fn a_board_that_can_restart_into_usb_boot_offers_it() {
    let url = serve_once(
        r#"{ "model": "raze", "serial": "4",
             "update_methods": ["image-write", "ab-tryboot", "usb-boot-reboot"] }"#,
    )
    .await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let actions = driver.capabilities(&identity).actions.unwrap();
    let usb_boot = actions
        .actions(&identity)
        .into_iter()
        .find(|action| action.id == crate::USB_BOOT_ACTION)
        .unwrap();
    assert!(usb_boot.destructive);
}

fn gadget_candidate(driver: &BoardDriver, probe: GadgetProbe) -> Candidate {
    let address = probe.url.clone();
    driver.probes.lock().unwrap().insert(address.clone(), probe);
    Candidate {
        link: LinkId(crate::gadget::LINK_ID.into()),
        family: Family::new("raze"),
        address,
    }
}

#[tokio::test]
async fn a_usb_gadget_board_must_report_its_usb_serial() {
    let driver = BoardDriver::new(package());
    let mine = serve_once(r#"{ "model": "raze", "serial": "a317bcbee5226d57" }"#).await;
    let candidate = gadget_candidate(
        &driver,
        GadgetProbe {
            url: mine,
            fallback: None,
            serial: Some("e5226d57".into()),
        },
    );
    let identity = driver.identify(&candidate).await.unwrap();
    assert_eq!(identity.key.to_string(), "raze:a317bcbee5226d57");

    let someone_else = serve_once(r#"{ "model": "raze", "serial": "10000000abcdef01" }"#).await;
    let candidate = gadget_candidate(
        &driver,
        GadgetProbe {
            url: someone_else,
            fallback: None,
            serial: Some("e5226d57".into()),
        },
    );
    assert!(matches!(
        driver.identify(&candidate).await,
        Err(DriverError::Unreachable(_))
    ));
}

#[tokio::test]
async fn an_image_without_per_board_addressing_answers_at_the_fixed_address() {
    let driver = BoardDriver::new(package());
    let legacy = serve_once(r#"{ "model": "raze", "serial": "a317bcbee5226d57" }"#).await;
    let candidate = gadget_candidate(
        &driver,
        GadgetProbe {
            // Nothing listens on port 1.
            url: "http://127.0.0.1:1/.well-known/pd-device".into(),
            fallback: Some(legacy.clone()),
            serial: Some("e5226d57".into()),
        },
    );
    let identity = driver.identify(&candidate).await.unwrap();
    assert_eq!(identity.address, legacy);
}

#[tokio::test]
async fn a_board_with_a_wrong_clock_offers_to_set_it() {
    let url = serve_once(r#"{ "model": "raze", "serial": "5", "time": 1773360000 }"#).await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let offset: i64 = identity.attributes["clock_offset_s"].parse().unwrap();
    assert!(offset < -1_000_000, "months behind: {offset}");
    let actions = driver
        .capabilities(&identity)
        .actions
        .unwrap()
        .actions(&identity);
    assert!(
        actions
            .iter()
            .any(|a| a.id == crate::SET_CLOCK_ACTION && !a.destructive)
    );
    assert!(!actions.iter().any(|a| a.id == crate::USB_BOOT_ACTION));
}

#[tokio::test]
async fn a_board_with_the_selftest_offers_it() {
    let url =
        serve_once(r#"{ "model": "raze", "serial": "5", "diagnostics": ["selftest"] }"#).await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let capabilities = driver.capabilities(&identity);
    assert!(capabilities.selftest.is_some());
    assert!(
        capabilities
            .kinds()
            .contains(&atlas_driver::CapabilityKind::SelfTest)
    );
}

#[tokio::test]
async fn a_different_model_is_rejected() {
    let url = serve_once(r#"{ "model": "other", "serial": "1" }"#).await;
    let driver = BoardDriver::new(package());

    let result = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await;

    assert!(matches!(result, Err(DriverError::Other(_))));
}

/// Serves each request by its path (query ignored) from `routes`, and
/// records the request lines.
async fn serve_paths(
    routes: &'static [(&'static str, &'static str)],
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0u8; 2048];
            let read = socket.read(&mut request).await.unwrap_or(0);
            let line = String::from_utf8_lossy(&request[..read])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            let target = line.split(' ').nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(target.clone());
            let path = target.split('?').next().unwrap_or("");
            let body = routes
                .iter()
                .find(|(route, _)| *route == path)
                .map_or("{}", |(_, body)| *body);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (format!("http://{address}/.well-known/pd-device"), seen)
}

#[tokio::test]
async fn a_board_with_status_and_events_reports_them() {
    let (url, seen) = serve_paths(&[
        (
            "/.well-known/pd-device",
            r#"{ "model": "raze", "serial": "6", "update_methods": ["image-write", "ab-tryboot"],
                 "endpoints": { "status": "/status", "events": "/events", "actions": "/actions" },
                 "actions": [ { "id": "locate", "label": "Find it" } ] }"#,
        ),
        (
            "/status",
            r#"{ "version": 1, "time": 100, "failed_units": ["x.service"],
                 "temperatures": [ { "id": "cpu-thermal", "celsius": 50.5 } ],
                 "fan": { "state": 1, "max_state": 4, "pwm": 179, "rpm": null },
                 "update": { "state": "staged", "version_staged": "2.0", "progress": 1000, "started_by": "local" } }"#,
        ),
        (
            "/events",
            r#"{ "events": [ { "t": 5, "boot_id": "b", "kind": "update.staged", "source": "local",
                               "message": "staged 2.0 in slot B", "data": { "slot": "B" } } ] }"#,
        ),
    ])
    .await;
    let driver = BoardDriver::new(package());
    let identity = driver
        .identify(&Candidate {
            link: LinkId("mdns".into()),
            family: Family::new("raze"),
            address: url,
        })
        .await
        .unwrap();
    let caps = driver.capabilities(&identity);
    assert!(caps.kinds().contains(&atlas_driver::CapabilityKind::Status));

    let status = caps
        .status
        .as_ref()
        .unwrap()
        .status(&identity)
        .await
        .unwrap();
    assert_eq!(status.failed_units, ["x.service"]);
    let update = status.update.unwrap();
    assert_eq!(update.version_staged.as_deref(), Some("2.0"));
    assert_eq!(update.started_by, Some(atlas_driver::EventSource::Local));

    let events = caps
        .status
        .as_ref()
        .unwrap()
        .events(&identity, Some(4), 50)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "update.staged");
    assert!(
        seen.lock()
            .unwrap()
            .contains(&"/events?since=4&limit=50".to_string())
    );

    // No metrics endpoint: the status is the telemetry.
    let metrics = caps.telemetry.unwrap().read(&identity).await.unwrap();
    assert_eq!(metrics[0].id, "temp");
    assert_eq!(metrics[0].value, 50.5);

    // The general controls come over SSH; locate stays the endpoint's.
    let ids: Vec<String> = caps
        .actions
        .unwrap()
        .actions(&identity)
        .into_iter()
        .map(|a| a.id)
        .collect();
    for id in [
        "locate",
        crate::REBOOT_ACTION,
        crate::POWER_OFF_ACTION,
        crate::UPDATE_CANCEL_ACTION,
        crate::UPDATE_ROLLBACK_ACTION,
    ] {
        assert!(ids.iter().any(|have| have == id), "{id} in {ids:?}");
    }
}
