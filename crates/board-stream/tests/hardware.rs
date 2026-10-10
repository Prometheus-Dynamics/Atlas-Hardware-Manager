//! The `hardware` topic against a fake lemnosd (Lemnos's own wire types on a
//! Unix socket): the device description, readings in batches at the rate
//! asked for, and lemnosd going away.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lemnos_ipc::{
    Axis, ChannelDesc, DeviceClass, DeviceDesc, DeviceStatus, Message, Quantity, RawReading,
    Refusal, Request, VERSION, decode_request,
};

fn imu() -> DeviceDesc {
    let channel = |name: &str, axis| ChannelDesc {
        name: name.into(),
        quantity: Quantity::Acceleration,
        axis,
        exponent: -3,
    };
    DeviceDesc {
        id: "imu".into(),
        label: "IMU".into(),
        class: DeviceClass::Imu,
        model: "bmi088".into(),
        status: DeviceStatus::Available,
        reason: String::new(),
        channels: vec![
            channel("acceleration.x", Axis::X),
            channel("acceleration.y", Axis::Y),
        ],
        controls: Vec::new(),
        pixels: 0,
    }
}

/// A lemnosd that answers Hello and List, and once subscribed sends
/// `readings` readings `every` apart, then closes. Records subscriptions.
fn fake_lemnosd(path: PathBuf, readings: u64, every: Duration) -> Arc<Mutex<Vec<(String, u32)>>> {
    let subscribed = Arc::new(Mutex::new(Vec::new()));
    let record = subscribed.clone();
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = socket.read(&mut chunk).unwrap_or(0);
            if read == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..read]);
            while let Ok(Some((request, used))) = decode_request(&buf) {
                buf.drain(..used);
                match request {
                    Request::Hello { .. } => {
                        let welcome = Message::Welcome {
                            version: VERSION,
                            board: "raze".into(),
                            client_id: 1,
                        };
                        socket.write_all(&welcome.encode()).unwrap();
                    }
                    Request::List => {
                        let mut fan = imu();
                        fan.id = "fan".into();
                        fan.class = DeviceClass::Fan;
                        socket
                            .write_all(&Message::Devices(vec![imu(), fan]).encode())
                            .unwrap();
                    }
                    Request::Subscribe {
                        id,
                        device,
                        period_ms,
                    } => {
                        record.lock().unwrap().push((device.clone(), period_ms));
                        // A fan produces no readings: refused. Else granted.
                        let result = if device == "fan" {
                            Err(Refusal::Unsupported)
                        } else {
                            Ok(f64::from(period_ms))
                        };
                        socket
                            .write_all(&Message::Reply { id, result }.encode())
                            .unwrap();
                        if device == "fan" {
                            continue;
                        }
                        for n in 0..readings {
                            let reading = Message::Reading(RawReading {
                                device: device.clone(),
                                timestamp_us: 1_000_000 + n * 10_000,
                                status: DeviceStatus::Available,
                                values: vec![120 + n as i32, -9810],
                            });
                            socket.write_all(&reading.encode()).unwrap();
                            std::thread::sleep(every);
                        }
                        // lemnosd goes away.
                        return;
                    }
                    _ => {}
                }
            }
        }
    });
    subscribed
}

fn lines_of(child: &mut std::process::Child) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

#[test]
fn readings_arrive_in_batches_with_the_devices_channels() {
    let dir = std::env::temp_dir().join(format!("board-stream-hw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).unwrap();
    let socket = dir.join("lemnosd.sock");
    let subscribed = fake_lemnosd(socket.clone(), 30, Duration::from_millis(5));

    let mut child = Command::new(env!("CARGO_BIN_EXE_board-stream"))
        .env("BOARD_RUN_DIR", dir.join("run"))
        .env("BOARD_DATA_DIR", dir.join("data"))
        .env("LEMNOSD_SOCKET", &socket)
        .env("BOARD_BOOT_ID", "b1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"GET /stream?topics=hardware&hardware=fan:100,imu:10,gps HTTP/1.1\r\n\r\n")
        .unwrap();
    let rx = lines_of(&mut child);
    let mut seen = Vec::new();
    let mut samples = 0;
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(5)) {
        if let Some(data) = line.strip_prefix("data: ")
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(data)
            && let Some(rows) = value.get("samples").and_then(|s| s.as_array())
        {
            samples += rows.len();
        }
        let done = line.starts_with("data: {\"reason\"");
        seen.push(line);
        if done {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        *subscribed.lock().unwrap(),
        [("fan".to_string(), 100), ("imu".to_string(), 10)],
        "only the known devices: {seen:#?}"
    );
    let devices = seen
        .iter()
        .skip_while(|line| *line != "event: hardware")
        .nth(1)
        .unwrap_or_else(|| panic!("a hardware message: {seen:#?}"));
    assert!(devices.contains(r#""channels":[{"name":"acceleration.x","unit":"m/s²"},{"name":"acceleration.y","unit":"m/s²"}]"#), "{devices}");
    assert!(
        devices.contains(r#"{"id":"gps","missing":true}"#),
        "{devices}"
    );
    assert!(devices.contains(r#""period_ms":10"#), "{devices}");
    assert_eq!(samples, 30, "every reading, scaled: {seen:#?}");
    let batches = seen.iter().filter(|line| *line == "event: samples").count();
    assert!(
        (2..30).contains(&batches),
        "batched, not one message each: {batches}"
    );
    assert!(
        seen.iter()
            .any(|line| line.contains("[1000000,0.12,-9.81]")),
        "lemnosd's time and the values in their unit: {seen:#?}"
    );
}
