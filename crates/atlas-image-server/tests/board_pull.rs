//! A board's writer pulling a real bundle from the image server, as a slot
//! install on a board plugged in here does (atlas-driver-board slots.rs):
//! `ATLAS_PULL_BUNDLE=<id>.slot.tar ATLAS_PULL_RUN=<script>` with
//! `--ignored`. The script gets the URL, the SHA-256 and the size, and runs
//! the board's `update stage-url` (devices/raze in a fake root, or a board
//! over its console); it must succeed.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use atlas_driver::Artifact;
use atlas_image_server::{ImageServer, ImageServerConfig};
use sha2::{Digest, Sha256};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs ATLAS_PULL_BUNDLE and ATLAS_PULL_RUN"]
async fn a_board_pulls_a_real_bundle() {
    let path = std::path::PathBuf::from(std::env::var("ATLAS_PULL_BUNDLE").unwrap());
    let run = std::env::var("ATLAS_PULL_RUN").unwrap();
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(&path).unwrap();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let sha256: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let size_bytes = std::fs::metadata(&path).unwrap().len();
    let server = ImageServer::new(ImageServerConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        ..ImageServerConfig::default()
    });
    let artifact = Artifact {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        path,
        sha256: sha256.clone(),
        size_bytes,
    };
    let url = server
        .url_for_peer(&artifact, Some(IpAddr::V4(Ipv4Addr::LOCALHOST)))
        .unwrap();
    let args = [run, url.clone(), sha256, size_bytes.to_string()];
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("sh")
            .args(&args)
            .status()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        status.success(),
        "the board's stage-url from {url}: {status}"
    );
}
