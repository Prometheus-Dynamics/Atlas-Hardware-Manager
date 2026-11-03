//! The cameras on a SystemCore, as its HeliOS lists them
//! (`GET /v1/cameras`): each one it can preview becomes a camera view
//! (`camera_stream.<name>`), like a board's.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde::Deserialize;

use crate::controller_peers::HELIOS_PORT;

/// A listing slower than this is left out of this identify.
const ASK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize)]
struct Camera {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    preview_available: bool,
}

/// `(attribute, preview URL)` for each camera in `GET /v1/cameras`'s
/// answer that HeliOS can preview, by its name (its id when two share one).
pub fn streams(ip: Ipv4Addr, body: &str) -> Vec<(String, String)> {
    let Ok(cameras) = serde_json::from_str::<Vec<Camera>>(body) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for camera in cameras
        .into_iter()
        .filter(|camera| camera.preview_available)
    {
        let url = format!("http://{ip}:{HELIOS_PORT}/v1/cameras/{}/preview", camera.id);
        let key = atlas_driver::attributes::camera_stream_key(&camera.name)
            .filter(|key| out.iter().all(|(taken, _)| taken != key))
            .or_else(|| atlas_driver::attributes::camera_stream_key(&camera.id));
        if let Some(key) = key {
            out.push((key, url));
        }
    }
    out
}

/// The SystemCore's camera views; none when its HeliOS doesn't answer.
pub async fn read(http: &reqwest::Client, ip: Ipv4Addr) -> Vec<(String, String)> {
    let body = async {
        http.get(format!("http://{ip}:{HELIOS_PORT}/v1/cameras"))
            .timeout(ASK_TIMEOUT)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()
    };
    body.await.map_or_else(Vec::new, |body| streams(ip, &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_previewable_camera_is_a_view() {
        let body = r#"[
            {"id":"capture_device_systemcore-3c99_video0","name":"046d:0825","preview_available":true},
            {"id":"capture_device_systemcore-3c99_video4","name":"C922 Pro Stream Webcam","preview_available":true},
            {"id":"capture_device_systemcore-3c99_video6","name":"C922 Pro Stream Webcam","preview_available":true},
            {"id":"capture_device_systemcore-3c99_video8","name":"busy","preview_available":false}
        ]"#;
        let ip = Ipv4Addr::new(172, 27, 0, 1);
        assert_eq!(
            streams(ip, body),
            vec![
                (
                    "camera_stream.046d-0825".to_string(),
                    "http://172.27.0.1:5880/v1/cameras/capture_device_systemcore-3c99_video0/preview".to_string()
                ),
                (
                    "camera_stream.C922-Pro-Stream-Webcam".to_string(),
                    "http://172.27.0.1:5880/v1/cameras/capture_device_systemcore-3c99_video4/preview".to_string()
                ),
                (
                    "camera_stream.capture_device_systemcore-3c99_video6".to_string(),
                    "http://172.27.0.1:5880/v1/cameras/capture_device_systemcore-3c99_video6/preview".to_string()
                ),
            ]
        );
        assert!(streams(ip, "<!doctype html>").is_empty());
    }
}
