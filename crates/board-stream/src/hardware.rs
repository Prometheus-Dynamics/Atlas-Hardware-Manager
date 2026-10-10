//! Live readings for the `hardware` topic: one lemnosd connection (client
//! `board-stream`) per viewer, subscribed to the devices it asked for at
//! their periods, only while it is connected. lemnosd polls a device at the
//! fastest period any client asked for (and at least its board `poll_ms`),
//! so the readings arrive at device rate. A thread reads them and hands
//! them to the stream, which sends them in batches.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use lemnos_ipc::{ClientEvent, DeviceClient, DeviceDesc, Update};
use serde_json::{Value, json};

/// The client name lemnosd sees (its logs; reads need no write policy).
const CLIENT: &str = "board-stream";

/// What the reader thread hands to the stream.
#[derive(Debug, PartialEq)]
pub enum Live {
    /// The requested devices: id, class, model, status, the period asked
    /// for, and their channels (name, unit) in the order samples have them;
    /// `"missing": true` for an id lemnosd doesn't have.
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
        "class": format!("{:?}", desc.class).to_lowercase(),
        "model": desc.model,
        "status": format!("{:?}", desc.status).to_lowercase(),
        "period_ms": period_ms,
        "channels": desc.channels.iter().map(|channel| json!({
            "name": channel.name,
            "unit": channel.quantity.unit().symbol(),
        })).collect::<Vec<_>>(),
    })
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
        let devices: Vec<Value> = wanted
            .iter()
            .map(|(id, period)| {
                known.iter().find(|desc| &desc.id == id).map_or_else(
                    || json!({ "id": id, "missing": true }),
                    |desc| describe(desc, *period),
                )
            })
            .collect();
        if tx
            .send(Live::Devices(json!({ "devices": devices })))
            .is_err()
        {
            return;
        }
        for (id, period) in &wanted {
            if known.iter().any(|desc| &desc.id == id)
                && let Err(error) = client.subscribe(id, *period)
            {
                return gone(error.to_string());
            }
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
