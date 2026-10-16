//! Live readings as Orion streams (Orion `docs/streams.md`): the board's
//! readings at their own rate, kept by orion-node in a history ring per
//! series and pushed to whoever subscribes (Atlas over a push session), with
//! no process started and no request per update.
//!
//! - `board.live`: CPU, cores, memory, thermal zones, the fan, uptime, from
//!   /proc and /sys at 10 Hz ([`sys`]).
//! - `board.hardware`: lemnosd's devices at device rate ([`hardware`]).
//!
//! Each series is sampled always or only while someone watches
//! ([`sampling`]: `BOARD_AGENT_LIVE_SAMPLING`, `BOARD_AGENT_LIVE_SAMPLING_SERIES`).
//! With nothing to sample the loop sleeps.

pub mod hardware;
pub mod sampling;
pub mod sys;

use std::sync::Arc;
use std::time::Duration;

use orion_client::StreamPublisher;
use orion_control_plane::{
    SampleChunk, SampleChunkBuilder, Sampling, SeriesDecl, StatusSubject, StreamDecl, StreamDemand,
    StreamRef,
};
use tokio::time::{Instant, MissedTickBehavior};

use crate::config::Config;
use hardware::Hardware;
use sys::Sys;

/// The publisher's client name on the node.
pub const CLIENT_NAME: &str = "board-agent.live";
/// The /proc and /sys stream.
pub const STREAM_SYS: &str = "board.live";
/// lemnosd's devices.
pub const STREAM_HARDWARE: &str = "board.hardware";

/// How often batches go out (and lemnosd's readings are collected).
const BATCH: Duration = Duration::from_millis(50);
/// How often lemnosd is looked for while it isn't there.
const HARDWARE_RETRY: Duration = Duration::from_secs(10);

pub(crate) fn unix_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// One stream: its series, which are sampled always, which someone wants.
struct Published {
    stream: StreamRef,
    always: Vec<bool>,
    demanded: Vec<bool>,
    builders: Vec<SampleChunkBuilder>,
}

impl Published {
    fn new(
        owner: &StatusSubject,
        id: &str,
        series: &mut [SeriesDecl],
        config: &Config,
        cheap: bool,
    ) -> Self {
        let always: Vec<bool> = series
            .iter_mut()
            .map(|decl| {
                let always = config.live_sampling.always(id, &decl.name, cheap);
                if !always {
                    decl.sampling = Sampling::WhileSubscribed;
                }
                always
            })
            .collect();
        Self {
            stream: StreamRef {
                owner: owner.clone(),
                id: id.into(),
            },
            demanded: vec![false; always.len()],
            builders: series
                .iter()
                .enumerate()
                .map(|(i, decl)| SampleChunkBuilder::new(i as u16, decl.kind))
                .collect(),
            always,
        }
    }

    fn decl(&self, series: Vec<SeriesDecl>) -> StreamDecl {
        StreamDecl {
            owner: self.stream.owner.clone(),
            id: self.stream.id.clone(),
            series,
        }
    }

    fn demand(&mut self, demand: &StreamDemand) {
        self.demanded.fill(false);
        for &series in &demand.series {
            if let Some(wanted) = self.demanded.get_mut(usize::from(series)) {
                *wanted = true;
            }
        }
    }

    fn wanted(&self) -> Vec<bool> {
        self.always
            .iter()
            .zip(&self.demanded)
            .map(|(a, d)| *a || *d)
            .collect()
    }

    fn take(&mut self) -> Vec<SampleChunk> {
        self.builders
            .iter_mut()
            .filter(|b| !b.is_empty())
            .map(SampleChunkBuilder::take)
            .collect()
    }
}

/// The longest wait between attempts while publishing keeps failing.
const RETRY_MAX: Duration = Duration::from_secs(60);
/// A session that lasted this long was working: the next failure waits from
/// the start again.
const HEALTHY: Duration = Duration::from_secs(30);

/// Publishes until the connection fails, then starts over; orion-node
/// restarting starts new epochs. While it keeps failing, the wait doubles
/// (from [`Config::retry`], at least 1 s, up to [`RETRY_MAX`]) and the error
/// is logged when it changes: each attempt is a stream session on the node,
/// so a steady failure mustn't open one a second.
pub async fn run(config: Arc<Config>) {
    let first = config.retry.max(Duration::from_secs(1));
    let mut wait = first;
    let mut said = String::new();
    loop {
        let started = Instant::now();
        let result = publish(&config).await;
        if started.elapsed() >= HEALTHY {
            wait = first;
        }
        if let Err(error) = result
            && error != said
        {
            crate::log(&format!(
                "live: {error}; trying again (waiting up to {} s between tries)",
                RETRY_MAX.as_secs()
            ));
            said = error;
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(RETRY_MAX);
    }
}

async fn publish(config: &Config) -> Result<(), String> {
    let mut publisher = StreamPublisher::connect_at(&config.stream_socket, CLIENT_NAME)
        .await
        .map_err(|e| e.to_string())?;
    let owner = StatusSubject::Node(publisher.node_id().clone());

    let mut sys = Sys::discover(&config.proc_dir, &config.sys_dir);
    let mut sys_series = std::mem::take(&mut sys.series);
    let mut sys_out = Published::new(&owner, STREAM_SYS, &mut sys_series, config, true);
    publisher
        .declare(sys_out.decl(sys_series))
        .await
        .map_err(|e| e.to_string())?;
    crate::log(&format!(
        "live: publishing {STREAM_SYS} ({} series) on node {}",
        sys_out.always.len(),
        publisher.node_id()
    ));

    let mut hardware: Option<(Hardware, Published)> = None;
    let mut look_for_hardware = Instant::now();
    let mut said_no_hardware = false;
    let mut tick = tokio::time::interval(BATCH);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut ticks: u64 = 0;

    loop {
        let sys_wanted = sys_out.wanted();
        let sys_active = sys_wanted.iter().any(|w| *w);
        let hardware_active = hardware.as_ref().is_some_and(|(hw, _)| hw.is_reading());
        tokio::select! {
            demand = publisher.next_demand() => {
                let demand = demand.map_err(|e| e.to_string())?;
                if demand.stream == sys_out.stream {
                    sys_out.demand(&demand);
                } else if let Some((hw, out)) = &mut hardware
                    && demand.stream == out.stream
                {
                    out.demand(&demand);
                    let wanted = out.wanted();
                    hw.read(hw.devices_of(|s| wanted[usize::from(s)]));
                }
            }
            _ = tick.tick(), if sys_active || hardware_active => {
                ticks += 1;
                if ticks.is_multiple_of(2) && sys_active {
                    sys.sample(unix_us(), &sys_wanted, &mut sys_out.builders);
                }
                let chunks = sys_out.take();
                publisher
                    .publish(&sys_out.stream, chunks)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some((hw, out)) = &mut hardware {
                    let gone = hw.drain(&mut out.builders).err();
                    let chunks = out.take();
                    publisher
                        .publish(&out.stream, chunks)
                        .await
                        .map_err(|e| e.to_string())?;
                    if let Some(reason) = gone {
                        crate::log(&format!("live: {reason}; looking again in {}s", HARDWARE_RETRY.as_secs()));
                        hardware = None;
                        look_for_hardware = Instant::now() + HARDWARE_RETRY;
                    }
                }
            }
            _ = tokio::time::sleep_until(look_for_hardware), if hardware.is_none() => {
                look_for_hardware = Instant::now() + HARDWARE_RETRY;
                let socket = config.lemnosd_socket.clone();
                let found = tokio::task::spawn_blocking(move || Hardware::discover(socket))
                    .await
                    .unwrap_or_else(|error| Err(error.to_string()));
                match found {
                    Ok(mut hw) if !hw.series.is_empty() => {
                        let mut series = std::mem::take(&mut hw.series);
                        let out = Published::new(&owner, STREAM_HARDWARE, &mut series, config, false);
                        publisher.declare(out.decl(series)).await.map_err(|e| e.to_string())?;
                        crate::log(&format!("live: publishing {STREAM_HARDWARE} ({} series)", out.always.len()));
                        let wanted = out.wanted();
                        hw.read(hw.devices_of(|s| wanted[usize::from(s)]));
                        hardware = Some((hw, out));
                        said_no_hardware = false;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if !said_no_hardware {
                            crate::log(&format!("live: no lemnosd ({error}); {STREAM_HARDWARE} waits for it"));
                            said_no_hardware = true;
                        }
                    }
                }
            }
        }
    }
}
