use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use atlas_driver::Artifact;
use atlas_image_server::{ImageServer, ImageServerConfig};
use reqwest::StatusCode;
use reqwest::header;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("atlas-image-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn local(config: ImageServerConfig) -> ImageServer {
    ImageServer::new(ImageServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        ..config
    })
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

/// Registers `path` and returns its full URL.
fn offer(server: &ImageServer, path: &Path, sha256: &str) -> (String, String) {
    let addr = server.start().unwrap();
    let size = std::fs::metadata(path).unwrap().len();
    let registered = server.register(path, sha256, size).unwrap();
    (
        format!("http://{addr}{}", registered.path()),
        registered.token,
    )
}

/// Sends a request line exactly as written (no client normalization) and
/// returns the status code.
async fn raw_status(addr: SocketAddr, method: &str, target: &str) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            format!("{method} {target} HTTP/1.1\r\nHost: atlas\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.unwrap();
    let reply = String::from_utf8_lossy(&reply);
    reply
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status in {reply:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn whole_files_heads_and_ranges() {
    let scratch = Scratch::new("ranges");
    let bytes = pattern(100_000);
    let sum = sha256(&bytes);
    let path = scratch.file("photon vision.img.xz", &bytes);
    let server = local(ImageServerConfig::default());
    let (url, _) = offer(&server, &path, &sum);
    assert!(url.contains("photon%20vision.img.xz"), "{url}");
    let client = reqwest::Client::new();
    let etag = format!("\"{sum}\"");

    let whole = client.get(&url).send().await.unwrap();
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(whole.headers()[header::ETAG], etag.as_str());
    assert_eq!(whole.headers()[header::ACCEPT_RANGES], "bytes");
    assert_eq!(whole.headers()[header::CONTENT_LENGTH], "100000");
    assert_eq!(whole.bytes().await.unwrap().as_ref(), bytes.as_slice());

    let head = client.head(&url).send().await.unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "100000");
    assert_eq!(head.headers()[header::ETAG], etag.as_str());
    assert!(head.bytes().await.unwrap().is_empty());

    let part = client
        .get(&url)
        .header(header::RANGE, "bytes=10-19")
        .send()
        .await
        .unwrap();
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.headers()[header::CONTENT_RANGE], "bytes 10-19/100000");
    assert_eq!(part.bytes().await.unwrap().as_ref(), &bytes[10..20]);

    // Resuming: everything from an offset, and a suffix.
    let rest = client
        .get(&url)
        .header(header::RANGE, "bytes=99000-")
        .header(header::IF_RANGE, etag.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(rest.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(rest.bytes().await.unwrap().as_ref(), &bytes[99_000..]);
    let tail = client
        .get(&url)
        .header(header::RANGE, "bytes=-5")
        .send()
        .await
        .unwrap();
    assert_eq!(tail.bytes().await.unwrap().as_ref(), &bytes[99_995..]);

    // A different file under the If-Range: the whole file instead.
    let changed = client
        .get(&url)
        .header(header::RANGE, "bytes=0-9")
        .header(header::IF_RANGE, "\"other\"")
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);

    let beyond = client
        .get(&url)
        .header(header::RANGE, "bytes=100000-")
        .send()
        .await
        .unwrap();
    assert_eq!(beyond.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(beyond.headers()[header::CONTENT_RANGE], "bytes */100000");

    let cached = client
        .get(&url)
        .header(header::IF_NONE_MATCH, etag.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_registered_paths_are_served() {
    let scratch = Scratch::new("paths");
    let bytes = pattern(1000);
    let sum = sha256(&bytes);
    let path = scratch.file("image.img", &bytes);
    scratch.file("secret.txt", b"not offered");
    let server = local(ImageServerConfig::default());
    let (url, token) = offer(&server, &path, &sum);
    let addr = server.local_addr().unwrap();
    let other = "0".repeat(64);

    assert_eq!(
        raw_status(addr, "GET", &format!("/images/{token}/image.img")).await,
        200
    );
    for target in [
        "/".to_string(),
        "/images/".to_string(),
        format!("/images/{token}"),
        format!("/images/{token}/"),
        format!("/images/{other}/image.img"),
        format!("/images/{token}/secret.txt"),
        format!("/images/{token}/../secret.txt"),
        format!("/images/{token}/..%2Fsecret.txt"),
        format!("/images/{token}/%2E%2E%2Fsecret.txt"),
        format!("/images/{token}/image.img/../secret.txt"),
        "/images/../secret.txt".to_string(),
        format!("/images/{}/image.img", token.to_ascii_uppercase()),
        format!("/images/{token}x/image.img"),
        format!("/images/{token}/IMAGE.IMG"),
    ] {
        assert_eq!(raw_status(addr, "GET", &target).await, 404, "{target}");
    }
    assert_eq!(
        raw_status(addr, "POST", &format!("/images/{token}/image.img")).await,
        405
    );
    assert_eq!(
        raw_status(addr, "DELETE", &format!("/images/{token}/image.img")).await,
        405
    );

    assert!(server.revoke(&token));
    assert_eq!(
        reqwest::get(&url).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );

    // Registering checks the file against what the release says.
    assert!(server.register(&path, &sum, 999).is_err());
    assert!(server.register(&path, "not-a-sum", 1000).is_err());
    assert!(server.register(&scratch.0, &sum, 1000).is_err());
    assert!(
        server
            .register(&scratch.0.join("missing"), &sum, 1000)
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registrations_expire_after_idling_and_change_is_noticed() {
    let scratch = Scratch::new("expiry");
    let bytes = pattern(1000);
    let sum = sha256(&bytes);
    let path = scratch.file("image.img", &bytes);
    let server = local(ImageServerConfig {
        idle_expiry: Duration::from_millis(400),
        ..ImageServerConfig::default()
    });
    let (url, _) = offer(&server, &path, &sum);

    // Each request keeps it alive.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(reqwest::get(&url).await.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(server.status().registrations, 1);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        reqwest::get(&url).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(server.status().registrations, 0);

    // A file that changed size since it was offered isn't served.
    let (url, _) = offer(&server, &path, &sum);
    std::fs::write(&path, b"shorter").unwrap();
    assert_eq!(reqwest::get(&url).await.unwrap().status(), StatusCode::GONE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_per_token_are_capped() {
    let scratch = Scratch::new("cap");
    let path = scratch.0.join("big.img");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(256 * 1024 * 1024).unwrap();
    drop(file);
    let server = local(ImageServerConfig {
        max_downloads_per_token: 1,
        ..ImageServerConfig::default()
    });
    let (url, _) = offer(&server, &path, &"0".repeat(64));

    let first = reqwest::get(&url).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let busy = reqwest::get(&url).await.unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(busy.headers().contains_key(header::RETRY_AFTER));
    // HEAD isn't a download.
    let head = reqwest::Client::new().head(&url).send().await.unwrap();
    assert_eq!(head.status(), StatusCode::OK);

    drop(first);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let again = reqwest::Client::new()
            .get(&url)
            .header(header::RANGE, "bytes=0-0")
            .send()
            .await
            .unwrap();
        if again.status() == StatusCode::PARTIAL_CONTENT {
            break;
        }
        assert!(Instant::now() < deadline, "the slot was never released");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A sparse 5 GiB image: headers come at once (nothing is read up front),
/// memory stays small while streaming, and offsets past 4 GiB work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gigabyte_images_stream_from_disk() {
    use std::io::{Seek, SeekFrom, Write};
    const LEN: u64 = 5 * 1024 * 1024 * 1024;
    let scratch = Scratch::new("sparse");
    let path = scratch.0.join("huge.img");
    let mut file = std::fs::File::create(&path).unwrap();
    file.set_len(LEN).unwrap();
    file.seek(SeekFrom::Start(LEN - 4)).unwrap();
    file.write_all(b"END!").unwrap();
    drop(file);
    let server = local(ImageServerConfig::default());
    let (url, _) = offer(&server, &path, &"0".repeat(64));
    let client = reqwest::Client::new();

    let head = client.head(&url).send().await.unwrap();
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH],
        LEN.to_string().as_str()
    );

    let started = Instant::now();
    let mut whole = client.get(&url).send().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "headers came late"
    );
    assert_eq!(whole.content_length(), Some(LEN));
    let mut read = 0u64;
    while read < 128 * 1024 * 1024 {
        let chunk = whole.chunk().await.unwrap().unwrap();
        assert!(chunk.iter().all(|byte| *byte == 0));
        read += chunk.len() as u64;
    }
    if let Some(rss) = resident_bytes() {
        assert!(rss < 1024 * 1024 * 1024, "resident memory {rss} bytes");
    }
    drop(whole);

    let end = client
        .get(&url)
        .header(header::RANGE, format!("bytes={}-", LEN - 8))
        .send()
        .await
        .unwrap();
    assert_eq!(end.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(end.bytes().await.unwrap().as_ref(), b"\0\0\0\0END!");
}

/// This process's resident set, where the OS tells it cheaply.
fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn urls_name_the_address_that_routes_to_the_board() {
    let scratch = Scratch::new("host");
    let bytes = pattern(64);
    let sum = sha256(&bytes);
    let path = scratch.file("image.img", &bytes);
    let artifact = Artifact {
        name: "image.img".into(),
        path,
        sha256: sum.clone(),
        size_bytes: 64,
    };
    let server = ImageServer::new(ImageServerConfig {
        bind: "0.0.0.0:0".parse().unwrap(),
        ..ImageServerConfig::default()
    });
    assert!(!server.status().listening);

    // Started on first use; a loopback board routes through loopback.
    let url = server
        .url_for_peer(&artifact, Some("127.0.0.1".parse().unwrap()))
        .unwrap();
    let port = server.local_addr().unwrap().port();
    assert!(
        url.starts_with(&format!("http://127.0.0.1:{port}/images/")),
        "{url}"
    );
    assert!(url.ends_with("/image.img"));
    let body = reqwest::get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(sha256(&body), sum);

    // No route and no configured host: say what to set.
    let error = server.url_for_peer(&artifact, None).unwrap_err();
    assert!(error.to_string().contains("image host"), "{error}");
    let mut config = server.config();
    config.host = Some("atlas-laptop.local".into());
    server.reconfigure(config).unwrap();
    let url = server.url_for_peer(&artifact, None).unwrap();
    assert!(url.starts_with(&format!("http://atlas-laptop.local:{port}/images/")));

    let status = server.status();
    assert!(status.listening && status.error.is_none());
    assert_eq!(status.port, port);
    assert_eq!(status.registrations, 2);
    assert_ne!(server.health().status, atlas_driver::HealthStatus::Error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_taken_port_is_reported() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = ImageServer::new(ImageServerConfig {
        bind: taken.local_addr().unwrap(),
        ..ImageServerConfig::default()
    });
    assert!(server.start().is_err());
    let status = server.status();
    assert!(!status.listening);
    assert!(status.error.is_some());
    assert_eq!(server.health().status, atlas_driver::HealthStatus::Error);

    // Moving to a free port recovers.
    let mut config = server.config();
    config.bind = "127.0.0.1:0".parse().unwrap();
    server.reconfigure(config).unwrap();
    server.start().unwrap();
    assert!(server.status().listening);
    assert!(server.status().error.is_none());
}
