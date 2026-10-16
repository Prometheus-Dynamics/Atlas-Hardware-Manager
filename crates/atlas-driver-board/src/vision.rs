//! What a board's PhotonVision sees, live, for the device page: each
//! camera's frame rate, latency and, on the Styx builds, the latency's
//! stages, and the tags it found. Read from PhotonVision's own UI socket
//! (`ws://<board>:5800/websocket_data`, MessagePack), which works on the
//! bench with no NetworkTables server; `updatePipelineResult` messages map
//! each camera to its newest result (about 10 a second).
//!
//! Stages (ms after the sensor's start of frame), as derived:
//! exposure/2 = latency − published, readout = dequeued, ISP = ispDone −
//! dequeued, detection = detectEnd − detectStart, PV + NT = published −
//! detectEnd. A build without them still has latency and fps.

use std::time::Duration;

use futures::StreamExt;
use rmpv::Value;
use serde::Serialize;

pub const PORT: u16 = 5800;
const PATH: &str = "/websocket_data";
const MAX_BACKOFF: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Stages {
    pub exposure_half: f64,
    pub readout: f64,
    pub isp: f64,
    pub detection: f64,
    pub pv_nt: f64,
    /// Start of frame to NetworkTables.
    pub sof_to_nt: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Target {
    pub id: Option<i64>,
    pub yaw: Option<f64>,
    pub pitch: Option<f64>,
    pub area: Option<f64>,
    pub ambiguity: Option<f64>,
    /// Camera to tag, m.
    pub distance: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CameraResult {
    pub camera: String,
    pub fps: Option<f64>,
    /// Capture to NetworkTables, ms.
    pub latency_ms: Option<f64>,
    pub detect_ms: Option<f64>,
    pub stages: Option<Stages>,
    pub targets: Vec<Target>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisionFrame {
    Results {
        cameras: Vec<CameraResult>,
    },
    /// Not reachable now; trying again.
    Waiting {
        reason: String,
    },
}

fn get<'a>(map: &'a Value, key: &str) -> Option<&'a Value> {
    map.as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}

fn num(map: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        let v = get(map, k)?;
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn stages(result: &Value) -> Option<Stages> {
    let s = get(result, "stages")?;
    let latency = num(result, &["latency"])?;
    let at = |k: &str| num(s, &[k]);
    let published = at("published")?;
    let dequeued = at("dequeued").unwrap_or(0.0);
    let isp = at("ispDone").map_or(0.0, |isp| (isp - dequeued).max(0.0));
    let detection = match (at("detectStart"), at("detectEnd")) {
        (Some(a), Some(b)) => (b - a).max(0.0),
        _ => 0.0,
    };
    let pv_nt = at("detectEnd").map_or(0.0, |end| (published - end).max(0.0));
    Some(Stages {
        exposure_half: (latency - published).max(0.0),
        readout: dequeued,
        isp,
        detection,
        pv_nt,
        sof_to_nt: published,
    })
}

fn target(t: &Value) -> Target {
    let distance = ["pose", "bestCameraToTarget", "cameraToTarget"]
        .iter()
        .find_map(|k| {
            let p = get(t, k)?;
            let (x, y, z) = (
                num(p, &["x"])?,
                num(p, &["y"])?,
                num(p, &["z"]).unwrap_or(0.0),
            );
            Some((x * x + y * y + z * z).sqrt())
        });
    Target {
        id: num(t, &["fiducialId", "fiducialID", "id"])
            .map(|v| v as i64)
            .filter(|id| *id >= 0),
        yaw: num(t, &["yaw"]),
        pitch: num(t, &["pitch"]),
        area: num(t, &["area"]),
        ambiguity: num(t, &["ambiguity", "poseAmbiguity"]),
        distance,
    }
}

/// The cameras' results in one UI message, or None for other messages.
pub fn results(message: &Value) -> Option<Vec<CameraResult>> {
    let update = get(message, "updatePipelineResult")?;
    let cameras = update
        .as_map()?
        .iter()
        .filter_map(|(name, result)| {
            Some(CameraResult {
                camera: name.as_str()?.to_owned(),
                fps: num(result, &["fps"]),
                latency_ms: num(result, &["latency", "latencyMillis"]),
                detect_ms: num(result, &["detectMillis"]),
                stages: stages(result),
                targets: get(result, "targets")
                    .and_then(Value::as_array)
                    .map(|list| list.iter().map(target).collect())
                    .unwrap_or_default(),
            })
        })
        .collect();
    Some(cameras)
}

/// `ws://<host>:5800/websocket_data` for an address Atlas reached the board at.
pub fn url_for(address: &str) -> String {
    let host = address
        .split("://")
        .last()
        .unwrap_or(address)
        .split('/')
        .next()
        .unwrap_or(address);
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host).to_owned()
    } else if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!("ws://{host}:{PORT}{PATH}")
}

/// Follows PhotonVision's results at `url` into `sink` until it returns
/// false; reconnects by itself (PhotonVision restarting, a reboot).
pub async fn follow(url: &str, sink: impl Fn(VisionFrame) -> bool) {
    let mut backoff = Duration::from_millis(500);
    loop {
        let reason = match tokio::time::timeout(
            Duration::from_secs(4),
            tokio_tungstenite::connect_async(url),
        )
        .await
        {
            Err(_) => "PhotonVision took too long to answer".to_owned(),
            Ok(Err(error)) => format!("PhotonVision isn't answering ({error})"),
            Ok(Ok((mut socket, _))) => {
                let mut reason = "PhotonVision closed the connection".to_owned();
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
                        Err(_) => {
                            reason = "no results for 5 s".into();
                            break;
                        }
                        Ok(None) => break,
                        Ok(Some(Err(error))) => {
                            reason = error.to_string();
                            break;
                        }
                        Ok(Some(Ok(message))) => {
                            if !message.is_binary() {
                                continue;
                            }
                            let data = message.into_data();
                            let Ok(value) = rmpv::decode::read_value(&mut &data[..]) else {
                                continue;
                            };
                            if let Some(cameras) = results(&value) {
                                backoff = Duration::from_millis(500);
                                if !sink(VisionFrame::Results { cameras }) {
                                    return;
                                }
                            }
                        }
                    }
                }
                reason
            }
        };
        if !sink(VisionFrame::Waiting { reason }) {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (Value::from(k), v))
                .collect(),
        )
    }

    #[test]
    fn a_styx_result_gives_stages_and_tags() {
        let stages = map(vec![
            ("dequeued", Value::F64(3.1)),
            ("ispDone", Value::F64(3.1)),
            ("detectStart", Value::F64(6.9)),
            ("detectEnd", Value::F64(7.17)),
            ("published", Value::F64(7.5)),
        ]);
        let tag = map(vec![
            ("fiducialId", Value::from(7)),
            ("yaw", Value::F64(-3.2)),
            ("ambiguity", Value::F64(0.04)),
            (
                "pose",
                map(vec![
                    ("x", Value::F64(2.0)),
                    ("y", Value::F64(1.0)),
                    ("z", Value::F64(0.2)),
                ]),
            ),
        ]);
        let result = map(vec![
            ("fps", Value::F64(58.0)),
            ("latency", Value::F64(11.6)),
            ("stages", stages),
            ("targets", Value::Array(vec![tag])),
        ]);
        let message = map(vec![("updatePipelineResult", map(vec![("front", result)]))]);
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &message).unwrap();
        let decoded = rmpv::decode::read_value(&mut &buf[..]).unwrap();
        let cameras = results(&decoded).unwrap();
        let front = &cameras[0];
        assert_eq!(front.camera, "front");
        let s = front.stages.as_ref().unwrap();
        assert!((s.exposure_half - 4.1).abs() < 1e-9);
        assert!((s.readout - 3.1).abs() < 1e-9);
        assert!(s.isp.abs() < 1e-9, "sliced: delivery at the first slice");
        assert!((s.detection - 0.27).abs() < 1e-9);
        assert!((s.pv_nt - 0.33).abs() < 1e-9);
        assert_eq!(front.targets[0].id, Some(7));
        assert!((front.targets[0].distance.unwrap() - 5.04_f64.sqrt()).abs() < 1e-9);
        // A libcamera build: latency and fps only.
        let plain = map(vec![(
            "updatePipelineResult",
            map(vec![(
                "rear",
                map(vec![
                    ("fps", Value::F64(30.0)),
                    ("latency", Value::F64(31.0)),
                ]),
            )]),
        )]);
        let cameras = results(&plain).unwrap();
        assert_eq!(cameras[0].stages, None);
        assert_eq!(results(&map(vec![("other", Value::Nil)])), None);
    }

    #[test]
    fn urls() {
        assert_eq!(
            url_for("10.99.99.11"),
            "ws://10.99.99.11:5800/websocket_data"
        );
        assert_eq!(
            url_for("http://raze.local:5800/"),
            "ws://raze.local:5800/websocket_data"
        );
        assert_eq!(url_for("fe80::1"), "ws://[fe80::1]:5800/websocket_data");
    }
}
