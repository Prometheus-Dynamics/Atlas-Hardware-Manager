//! Support bundles from HeliOS devices: `GET /v1/support/bundle` (a
//! redacted .tar.gz HeliOS makes, with the board's own support-bundle in
//! it on a Raze). A SystemCore's own, and each board's behind it through
//! its gateway (`/v1/peers/<node>/support/bundle`); a board behind a
//! controller through that gateway; any other board from its own HeliOS.
//! No SSH key or login anywhere.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use atlas_driver::Identity;
use atlas_driver::controller_peer::ControllerPeer;
use atlas_driver::util::{address_host, host_port};

/// HeliOS's API on a board (the controller's is `HELIOS_PORT`).
const BOARD_HELIOS_PORT: u16 = 5800;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A bundle takes a while: HeliOS packs the journal of three boots.
const FETCH_TIMEOUT: Duration = Duration::from_secs(180);
/// A bundle larger than this is left out (a board's is a few MB).
const MAX_BUNDLE_BYTES: usize = 256 << 20;

/// One file of the saved archive.
#[derive(Debug, PartialEq, Eq)]
pub struct BundleMember {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The HeliOS bundles for `device`, and a line for each that couldn't be
/// had (the device doesn't run HeliOS, a peer didn't answer).
pub async fn device_bundles(device: &Identity) -> (Vec<BundleMember>, Vec<String>) {
    let http = match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .build()
    {
        Ok(http) => http,
        Err(error) => return (Vec::new(), vec![error.to_string()]),
    };
    if let Some(peer) = ControllerPeer::from_address(&device.address) {
        let base = format!("http://{}", host_port(&peer.host, peer.port));
        return fetch_all(&http, &[peer_bundle(&base, &peer.node)]).await;
    }
    let Some(host) = address_host(&device.address) else {
        return (
            Vec::new(),
            vec!["no network address for a HeliOS bundle".into()],
        );
    };
    if device.key.family.as_str() == atlas_driver_robot::SYSTEMCORE_FAMILY {
        let base = format!(
            "http://{}",
            host_port(host, atlas_driver_robot::HELIOS_PORT)
        );
        controller_bundles(&http, &base).await
    } else {
        let base = format!("http://{}", host_port(host, BOARD_HELIOS_PORT));
        let name = format!(
            "{}-{}.tar.gz",
            device.key.family.as_str(),
            device.key.serial.0
        );
        fetch_all(&http, &[(name, format!("{base}/v1/support/bundle"))]).await
    }
}

/// A controller's own bundle and each routed peer's, through its gateway.
async fn controller_bundles(
    http: &reqwest::Client,
    base: &str,
) -> (Vec<BundleMember>, Vec<String>) {
    let mut wanted = vec![(
        "systemcore.tar.gz".to_string(),
        format!("{base}/v1/support/bundle"),
    )];
    let mut missing = Vec::new();
    match get(http, &format!("{base}/v1/peers")).await {
        Ok(body) => match atlas_driver_robot::parse_peers(&String::from_utf8_lossy(&body)) {
            Ok(nodes) => wanted.extend(nodes.iter().map(|node| peer_bundle(base, node))),
            Err(error) => missing.push(format!("the boards behind it: {error}")),
        },
        Err(error) => missing.push(format!("the boards behind it: {error}")),
    }
    let (members, more) = fetch_all(http, &wanted).await;
    missing.extend(more);
    (members, missing)
}

fn peer_bundle(base: &str, node: &str) -> (String, String) {
    (
        format!("{node}.tar.gz"),
        format!("{base}/v1/peers/{node}/support/bundle"),
    )
}

async fn fetch_all(
    http: &reqwest::Client,
    wanted: &[(String, String)],
) -> (Vec<BundleMember>, Vec<String>) {
    let mut members = Vec::new();
    let mut missing = Vec::new();
    for (name, url) in wanted {
        match get(http, url).await {
            Ok(bytes) => members.push(BundleMember {
                name: name.clone(),
                bytes,
            }),
            Err(error) => missing.push(format!("{name}: {error}")),
        }
    }
    (members, missing)
}

async fn get(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(format!(
            "{url} sent {} MB, more than a bundle",
            bytes.len() >> 20
        ));
    }
    Ok(bytes.to_vec())
}

/// Writes the archive: `atlas.txt` (Atlas's view of the device), then each
/// device bundle as it came, all in one .tar.gz.
pub fn write_bundle_archive(
    path: &Path,
    atlas_txt: &str,
    members: &[BundleMember],
) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("could not write {}: {error}", path.display());
    let file = std::fs::File::create(path).map_err(fail)?;
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::default(),
    ));
    let now = atlas_driver::util::now_ms() / 1000;
    let mut add = |name: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(now);
        header.set_cksum();
        archive.append_data(&mut header, name, bytes)
    };
    add("atlas.txt", atlas_txt.as_bytes()).map_err(fail)?;
    for member in members {
        add(&member.name, &member.bytes).map_err(fail)?;
    }
    archive
        .into_inner()
        .and_then(|gz| gz.finish())
        .and_then(|mut file| file.flush())
        .map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A HeliOS that answers `routes` (path, status, body) and 404 else.
    async fn fake_helios(routes: Vec<(&'static str, u16, Vec<u8>)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut request = vec![0; 4096];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&request[..read]);
                let path = line.split(' ').nth(1).unwrap_or_default().to_string();
                let (status, body) = routes
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, s, b)| (*s, b.clone()))
                    .unwrap_or((404, Vec::new()));
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        base
    }

    #[tokio::test]
    async fn a_controller_gives_its_own_bundle_and_each_boards() {
        let peers = br#"{"peers":[{"node_id":"raze-37bc","routed":true},{"node_id":"raze-dead","routed":true},{"node_id":"cam","routed":false}]}"#;
        let base = fake_helios(vec![
            ("/v1/support/bundle", 200, b"SC".to_vec()),
            ("/v1/peers", 200, peers.to_vec()),
            ("/v1/peers/raze-37bc/support/bundle", 200, b"RZ".to_vec()),
            ("/v1/peers/raze-dead/support/bundle", 502, Vec::new()),
        ])
        .await;
        let http = reqwest::Client::new();
        let (members, missing) = controller_bundles(&http, &base).await;
        assert_eq!(
            members,
            vec![
                BundleMember {
                    name: "systemcore.tar.gz".into(),
                    bytes: b"SC".to_vec()
                },
                BundleMember {
                    name: "raze-37bc.tar.gz".into(),
                    bytes: b"RZ".to_vec()
                },
            ]
        );
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert!(missing[0].starts_with("raze-dead.tar.gz:") && missing[0].contains("502"));
    }

    #[test]
    fn the_archive_holds_atlas_txt_and_each_bundle() {
        let dir = std::env::temp_dir().join(format!("atlas-support-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bundle.tar.gz");
        let members = [BundleMember {
            name: "systemcore.tar.gz".into(),
            bytes: vec![1, 2, 3],
        }];
        write_bundle_archive(&path, "hello", &members).unwrap();
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
            std::fs::File::open(&path).unwrap(),
        ));
        let mut found = Vec::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            found.push((entry.path().unwrap().display().to_string(), bytes));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            found,
            vec![
                ("atlas.txt".into(), b"hello".to_vec()),
                ("systemcore.tar.gz".into(), vec![1, 2, 3]),
            ]
        );
    }
}
