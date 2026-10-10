//! Live readings for the `hardware` topic: one lemnosd connection (client
//! `board-stream`) per viewer, subscribed to the devices it asked for at
//! their periods, only while it is connected. lemnosd polls a device at the
//! fastest period any client asked for (and at least its board `poll_ms`),
//! so the readings arrive at device rate. A thread reads them and hands
//! them to the stream, which sends them in batches.
//!
//! A light (the status ring) has no readings: its frames are watched instead
//! (lemnosd's frame watch, exactly the colours it wrote, sent when they
//! change) and stream as samples of the channels `offset`, `clockwise`,
//! `brightness` (the ring-wide look brightness, 0..1) and `led.0`..`led.<n>`
//! (each logical LED's colour, `0xWWRRGGBB` as a number; logical LED 0 is
//! the ring's top, physical LED `offset`, running clockwise or not).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lemnos_ipc::{
    ClientError, ClientEvent, DeviceClass, DeviceClient, DeviceDesc, FrameUpdate, LedClient,
    LightInfo, Update,
};
use serde_json::{Value, json};

/// The client name lemnosd sees (its logs; reads need no write policy).
const CLIENT: &str = "board-stream";

/// What the reader thread hands to the stream.
#[derive(Debug, PartialEq)]
pub enum Live {
    /// The requested devices: id, class, model, status, the period lemnosd
    /// granted, and their channels (name, unit) in the order samples have
    /// them; `"missing": true` for an id lemnosd doesn't have, `"refused"`
    /// with lemnosd's reason for one it won't stream (a fan, a light).
    Devices(Value),
    /// One reading: the device, lemnosd's monotonic time (µs) and a value
    /// per channel (`None`: not read).
    Sample {
        device: String,
        t_us: u64,
        values: Vec<Option<f64>>,
    },
    /// lemnosd can't be reached (or went away): why.
    Gone(String),
}

fn describe(desc: &DeviceDesc, period_ms: u32) -> Value {
    json!({
        "id": desc.id,
        "class": desc.class.name(),
        "model": desc.model,
        "status": format!("{:?}", desc.status).to_lowercase(),
        "period_ms": period_ms,
        "channels": desc.channels.iter().map(|channel| json!({
            "name": channel.name,
            "unit": channel.quantity.unit().symbol(),
        })).collect::<Vec<_>>(),
    })
}

/// The most frames a second a viewer gets of a light.
const MAX_FPS: u16 = 30;

/// A light's description: its geometry first, then a channel per LED.
fn describe_light(desc: &DeviceDesc, info: &LightInfo, fps: u16) -> Value {
    let mut channels = vec![
        json!({ "name": "offset", "unit": "" }),
        json!({ "name": "clockwise", "unit": "" }),
        json!({ "name": "brightness", "unit": "" }),
    ];
    channels.extend((0..info.count).map(|i| json!({ "name": format!("led.{i}"), "unit": "" })));
    json!({
        "id": desc.id,
        "class": desc.class.name(),
        "model": desc.model,
        "status": format!("{:?}", desc.status).to_lowercase(),
        "period_ms": 1000 / u32::from(fps.max(1)),
        "channels": channels,
    })
}

/// A light's sample: its geometry, then each LED's colour.
fn light_values(info: &LightInfo, pixels: &[u32]) -> Vec<Option<f64>> {
    let mut values = vec![
        Some(f64::from(info.offset)),
        Some(if info.clockwise { 1.0 } else { 0.0 }),
        Some(f64::from(info.look_brightness) / 1000.0),
    ];
    values
        .extend((0..usize::from(info.count)).map(|i| pixels.get(i).map(|&pixel| f64::from(pixel))));
    values
}

/// Starts watching the light `id`'s frames: its description, and the client
/// to read them from (the light's geometry first).
fn watch_light(
    socket: &Path,
    id: &str,
    period_ms: u32,
) -> Result<(LedClient, LightInfo, u16), String> {
    let fps = u16::try_from(1000 / period_ms.max(1))
        .unwrap_or(MAX_FPS)
        .clamp(1, MAX_FPS);
    let mut client = LedClient::connect(socket, CLIENT).map_err(|error| error.to_string())?;
    let granted = client
        .watch_frames(id, fps)
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        match client.next_frame(Some(Duration::from_millis(500))) {
            Ok(Some(ClientEvent::Data(FrameUpdate::Info(info)))) => {
                return Ok((client, info, granted.max(1)));
            }
            Ok(Some(ClientEvent::Disconnected { error })) => return Err(error.to_string()),
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("lemnosd didn't describe the light".into())
}

/// Hands `id`'s frames to the stream until it is dropped or lemnosd goes.
fn read_light(
    mut client: LedClient,
    mut info: LightInfo,
    tx: mpsc::Sender<Live>,
    started: Instant,
) {
    loop {
        let pixels = match client.next_frame(Some(Duration::from_secs(1))) {
            Ok(Some(ClientEvent::Data(FrameUpdate::Frame(frame)))) => frame.pixels,
            // The geometry or what it shows changed: the next frame says so.
            Ok(Some(ClientEvent::Data(FrameUpdate::Info(next)))) => {
                info = next;
                continue;
            }
            Ok(Some(ClientEvent::Disconnected { error })) => {
                let _ = tx.send(Live::Gone(format!("lemnosd: {error}")));
                return;
            }
            Ok(_) => continue,
            Err(error) => {
                let _ = tx.send(Live::Gone(format!("lemnosd: {error}")));
                return;
            }
        };
        let sample = Live::Sample {
            device: info.device.clone(),
            t_us: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            values: light_values(&info, &pixels),
        };
        if tx.send(sample).is_err() {
            return;
        }
    }
}

/// Starts the reader for `wanted` (device id, period ms) on lemnosd's
/// socket; it ends when the receiver is dropped or lemnosd goes.
pub fn start(socket: PathBuf, wanted: Vec<(String, u32)>) -> mpsc::Receiver<Live> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let gone = |error: String| {
            let _ = tx.send(Live::Gone(format!("lemnosd: {error}")));
        };
        let mut client = match DeviceClient::options(&socket, CLIENT)
            .events(false)
            .devices()
        {
            Ok(client) => client,
            Err(error) => return gone(error.to_string()),
        };
        let known: Vec<DeviceDesc> = client.devices().cloned().collect();
        let mut devices: Vec<Value> = Vec::new();
        let mut lights = Vec::new();
        for (id, period) in &wanted {
            let Some(desc) = known.iter().find(|desc| &desc.id == id) else {
                devices.push(json!({ "id": id, "missing": true }));
                continue;
            };
            if desc.class == DeviceClass::Light {
                match watch_light(&socket, id, *period) {
                    Ok((light, info, fps)) => {
                        devices.push(describe_light(desc, &info, fps));
                        lights.push((light, info));
                    }
                    Err(reason) => {
                        let mut device = describe(desc, *period);
                        device["refused"] = Value::from(reason);
                        devices.push(device);
                    }
                }
                continue;
            }
            // lemnosd grants a period (no faster than it can read the
            // device), or says why nothing will come.
            match client.subscribe(id, *period) {
                Ok(granted) => devices.push(describe(desc, granted.max(1))),
                Err(ClientError::Refused(refusal)) => {
                    let mut device = describe(desc, *period);
                    device["refused"] = Value::from(refusal.to_string());
                    devices.push(device);
                }
                // An older lemnosd doesn't answer, but the subscription stands.
                Err(ClientError::Timeout) => devices.push(describe(desc, *period)),
                Err(error) => return gone(error.to_string()),
            }
        }
        if tx
            .send(Live::Devices(json!({ "devices": devices })))
            .is_err()
        {
            return;
        }
        let started = Instant::now();
        for (light, info) in lights {
            let tx = tx.clone();
            std::thread::spawn(move || read_light(light, info, tx, started));
        }
        loop {
            match client.next_event_timeout(Duration::from_secs(1)) {
                Ok(Some(ClientEvent::Data(Update::Reading(reading)))) => {
                    let sample = Live::Sample {
                        device: reading.device.clone(),
                        t_us: reading.timestamp_us,
                        values: reading.values().map(|(_, value)| value).collect(),
                    };
                    if tx.send(sample).is_err() {
                        return;
                    }
                }
                Ok(Some(ClientEvent::Disconnected { error })) => return gone(error.to_string()),
                Ok(_) => {}
                Err(error) => return gone(error.to_string()),
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_streams_its_geometry_then_each_leds_colour() {
        let info = LightInfo {
            device: "status-ring".into(),
            count: 3,
            offset: 5,
            clockwise: true,
            look: "status.ok".into(),
            layer: "status".into(),
            owner: "lemnosd".into(),
            look_brightness: 400,
        };
        // A short frame leaves the missing LEDs unread.
        assert_eq!(
            light_values(&info, &[0x00ff_0000, 0x0000_ff00]),
            vec![
                Some(5.0),
                Some(1.0),
                Some(0.4),
                Some(f64::from(0x00ff_0000_u32)),
                Some(f64::from(0x0000_ff00_u32)),
                None
            ]
        );
    }
}
