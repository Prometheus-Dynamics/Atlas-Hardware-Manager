//! The self-healing camera view (`atlas_driver_board::camera`) against a
//! real MJPEG server: frames arrive; the server is killed and the view says
//! it is reconnecting; the server comes back on the same port and frames
//! flow again; a stream that stops sending is dropped and reopened. Never
//! more than one connection at a time.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use atlas_driver_board::camera::{CameraEvent, CameraState, follow};

struct Server {
    stop: Arc<AtomicBool>,
    /// Keep the connection open but send nothing.
    freeze: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(addr: SocketAddr, open: &Arc<AtomicUsize>, most_open: &Arc<AtomicUsize>) -> Self {
        let listener = TcpListener::bind(addr).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let freeze = Arc::new(AtomicBool::new(false));
        let (s, f, o, m) = (
            stop.clone(),
            freeze.clone(),
            open.clone(),
            most_open.clone(),
        );
        let thread = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !s.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (s, f, o, m) = (s.clone(), f.clone(), o.clone(), m.clone());
                connections.push(std::thread::spawn(move || {
                    let now = o.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(now, Ordering::SeqCst);
                    socket.set_nonblocking(false).unwrap();
                    let mut request = [0u8; 1024];
                    let _ = socket.read(&mut request);
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=frame\r\n\r\n",
                    );
                    let mut n = 0u8;
                    socket
                        .set_read_timeout(Some(Duration::from_millis(5)))
                        .unwrap();
                    while !s.load(Ordering::SeqCst) {
                        // The viewer hung up (it sees an EOF while frozen too).
                        let mut probe = [0u8; 64];
                        if matches!(socket.read(&mut probe), Ok(0)) {
                            break;
                        }
                        if !f.load(Ordering::SeqCst) {
                            n = n.wrapping_add(1);
                            let part = [
                                b"--frame\r\nContent-Type: image/jpeg\r\n\r\n".as_slice(),
                                &[0xFF, 0xD8, 0xFF, 0xE0, n, 0xFF, 0xD9],
                                b"\r\n",
                            ]
                            .concat();
                            if socket.write_all(&part).is_err() {
                                break;
                            }
                        }
                        std::thread::sleep(Duration::from_millis(30));
                    }
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                    o.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for connection in connections {
                let _ = connection.join();
            }
        });
        Self {
            stop,
            freeze,
            thread: Some(thread),
        }
    }

    fn kill(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[derive(Default)]
struct Seen {
    frames: usize,
    states: Vec<CameraState>,
}

async fn until(seen: &Arc<Mutex<Seen>>, what: &str, test: impl Fn(&Seen) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !test(&seen.lock().unwrap()) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_view_recovers_when_the_stream_dies_comes_back_or_stalls() {
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let (open, most_open) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let server = Server::start(addr, &open, &most_open);

    let seen = Arc::new(Mutex::new(Seen::default()));
    let sink = seen.clone();
    let url = format!("http://{addr}/stream.mjpg");
    let viewer = tokio::spawn(async move {
        follow(&url, move |event| {
            let mut seen = sink.lock().unwrap();
            match event {
                CameraEvent::Frame(jpeg) => {
                    assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
                    seen.frames += 1;
                }
                CameraEvent::State(state) => seen.states.push(state),
            }
            true
        })
        .await;
    });
    until(&seen, "frames", |s| s.frames >= 5).await;
    assert!(seen.lock().unwrap().states.contains(&CameraState::Live));

    // The endpoint goes away (PhotonVision restarts, the board reboots).
    server.kill();
    until(&seen, "reconnecting", |s| {
        matches!(s.states.last(), Some(CameraState::Reconnecting { .. }))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // It comes back on the same address: frames flow again by themselves.
    let before = seen.lock().unwrap().frames;
    let server = Server::start(addr, &open, &most_open);
    until(&seen, "frames after the restart", |s| {
        s.frames >= before + 5
    })
    .await;
    assert_eq!(seen.lock().unwrap().states.last(), Some(&CameraState::Live));

    // The stream stops sending without closing: dropped and reopened.
    server.freeze.store(true, Ordering::SeqCst);
    let states = seen.lock().unwrap().states.len();
    until(&seen, "a stall noticed", |s| {
        s.states[states..].iter().any(|state| {
            matches!(state, CameraState::Reconnecting { reason, .. } if reason.contains("no frame"))
        })
    })
    .await;
    server.freeze.store(false, Ordering::SeqCst);
    let before = seen.lock().unwrap().frames;
    until(&seen, "frames after the stall", |s| s.frames >= before + 5).await;

    viewer.abort();
    server.kill();
    assert_eq!(
        most_open.load(Ordering::SeqCst),
        1,
        "the old connection closes before a new one opens"
    );
}
