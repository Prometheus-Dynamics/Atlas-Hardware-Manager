//! The `board.live` stream's readings, from /proc and /sys: CPU (all and per
//! core), thermal zones, memory, the fan's hwmon, uptime. Each file stays
//! open and is read again from offset 0 (one `pread` a file, no process, no
//! allocation once the buffer has grown).

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

use orion_control_plane::{HistoryPolicy, SampleChunkBuilder, SeriesDecl, SeriesKind};

/// 10 Hz.
pub const PERIOD_US: u32 = 100_000;
/// Uptime: 1 Hz.
const UPTIME_EVERY: u64 = 10;
/// Five minutes at full rate.
const HISTORY_MS: u32 = 300_000;
/// A gap longer than this restarts the CPU deltas (the series was idle).
const MAX_CPU_GAP_US: u64 = 500_000;

enum Reader {
    /// `/proc/stat`: series `first` is all CPUs, then a series per core.
    Stat {
        file: File,
        first: u16,
        cores: usize,
        /// (busy, total) jiffies at the last sample, all CPUs first.
        previous: Vec<(u64, u64)>,
        previous_t_us: u64,
    },
    /// `/proc/meminfo`: `memory` (% used), then `memory.available` (MiB).
    Meminfo { file: File, first: u16 },
    /// A value scaled by `scale` (a thermal zone's millidegrees, an rpm).
    Value { file: File, series: u16, scale: f64 },
    /// `/proc/uptime`, in seconds.
    Uptime { file: File, series: u16 },
}

pub struct Sys {
    pub series: Vec<SeriesDecl>,
    readers: Vec<Reader>,
    buf: Vec<u8>,
    ticks: u64,
}

fn gauge(name: impl Into<String>, unit: &str) -> SeriesDecl {
    SeriesDecl::new(name, SeriesKind::F32, PERIOD_US)
        .with_unit(unit)
        .with_history(HistoryPolicy::full(HISTORY_MS))
}

fn line(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// Reads `file` from offset 0 into `buf`, growing it until the file fits.
fn read_all<'a>(file: &File, buf: &'a mut Vec<u8>) -> Option<&'a [u8]> {
    if buf.is_empty() {
        buf.resize(4096, 0);
    }
    loop {
        let mut len = 0;
        loop {
            match file.read_at(&mut buf[len..], len as u64) {
                Ok(0) => break,
                Ok(n) => {
                    len += n;
                    if len == buf.len() {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
        if len < buf.len() {
            return Some(&buf[..len]);
        }
        let grown = buf.len() * 2;
        buf.resize(grown, 0);
    }
}

fn number(text: &[u8]) -> Option<u64> {
    let text = text.trim_ascii();
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(text).ok()?.parse().ok()
}

/// (busy, total) of one `cpu…` line of /proc/stat: busy is all but idle
/// and iowait; guest time is already in user time.
fn cpu_jiffies(fields: &[u8]) -> Option<(u64, u64)> {
    let mut total = 0;
    let mut idle = 0;
    for (i, field) in fields
        .split(|b| *b == b' ')
        .filter(|f| !f.is_empty())
        .take(8)
        .enumerate()
    {
        let value = number(field)?;
        total += value;
        if i == 3 || i == 4 {
            idle += value;
        }
    }
    Some((total - idle, total))
}

impl Sys {
    /// What `proc` and `sys` (normally /proc and /sys) have.
    pub fn discover(proc: &Path, sys: &Path) -> Self {
        let mut series = Vec::new();
        let mut readers = Vec::new();
        let index = |series: &Vec<SeriesDecl>| u16::try_from(series.len()).unwrap_or(u16::MAX);

        if let Ok(file) = File::open(proc.join("stat")) {
            let cores = std::fs::read_to_string(proc.join("stat"))
                .map(|text| {
                    text.lines()
                        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
                        .count()
                })
                .unwrap_or(0);
            let first = index(&series);
            series.push(gauge("cpu", "%"));
            series.extend((0..cores).map(|core| gauge(format!("cpu.core{core}"), "%")));
            readers.push(Reader::Stat {
                file,
                first,
                cores,
                previous: Vec::new(),
                previous_t_us: 0,
            });
        }
        if let Ok(file) = File::open(proc.join("meminfo")) {
            let first = index(&series);
            series.push(gauge("memory", "%"));
            series.push(gauge("memory.available", "MiB"));
            readers.push(Reader::Meminfo { file, first });
        }
        let mut zones: Vec<_> = std::fs::read_dir(sys.join("class/thermal"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("thermal_zone"))
            })
            .collect();
        zones.sort();
        for zone in zones {
            let (Some(kind), Ok(file)) = (line(&zone.join("type")), File::open(zone.join("temp")))
            else {
                continue;
            };
            let mut name = format!("{kind}.temp");
            if series.iter().any(|s: &SeriesDecl| s.name == name) {
                name = format!(
                    "{kind}{}.temp",
                    zone.to_string_lossy()
                        .trim_start_matches(|c: char| !c.is_ascii_digit())
                );
            }
            readers.push(Reader::Value {
                file,
                series: index(&series),
                scale: 0.001,
            });
            series.push(gauge(name, "°C"));
        }
        let mut hwmons: Vec<_> = std::fs::read_dir(sys.join("class/hwmon"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        hwmons.sort();
        let fan = hwmons
            .iter()
            .find_map(|dir| File::open(dir.join("fan1_input")).ok().map(|f| (dir, f)));
        if let Some((dir, file)) = fan {
            readers.push(Reader::Value {
                file,
                series: index(&series),
                scale: 1.0,
            });
            series.push(gauge("fan.rpm", "rpm"));
            if let Ok(file) = File::open(dir.join("pwm1")) {
                readers.push(Reader::Value {
                    file,
                    series: index(&series),
                    scale: 100.0 / 255.0,
                });
                series.push(gauge("fan.pwm", "%"));
            }
        }
        if let Ok(file) = File::open(proc.join("uptime")) {
            readers.push(Reader::Uptime {
                file,
                series: index(&series),
            });
            series.push(
                SeriesDecl::new("uptime", SeriesKind::F64, PERIOD_US * UPTIME_EVERY as u32)
                    .with_unit("s")
                    .with_history(HistoryPolicy::full(HISTORY_MS)),
            );
        }
        Self {
            series,
            readers,
            buf: Vec::new(),
            ticks: 0,
        }
    }

    /// Reads everything `wanted` (by series index) asks for, at `t_us`,
    /// into `out` (one builder per series).
    pub fn sample(&mut self, t_us: u64, wanted: &[bool], out: &mut [SampleChunkBuilder]) {
        let tick = self.ticks;
        self.ticks += 1;
        let wants = |series: u16| wanted.get(usize::from(series)).copied().unwrap_or(false);
        let mut push = |series: u16, value: f64| {
            if wants(series)
                && let Some(builder) = out.get_mut(usize::from(series))
            {
                builder.push(t_us, value);
            }
        };
        for reader in &mut self.readers {
            match reader {
                Reader::Stat {
                    file,
                    first,
                    cores,
                    previous,
                    previous_t_us,
                } => {
                    let range = usize::from(*first)..usize::from(*first) + 1 + *cores;
                    if !wanted.get(range).is_some_and(|w| w.iter().any(|w| *w)) {
                        continue;
                    }
                    let Some(text) = read_all(file, &mut self.buf) else {
                        continue;
                    };
                    let fresh = t_us.saturating_sub(*previous_t_us) > MAX_CPU_GAP_US
                        || previous.len() != 1 + *cores;
                    let mut now = Vec::with_capacity(1 + *cores);
                    for row in text.split(|b| *b == b'\n') {
                        let Some(rest) = row.strip_prefix(b"cpu") else {
                            break;
                        };
                        let start = rest.iter().position(|b| *b == b' ').unwrap_or(rest.len());
                        if let Some(jiffies) = cpu_jiffies(&rest[start..]) {
                            now.push(jiffies);
                        }
                    }
                    if !fresh && now.len() == previous.len() {
                        for (i, ((busy, total), (was_busy, was_total))) in
                            now.iter().zip(previous.iter()).enumerate()
                        {
                            let total = total.saturating_sub(*was_total);
                            if total > 0 {
                                let busy = busy.saturating_sub(*was_busy) as f64;
                                push(*first + i as u16, 100.0 * busy / total as f64);
                            }
                        }
                    }
                    *previous = now;
                    *previous_t_us = t_us;
                }
                Reader::Meminfo { file, first } => {
                    if !wants(*first) && !wants(*first + 1) {
                        continue;
                    }
                    let Some(text) = read_all(file, &mut self.buf) else {
                        continue;
                    };
                    let (mut total, mut available) = (None, None);
                    for row in text.split(|b| *b == b'\n') {
                        let field = |prefix: &[u8]| {
                            row.strip_prefix(prefix)
                                .and_then(|rest| number(rest.trim_ascii().strip_suffix(b"kB")?))
                        };
                        total = total.or_else(|| field(b"MemTotal:"));
                        available = available.or_else(|| field(b"MemAvailable:"));
                        if total.is_some() && available.is_some() {
                            break;
                        }
                    }
                    if let (Some(total), Some(available)) = (total, available)
                        && total > 0
                    {
                        let used = total.saturating_sub(available) as f64;
                        push(*first, 100.0 * used / total as f64);
                        push(*first + 1, available as f64 / 1024.0);
                    }
                }
                Reader::Value {
                    file,
                    series,
                    scale,
                } => {
                    if !wants(*series) {
                        continue;
                    }
                    if let Some(value) = read_all(file, &mut self.buf).and_then(|t| {
                        let t = t.trim_ascii();
                        let negative = t.first() == Some(&b'-');
                        let value = number(if negative { &t[1..] } else { t })? as f64;
                        Some(if negative { -value } else { value })
                    }) {
                        push(*series, value * *scale);
                    }
                }
                Reader::Uptime { file, series } => {
                    if !wants(*series) || !tick.is_multiple_of(UPTIME_EVERY) {
                        continue;
                    }
                    if let Some(seconds) = read_all(file, &mut self.buf).and_then(|t| {
                        let first = t.split(|b| *b == b' ').next()?;
                        std::str::from_utf8(first).ok()?.trim().parse::<f64>().ok()
                    }) {
                        push(*series, seconds);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn it_reads_cpu_memory_thermal_fan_and_uptime() {
        let root = std::env::temp_dir().join(format!("board-agent-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (proc, sys) = (root.join("proc"), root.join("sys"));
        let stat = |busy: u64, idle: u64| {
            format!(
                "cpu  {busy} 0 0 {idle} 0 0 0 0 0 0\ncpu0 {busy} 0 0 {idle} 0 0 0 0 0 0\nintr 1 2\n"
            )
        };
        write(&proc.join("stat"), &stat(100, 900));
        write(
            &proc.join("meminfo"),
            "MemTotal:        2048000 kB\nMemFree:  1 kB\nMemAvailable:    1536000 kB\n",
        );
        write(&proc.join("uptime"), "12.50 40.00\n");
        write(
            &sys.join("class/thermal/thermal_zone0/type"),
            "cpu-thermal\n",
        );
        write(&sys.join("class/thermal/thermal_zone0/temp"), "54321\n");
        write(&sys.join("class/hwmon/hwmon2/fan1_input"), "3100\n");
        write(&sys.join("class/hwmon/hwmon2/pwm1"), "255\n");

        let mut live = Sys::discover(&proc, &sys);
        let names: Vec<String> = live.series.iter().map(|s| s.name.clone()).collect();
        assert_eq!(
            names,
            [
                "cpu",
                "cpu.core0",
                "memory",
                "memory.available",
                "cpu-thermal.temp",
                "fan.rpm",
                "fan.pwm",
                "uptime"
            ]
        );
        let kinds: Vec<_> = live.series.iter().map(|s| s.kind).collect();
        let mut out: Vec<_> = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| SampleChunkBuilder::new(i as u16, *kind))
            .collect();
        let all = vec![true; names.len()];
        live.sample(1_000_000, &all, &mut out);
        assert!(out[0].is_empty(), "the CPU needs two readings");
        write(&proc.join("stat"), &stat(150, 1_000));
        live.sample(1_100_000, &all, &mut out);
        let mut value = |i: usize| {
            let chunk = out[i].take();
            let (_, value) = *chunk
                .samples(kinds[i])
                .expect("decodes")
                .last()
                .expect("a sample");
            value
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(near(value(0), 100.0 * 50.0 / 150.0));
        assert!(near(value(1), 100.0 * 50.0 / 150.0));
        assert!(near(value(2), 25.0));
        assert!(near(value(3), 1500.0));
        assert!(near(value(4), 54.321));
        assert!(near(value(5), 3100.0));
        assert!(near(value(6), 100.0));
        assert!(near(value(7), 12.5), "uptime on the first tick");

        // Only what is wanted is read.
        let mut only_fan = vec![false; names.len()];
        only_fan[5] = true;
        live.sample(1_200_000, &only_fan, &mut out);
        assert!(
            out.iter()
                .enumerate()
                .all(|(i, b)| (i == 5) != b.is_empty())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
