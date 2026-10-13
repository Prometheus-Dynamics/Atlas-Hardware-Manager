//! The `board.hardware` stream: lemnosd's devices, a series per channel
//! (`<device>.<channel>`), read with board-stream's reader
//! (`board_stream::hardware`) for the devices someone watches, or that are
//! sampled always. A light streams its geometry and each LED's colour
//! (`<light>.led.<n>`, `0xWWRRGGBB` as `U32`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use board_stream::hardware::{Live, start};
use lemnos_ipc::{DeviceClass, DeviceClient};
use orion_control_plane::{HistoryPolicy, SampleChunkBuilder, SeriesDecl, SeriesKind};

/// The client name lemnosd sees for the device list.
const CLIENT: &str = "board-agent";

struct Device {
    id: String,
    light: bool,
    /// Its first series; then one per channel.
    first: u16,
    count: u16,
    period_ms: u32,
}

pub struct Hardware {
    pub series: Vec<SeriesDecl>,
    socket: PathBuf,
    devices: Vec<Device>,
    /// The devices being read (indexes) and their readings.
    reading: BTreeSet<usize>,
    rx: Option<Receiver<Live>>,
}

/// How often a device of `class` is asked for.
fn period_ms(class: DeviceClass) -> u32 {
    match class {
        DeviceClass::Imu
        | DeviceClass::Accelerometer
        | DeviceClass::Gyroscope
        | DeviceClass::Magnetometer
        | DeviceClass::Orientation => 10,
        DeviceClass::Light => 33,
        _ => 100,
    }
}

fn history(period_ms: u32) -> HistoryPolicy {
    if period_ms < 100 {
        // 30 s at full rate, 5 minutes at 10 Hz.
        HistoryPolicy::full(30_000).with_reduced(300_000, 100)
    } else {
        HistoryPolicy::full(300_000)
    }
}

/// CLOCK_BOOTTIME (lemnosd's reading times) minus the Unix clock, in µs.
fn boot_to_unix_us() -> i64 {
    let unix = super::unix_us() as i64;
    let boot = nix::time::clock_gettime(nix::time::ClockId::CLOCK_BOOTTIME)
        .map(|t| t.tv_sec() * 1_000_000 + t.tv_nsec() / 1_000)
        .unwrap_or(0);
    unix - boot
}

impl Hardware {
    /// lemnosd's devices, or why there are none.
    pub fn discover(socket: PathBuf) -> Result<Self, String> {
        let client = DeviceClient::options(&socket, CLIENT)
            .events(false)
            .devices()
            .map_err(|error| error.to_string())?;
        let mut series = Vec::new();
        let mut devices = Vec::new();
        for desc in client.devices() {
            let period = period_ms(desc.class);
            let first = u16::try_from(series.len()).map_err(|_| "too many channels")?;
            let light = desc.class == DeviceClass::Light;
            let gauge = |name: String, unit: &str, kind: SeriesKind| {
                let decl = SeriesDecl::new(format!("{}.{name}", desc.id), kind, period * 1000)
                    .with_history(history(period));
                if unit.is_empty() {
                    decl
                } else {
                    decl.with_unit(unit)
                }
            };
            if light {
                for name in ["offset", "clockwise", "brightness"] {
                    series.push(gauge(name.into(), "", SeriesKind::F32));
                }
                for led in 0..desc.pixels {
                    series.push(gauge(format!("led.{led}"), "", SeriesKind::U32));
                }
            } else {
                for channel in &desc.channels {
                    let unit = channel.quantity.unit().symbol();
                    series.push(gauge(channel.name.clone(), unit, SeriesKind::F32));
                }
            }
            let count = u16::try_from(series.len()).map_err(|_| "too many channels")? - first;
            if count > 0 {
                devices.push(Device {
                    id: desc.id.clone(),
                    light,
                    first,
                    count,
                    period_ms: period,
                });
            }
        }
        Ok(Self {
            series,
            socket,
            devices,
            reading: BTreeSet::new(),
            rx: None,
        })
    }

    /// The devices with at least one of `series` (indexes).
    pub fn devices_of(&self, series: impl Fn(u16) -> bool) -> BTreeSet<usize> {
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, d)| (d.first..d.first + d.count).any(&series))
            .map(|(i, _)| i)
            .collect()
    }

    /// Reads exactly `devices` from now on.
    pub fn read(&mut self, devices: BTreeSet<usize>) {
        if devices == self.reading && (self.rx.is_some() || devices.is_empty()) {
            return;
        }
        // Dropping the receiver ends the old reader at its next reading.
        self.rx = None;
        if !devices.is_empty() {
            let wanted = devices
                .iter()
                .map(|&i| (self.devices[i].id.clone(), self.devices[i].period_ms))
                .collect();
            self.rx = Some(start(self.socket.clone(), wanted));
        }
        self.reading = devices;
    }

    pub fn is_reading(&self) -> bool {
        self.rx.is_some()
    }

    /// Moves what arrived into `out` (a builder per series). `Err`: lemnosd
    /// went away, and why.
    pub fn drain(&mut self, out: &mut [SampleChunkBuilder]) -> Result<(), String> {
        let Some(rx) = &self.rx else {
            return Ok(());
        };
        let offset = boot_to_unix_us();
        loop {
            match rx.try_recv() {
                Ok(Live::Sample {
                    device,
                    t_us,
                    values,
                }) => {
                    let Some(device) = self.devices.iter().find(|d| d.id == device) else {
                        continue;
                    };
                    // A light's times are the reader's own; a reading's are
                    // lemnosd's CLOCK_BOOTTIME.
                    let t = if device.light {
                        super::unix_us()
                    } else {
                        u64::try_from(t_us as i64 + offset).unwrap_or(0)
                    };
                    for (i, value) in values
                        .into_iter()
                        .take(usize::from(device.count))
                        .enumerate()
                    {
                        if let (Some(value), Some(builder)) =
                            (value, out.get_mut(usize::from(device.first) + i))
                        {
                            builder.push(t, value);
                        }
                    }
                }
                Ok(Live::Devices(_)) => {}
                Ok(Live::Gone(reason)) => {
                    self.rx = None;
                    return Err(reason);
                }
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return Err("lemnosd's reader ended".into());
                }
            }
        }
    }
}
