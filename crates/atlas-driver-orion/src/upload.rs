//! A bundle uploaded to a board behind a robot controller, through the
//! controller's HeliOS: the board can't reach this computer to fetch it, so
//! the file goes to the board's own HeliOS (`/v1/update/uploads`), which
//! keeps it in /data/updates and answers with its `file://` URL there. The
//! board's writer installs that in place and checks its SHA-256.

use std::time::Duration;

use atlas_driver::controller_peer::ControllerPeer;
use atlas_driver::{Artifact, DriverError};
use tokio::io::AsyncReadExt;

/// Read from the file and sent at a time.
const CHUNK: usize = 256 * 1024;
/// No answer at all from the controller for this long fails the upload.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Uploads `artifact` to the board `peer` names, calling `sent` with the
/// fraction sent so far; returns the upload's URL on the board.
pub(crate) async fn upload(
    peer: &ControllerPeer,
    artifact: &Artifact,
    sent: impl Fn(f32) + Send + Sync + 'static,
) -> Result<String, DriverError> {
    let file = tokio::fs::File::open(&artifact.path)
        .await
        .map_err(|error| DriverError::Other(format!("{}: {error}", artifact.path.display())))?;
    let total = artifact.size_bytes.max(1);
    let sent = std::sync::Arc::new(sent);
    let body = futures::stream::unfold((file, 0u64), move |(mut file, done)| {
        let sent = sent.clone();
        async move {
            let mut buffer = vec![0; CHUNK];
            match file.read(&mut buffer).await {
                Ok(0) => None,
                Ok(read) => {
                    buffer.truncate(read);
                    let done = done + read as u64;
                    sent(done as f32 / total as f32);
                    Some((Ok::<_, std::io::Error>(buffer), (file, done)))
                }
                Err(error) => Some((Err(error), (file, done))),
            }
        }
    });
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| DriverError::Other(error.to_string()))?;
    let response = client
        .post(peer.upload_url())
        .header("x-helios-filename", &artifact.name)
        .header("x-helios-sha256", &artifact.sha256)
        .header(reqwest::header::CONTENT_LENGTH, artifact.size_bytes)
        .body(reqwest::Body::wrap_stream(body))
        .send()
        .await
        .map_err(|error| {
            DriverError::Unreachable(format!(
                "uploading to the board through {}: {error}",
                peer.host
            ))
        })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(DriverError::Other(format!(
            "the board's HeliOS refused the upload ({status}): {}",
            text.trim()
        )));
    }
    image_url(&text, &artifact.sha256)
}

/// The upload's `image_url` from HeliOS's answer, once it says it has the
/// file Atlas sent.
fn image_url(answer: &str, sha256: &str) -> Result<String, DriverError> {
    let value: serde_json::Value = serde_json::from_str(answer)
        .map_err(|_| DriverError::Other(format!("the board's HeliOS answered: {answer}")))?;
    let got = value["sha256"].as_str().unwrap_or_default();
    if !got.eq_ignore_ascii_case(sha256) {
        return Err(DriverError::Other(format!(
            "the board's HeliOS has a different file (sha256 {got})"
        )));
    }
    value["image_url"]
        .as_str()
        .filter(|url| url.starts_with("file:///data/updates/"))
        .map(str::to_string)
        .ok_or_else(|| {
            DriverError::Other(format!("the board's HeliOS gave no upload URL: {answer}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_names_the_upload_on_the_board() {
        let sha = "ab".repeat(32);
        let answer = format!(
            r#"{{"id":"u1","filename":"x.slot.tar","size_bytes":3,"sha256":"{sha}","image_url":"file:///data/updates/u1/x.slot.tar","uploaded_at_ms":1,"version":null}}"#
        );
        assert_eq!(
            image_url(&answer, &sha).unwrap(),
            "file:///data/updates/u1/x.slot.tar"
        );
        assert!(image_url(&answer, &"cd".repeat(32)).is_err());
        assert!(
            image_url(
                &answer.replace("file:///data/updates/", "file:///etc/"),
                &sha
            )
            .is_err()
        );
        assert!(image_url("not json", &sha).is_err());
    }
}
