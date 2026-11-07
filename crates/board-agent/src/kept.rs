//! `/run/board/hardware.json` kept from one lemnosd connection: lemnosd's
//! devices and a recent reading of each, for the identity endpoint's
//! `GET /status` (its sandboxed user can't reach lemnosd's socket) and the
//! support bundle. It replaces `status --refresh-hardware` on a 10 s timer,
//! which spawned a `lemnos-ctl list` and a `lemnos-ctl read` per device on
//! every run.
//!
//! Devices are subscribed at [`PERIOD_MS`]: lemnosd polls a device at the
//! fastest period any client asked for and at least its board `poll_ms`, so
//! this adds no reads of the bus. The ones it won't stream (a fan, a light)
//! are read on the same connection every [`READ_EVERY`]. Status changes come
//! as events. The file is rewritten when what it says changed, at most every
//! [`MIN_GAP`], and at least every [`HEARTBEAT`] so readers (which use it
//! while under 60 s old) keep using it; it is removed while lemnosd is gone,
//! so they say `null` rather than repeat old readings.
//!
//! Same shape as the script wrote: `{"at":<unix s>,"devices":[{"id",
//! "class","model","status","readings":[{"name","value","unit"}],
//! "controls":[..]}]}`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lemnos_ipc::{
    ClientError, ClientEvent, DeviceClient, DeviceDesc, DeviceStatus, Event, Reading, Update,
};
use serde_json::{Value, json};

/// The client name lemnosd sees, as `lemnos-ctl --client board-status` was.
const CLIENT: &str = "board-status";
/// How often a subscribed device's reading is asked for.
pub const PERIOD_MS: u32 = 10_000;
/// How often the devices lemnosd won't stream are read.
pub const READ_EVERY: Duration = Duration::from_secs(30);
/// The least time between two writes of the file.
pub const MIN_GAP: Duration = Duration::from_secs(5);
/// The most time between two writes, changed or not.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

/// One reading as the file has it: (channel, value, unit).
type Values = Vec<(String, Option<f64>, String)>;

fn values(reading: &Reading) -> Values {
    reading
        .channels()
        .iter()
        .zip(reading.values())
        .map(|(channel, (name, value))| {
            (
                name.to_owned(),
                value,
                channel.quantity.unit().symbol().to_owned(),
            )
        })
        .collect()
}

/// The devices (in lemnosd's order) with their last readings, without `at`.
pub fn devices_json(devices: &[DeviceDesc], readings: &HashMap<String, Values>) -> Value {
    Value::Array(
        devices
            .iter()
            .map(|desc| {
                let readings = readings.get(&desc.id).map_or_else(Vec::new, |values| {
                    values
                        .iter()
                        .map(|(name, value, unit)| json!({ "name": name, "value": value, "unit": unit }))
                        .collect()
                });
                json!({
                    "id": desc.id,
                    "class": desc.class.name(),
                    "model": desc.model,
                    "status": desc.status.name(),
                    "readings": readings,
                    "controls": desc.controls.iter().map(|control| control.name.as_str()).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Writes `{"at":..,"devices":..}` to `path` through a temporary file, world-readable.
fn write(path: &Path, devices: &Value) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let text = format!("{}\n", json!({ "at": unix_now(), "devices": devices }));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, path)
}

struct Keeper {
    path: PathBuf,
    devices: Vec<DeviceDesc>,
    readings: HashMap<String, Values>,
    /// Devices lemnosd won't stream, read every READ_EVERY.
    polled: Vec<String>,
    read_at: Option<Instant>,
    written: Option<(Instant, Value)>,
}

impl Keeper {
    /// (Re)lists the devices and subscribes the ones lemnosd streams.
    fn setup(&mut self, client: &mut DeviceClient) -> Result<(), ClientError> {
        self.devices = client.list()?;
        self.readings
            .retain(|id, _| self.devices.iter().any(|desc| &desc.id == id));
        self.polled.clear();
        for desc in &self.devices {
            if desc.status == DeviceStatus::Missing {
                continue;
            }
            match client.subscribe(&desc.id, PERIOD_MS) {
                Ok(_) | Err(ClientError::Timeout) => {}
                Err(ClientError::Refused(_)) => self.polled.push(desc.id.clone()),
                Err(error) => return Err(error),
            }
        }
        self.read_at = None;
        Ok(())
    }

    fn read_polled(&mut self, client: &mut DeviceClient) {
        if self.read_at.is_some_and(|at| at.elapsed() < READ_EVERY) {
            return;
        }
        self.read_at = Some(Instant::now());
        for id in &self.polled {
            if let Ok(reading) = client.read(id) {
                self.readings.insert(id.clone(), values(&reading));
            }
        }
    }

    fn on(&mut self, update: Update) {
        match update {
            Update::Reading(reading) => {
                self.readings
                    .insert(reading.device.clone(), values(&reading));
            }
            Update::Event(Event::Status {
                device,
                status,
                reason,
                ..
            }) => {
                if let Some(desc) = self.devices.iter_mut().find(|desc| desc.id == device) {
                    desc.status = status;
                    desc.reason = reason;
                }
            }
            Update::Event(_) => {}
        }
    }

    /// Writes the file when it changed (no sooner than MIN_GAP after the
    /// last write) or when HEARTBEAT has passed.
    fn flush(&mut self) {
        let devices = devices_json(&self.devices, &self.readings);
        let due = match &self.written {
            None => true,
            Some((at, last)) => {
                at.elapsed() >= HEARTBEAT || (*last != devices && at.elapsed() >= MIN_GAP)
            }
        };
        if !due {
            return;
        }
        match write(&self.path, &devices) {
            Ok(()) => self.written = Some((Instant::now(), devices)),
            Err(error) => crate::log(&format!("{}: {error}", self.path.display())),
        }
    }

    /// lemnosd went away: no file, so readers say null.
    fn gone(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        self.written = None;
        self.devices.clear();
        self.readings.clear();
        self.polled.clear();
    }
}

/// Keeps `run_dir/hardware.json` from lemnosd at `socket` until the process
/// ends (a thread of its own: lemnos-ipc's client is blocking).
pub fn start(socket: PathBuf, run_dir: PathBuf) {
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(&run_dir);
        let mut keeper = Keeper {
            path: run_dir.join("hardware.json"),
            devices: Vec::new(),
            readings: HashMap::new(),
            polled: Vec::new(),
            read_at: None,
            written: None,
        };
        // Reconnecting: a lemnosd that starts later or restarts is picked up
        // (subscriptions are sent again; the devices are listed again).
        let mut client = match DeviceClient::options(&socket, CLIENT)
            .reconnecting()
            .devices()
        {
            Ok(client) => client,
            Err(error) => return crate::log(&format!("hardware.json: lemnosd: {error}")),
        };
        let mut ready = client.is_connected() && keeper.setup(&mut client).is_ok();
        loop {
            match client.next_event_timeout(Duration::from_secs(1)) {
                Ok(Some(ClientEvent::Connected { .. })) => {
                    ready = keeper.setup(&mut client).is_ok()
                }
                Ok(Some(ClientEvent::Disconnected { .. })) => {
                    ready = false;
                    keeper.gone();
                }
                Ok(Some(ClientEvent::Data(update))) => keeper.on(update),
                Ok(None) => {}
                Err(_) => {
                    ready = false;
                    keeper.gone();
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
            if ready {
                keeper.read_polled(&mut client);
                keeper.flush();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use lemnos_ipc::{ControlDesc, DeviceClass, Quantity};

    use super::*;

    fn device(id: &str, class: DeviceClass, status: DeviceStatus, controls: &[&str]) -> DeviceDesc {
        DeviceDesc {
            id: id.into(),
            label: id.into(),
            class,
            model: "m".into(),
            status,
            reason: String::new(),
            channels: Vec::new(),
            controls: controls
                .iter()
                .map(|name| ControlDesc {
                    name: (*name).into(),
                    quantity: Quantity::Ratio,
                    exponent: 0,
                    min: 0,
                    max: 100,
                })
                .collect(),
            pixels: 0,
        }
    }

    #[test]
    fn the_file_says_what_the_script_said() {
        let devices = [
            device(
                "power",
                DeviceClass::PowerMonitor,
                DeviceStatus::Available,
                &[],
            ),
            device(
                "fan",
                DeviceClass::Temperature,
                DeviceStatus::Missing,
                &["duty"],
            ),
        ];
        let readings = HashMap::from([(
            "power".to_owned(),
            vec![
                ("voltage".to_owned(), Some(12.1), "V".to_owned()),
                ("current".to_owned(), None, "A".to_owned()),
            ],
        )]);
        assert_eq!(
            devices_json(&devices, &readings),
            json!([
                { "id": "power", "class": DeviceClass::PowerMonitor.name(), "model": "m", "status": "available",
                  "readings": [{ "name": "voltage", "value": 12.1, "unit": "V" }, { "name": "current", "value": null, "unit": "A" }],
                  "controls": [] },
                { "id": "fan", "class": DeviceClass::Temperature.name(), "model": "m", "status": "missing",
                  "readings": [], "controls": ["duty"] },
            ])
        );
    }
}
