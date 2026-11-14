//! The agent as a service, on loopback, without a board: its info, uploads
//! checked by SHA-256, and a flash job that runs, ends with why it couldn't
//! flash, and can be read again from any point.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use atlas_usb_agent::{AgentInfo, BootOutcome, Event, JobEvents, JobStarted, Stored};

struct Agent {
    child: Child,
    address: SocketAddr,
    state: std::path::PathBuf,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

fn start() -> Agent {
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    // Unique per test: the tests run in parallel, and each one's state is
    // removed when its agent drops.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let state = std::env::temp_dir().join(format!(
        "atlas-usb-agent-test-{}-{stamp}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&state);
    let child = Command::new(env!("CARGO_BIN_EXE_atlas-usb-agent"))
        .args(["serve", "--listen", &address.to_string(), "--state"])
        .arg(&state)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline, "the agent didn't start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Agent {
        child,
        address,
        state,
    }
}

/// One HTTP/1.1 request; the status and body.
fn request(
    agent: &Agent,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, String) {
    let mut stream = TcpStream::connect(agent.address).unwrap();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nhost: agent\r\nconnection: close\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    let status = reply[9..12].parse().unwrap();
    let body = reply
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn uploads_jobs_and_their_events() {
    let agent = start();
    let (status, body) = request(&agent, "GET", "/v1/agent", &[], b"");
    assert_eq!(status, 200);
    let info: AgentInfo = serde_json::from_str(&body).unwrap();
    assert!(!info.boot_files);
    assert_eq!(info.running, None);

    // An image needs its SHA-256, and must match it.
    let image = vec![7u8; 3 << 20];
    let digest = sha256(&image);
    let (status, _) = request(&agent, "PUT", "/v1/images/test.img", &[], b"x");
    assert_eq!(status, 400);
    let wrong = "0".repeat(64);
    let (status, body) = request(
        &agent,
        "PUT",
        "/v1/images/test.img",
        &[("x-sha256", &wrong)],
        &image,
    );
    assert_eq!(status, 400, "{body}");
    assert!(!agent.state.join("images/test.img").exists());
    let (status, body) = request(
        &agent,
        "PUT",
        "/v1/images/test.img",
        &[("x-sha256", &digest)],
        &image,
    );
    assert_eq!(status, 200, "{body}");
    let stored: Stored = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (stored.size, stored.sha256.as_str()),
        (3 << 20, digest.as_str())
    );
    let (status, _) = request(
        &agent,
        "PUT",
        "/v1/images/..%2Fx",
        &[("x-sha256", &digest)],
        b"x",
    );
    assert_eq!(status, 404);

    // No flash before the boot files.
    let flash = br#"{"port":"9-9","image":"test.img","sha256":null}"#;
    let (status, _) = request(&agent, "POST", "/v1/flash", &[], flash);
    assert_eq!(status, 409);
    for name in atlas_usb_agent::BOOT_FILES {
        let (status, _) = request(&agent, "PUT", &format!("/v1/boot/{name}"), &[], b"boot");
        assert_eq!(status, 200);
    }
    let (_, body) = request(&agent, "GET", "/v1/agent", &[], b"");
    assert!(serde_json::from_str::<AgentInfo>(&body).unwrap().boot_files);

    // Nothing is on USB 9-9: the job runs and says so.
    let (status, body) = request(&agent, "POST", "/v1/flash", &[], flash);
    assert_eq!(status, 200, "{body}");
    let job: JobStarted = serde_json::from_str(&body).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let events = loop {
        let (_, body) = request(&agent, "GET", &format!("/v1/jobs/{}", job.id), &[], b"");
        let events: JobEvents = serde_json::from_str(&body).unwrap();
        if events.finished {
            break events;
        }
        assert!(Instant::now() < deadline, "the job didn't finish");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        matches!(events.events.last(), Some(Event::Error { message }) if message.contains("nothing is on USB 9-9")),
        "{:?}",
        events.events
    );
    // Read again from the end: nothing new, still finished.
    let (_, body) = request(
        &agent,
        "GET",
        &format!("/v1/jobs/{}?from={}", job.id, events.next),
        &[],
        b"",
    );
    let again: JobEvents = serde_json::from_str(&body).unwrap();
    assert!(again.events.is_empty() && again.finished);
    assert!(agent.state.join(format!("jobs/{}.jsonl", job.id)).is_file());
    let (status, _) = request(&agent, "GET", "/v1/jobs/nope", &[], b"");
    assert_eq!(status, 404);
}

#[test]
fn boot_only_files_and_jobs() {
    let agent = start();
    let config = b"boot_ramdisk=1";
    let bootcode = [1u8; 64];
    let config_sha = sha256(config);
    let code_sha = sha256(&bootcode);

    // A boot-only file needs its SHA-256, which must match, and a path that
    // is a name or overlays/<name> and nothing else.
    let (status, _) = request(&agent, "PUT", "/v1/boot-only/files/config.txt", &[], config);
    assert_eq!(status, 400);
    let wrong = "0".repeat(64);
    let (status, _) = request(
        &agent,
        "PUT",
        "/v1/boot-only/files/config.txt",
        &[("x-sha256", &wrong)],
        config,
    );
    assert_eq!(status, 400);
    assert!(!agent.state.join("boot-only/files/config.txt").exists());
    for bad in [
        "/v1/boot-only/files/..%2Fx",
        "/v1/boot-only/files/a/b/c/d/e",
        "/v1/boot-only/files/slots/./x",
        "/v1/boot-only/files/overlays/../x",
        "/v1/boot-only/files//etc",
    ] {
        let (status, body) = request(&agent, "PUT", bad, &[("x-sha256", &config_sha)], config);
        assert_eq!(status, 400, "{bad}: {body}");
    }
    let (status, body) = request(
        &agent,
        "PUT",
        "/v1/boot-only/files/config.txt",
        &[("x-sha256", &config_sha)],
        config,
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = request(
        &agent,
        "PUT",
        "/v1/boot-only/files/bootcode5.bin",
        &[("x-sha256", &code_sha)],
        &bootcode,
    );
    assert_eq!(status, 200, "{body}");
    let (status, _) = request(&agent, "GET", "/v1/boot-only/files/config.txt", &[], b"");
    assert_eq!(status, 405);

    // A job names only stored files, with their hashes, and a timeout in range.
    let start = |body: String| request(&agent, "POST", "/v1/boot-only", &[], body.as_bytes());
    let (status, body) = start(format!(
        r#"{{"port":"9-9","files":{{"nope.txt":"{config_sha}"}}}}"#
    ));
    assert_eq!(status, 409, "{body}");
    let (status, body) = start(format!(
        r#"{{"port":"9-9","files":{{"config.txt":"{wrong}","bootcode5.bin":"{code_sha}"}}}}"#
    ));
    assert_eq!(status, 409, "{body}");
    let (status, _) = start(format!(
        r#"{{"port":"9-9","files":{{"config.txt":"{config_sha}"}},"timeout_s":0}}"#
    ));
    assert_eq!(status, 400);
    let (status, body) = start(format!(
        r#"{{"port":"9-9","files":{{"../config.txt":"{config_sha}"}}}}"#
    ));
    assert_eq!(status, 400, "{body}");
    // Without a second stage among its files the set is refused.
    let (status, body) = start(format!(
        r#"{{"port":"9-9","files":{{"config.txt":"{config_sha}"}}}}"#
    ));
    assert_eq!(status, 400, "{body}");

    // A set that is there: nothing is on USB 9-9, so the job waits out its
    // timeout, says so, and removes its own set.
    let (status, body) = start(format!(
        r#"{{"port":"9-9","files":{{"config.txt":"{config_sha}","bootcode5.bin":"{code_sha}"}},"timeout_s":1}}"#
    ));
    assert_eq!(status, 200, "{body}");
    let job: JobStarted = serde_json::from_str(&body).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let events = loop {
        let (_, body) = request(&agent, "GET", &format!("/v1/jobs/{}", job.id), &[], b"");
        let events: JobEvents = serde_json::from_str(&body).unwrap();
        if events.finished {
            break events;
        }
        assert!(Instant::now() < deadline, "the boot-only job didn't finish");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        matches!(
            events.events.last(),
            Some(Event::BootEnd {
                outcome: BootOutcome::Timeout,
                message: Some(_)
            })
        ),
        "{:?}",
        events.events
    );
    // Nothing of the job's own is left, and the stored files are untouched.
    let left: Vec<_> = std::fs::read_dir(agent.state.join("boot-only"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["files".to_string()]);
    assert!(agent.state.join("boot-only/files/config.txt").is_file());
    assert!(!agent.state.join("boot/config.txt").exists());
}
