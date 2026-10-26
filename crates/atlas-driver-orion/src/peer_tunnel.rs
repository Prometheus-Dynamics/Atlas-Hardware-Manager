//! A board behind a robot controller's orion-node, through the tunnel the
//! controller's HeliOS offers (`atlas_driver::controller_peer`): a listener
//! on this computer's loopback per board, each connection to it one
//! `GET /v1/peers/<node>/orion` Upgrade (`101 Switching Protocols`) at the
//! controller, then the bytes both ways. Orion's own auth runs end to end
//! over it; HeliOS refuses it while the robot's security is on (403) and
//! after 4 at once (429), and closes it when idle.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use atlas_driver::controller_peer::ControllerPeer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The controller's answer to the Upgrade, at most.
const HEAD_LIMIT: usize = 8 * 1024;

/// The tunnels' own runtime: a listener outlives whichever runtime first
/// asked for it (the app's, a command's, a test's).
fn runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("atlas-peer-tunnel")
                .enable_io()
                .enable_time()
                .build()
                .ok()
        })
        .as_ref()
}

fn listeners() -> &'static Mutex<HashMap<ControllerPeer, u16>> {
    static LISTENERS: OnceLock<Mutex<HashMap<ControllerPeer, u16>>> = OnceLock::new();
    LISTENERS.get_or_init(Default::default)
}

/// The `orion+tcp://` URL that reaches `peer`'s orion-node: the loopback
/// listener, started the first time. `None` when no port could be had.
pub fn url_for(peer: &ControllerPeer) -> Option<String> {
    let mut listeners = listeners().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(port) = listeners.get(peer) {
        return Some(format!("orion+tcp://127.0.0.1:{port}"));
    }
    let runtime = runtime()?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.set_nonblocking(true).ok()?;
    let port = listener.local_addr().ok()?.port();
    let target = peer.clone();
    runtime.spawn(async move {
        let Ok(listener) = TcpListener::from_std(listener) else {
            return;
        };
        while let Ok((client, _)) = listener.accept().await {
            let target = target.clone();
            tokio::spawn(async move {
                let _ = relay(client, &target).await;
            });
        }
    });
    listeners.insert(peer.clone(), port);
    Some(format!("orion+tcp://127.0.0.1:{port}"))
}

/// The `orion+tcp://` URL for a board Atlas reached through a robot
/// controller (its address is the controller's identity URL for it): the
/// tunnel, never the controller's own node (port 0, which answers nothing,
/// when no tunnel could start). `None` for any other address.
pub fn orion_url(address: &str) -> Option<String> {
    let peer = ControllerPeer::from_address(address)?;
    Some(url_for(&peer).unwrap_or_else(|| "orion+tcp://127.0.0.1:0".to_string()))
}

/// One connection: the Upgrade at the controller, then both ways.
async fn relay(mut client: TcpStream, peer: &ControllerPeer) -> std::io::Result<()> {
    let mut upstream = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((peer.host.as_str(), peer.port)),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "controller"))??;
    let rest = upgrade(&mut upstream, peer).await?;
    if !rest.is_empty() {
        client.write_all(&rest).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Sends the Upgrade and reads the answer's head; the bytes after it, which
/// are already the board's.
async fn upgrade(upstream: &mut TcpStream, peer: &ControllerPeer) -> std::io::Result<Vec<u8>> {
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: Upgrade\r\nUpgrade: orion\r\n\r\n",
        peer.orion_path(),
        peer.host,
        peer.port
    );
    upstream.write_all(request.as_bytes()).await?;
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    let end = loop {
        let read = tokio::time::timeout(CONNECT_TIMEOUT, upstream.read(&mut buffer))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "upgrade"))??;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        head.extend_from_slice(&buffer[..read]);
        if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if head.len() > HEAD_LIMIT {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    switched(&head[..end])?;
    Ok(head[end..].to_vec())
}

/// Whether the controller's answer is `101`; else why not.
fn switched(head: &[u8]) -> std::io::Result<()> {
    let status = String::from_utf8_lossy(head);
    let line = status.lines().next().unwrap_or("");
    let code = line.split_whitespace().nth(1).unwrap_or("");
    if code == "101" {
        return Ok(());
    }
    Err(std::io::Error::other(match code {
        "403" => "the controller refuses the tunnel: the robot's security is on".to_string(),
        "429" => "the controller has as many tunnels to this board as it allows".to_string(),
        _ => format!("the controller answered `{line}`"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_connection_upgrades_at_the_controller_then_carries_the_boards_bytes() {
        // A controller that checks the Upgrade, switches, sends a byte of
        // the board's with its answer, then echoes.
        let controller = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = controller.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = controller.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(
                head.starts_with("GET /v1/peers/raze-37bc/orion HTTP/1.1\r\n"),
                "{head}"
            );
            assert!(head.contains("Upgrade: orion\r\n"), "{head}");
            socket
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: orion\r\n\r\nB")
                .await
                .unwrap();
            let mut buffer = [0u8; 5];
            socket.read_exact(&mut buffer).await.unwrap();
            socket.write_all(&buffer).await.unwrap();
        });
        let peer = ControllerPeer {
            host: "127.0.0.1".into(),
            port,
            node: "raze-37bc".into(),
        };
        let url = url_for(&peer).unwrap();
        assert_eq!(url_for(&peer).unwrap(), url, "one listener per board");
        let local = url.strip_prefix("orion+tcp://").unwrap();
        let mut client = TcpStream::connect(local).await.unwrap();
        let mut first = [0u8; 1];
        client.read_exact(&mut first).await.unwrap();
        assert_eq!(&first, b"B");
        client.write_all(b"hello").await.unwrap();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"hello");
    }

    #[test]
    fn a_refused_tunnel_says_why() {
        assert!(switched(b"HTTP/1.1 101 Switching Protocols\r\n\r\n").is_ok());
        let on = switched(b"HTTP/1.1 403 Forbidden\r\n\r\n").unwrap_err();
        assert!(on.to_string().contains("security is on"), "{on}");
        assert!(switched(b"HTTP/1.1 429 Too Many\r\n\r\n").is_err());
    }
}
