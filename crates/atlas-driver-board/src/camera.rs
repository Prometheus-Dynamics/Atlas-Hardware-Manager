//! A camera view that heals itself: Atlas reads the board's MJPEG stream
//! (PhotonVision's, or any `multipart/x-mixed-replace` of JPEGs; a plain
//! JPEG URL is read again every second) and hands the frames on, instead of
//! leaving it to an `<img>` that freezes on a broken stream and never says
//! so.
//!
//! - A stream with no frame for [`STALL`] counts as stalled: its connection
//!   is dropped (always before the next one opens) and opened again.
//! - Reconnecting backs off from 500 ms to [`MAX_BACKOFF`], and starts again
//!   from the bottom once frames flow, so it recovers by itself when
//!   PhotonVision restarts, the board reboots or the cable comes back.
//! - Every change is told ([`CameraState`]), so the view keeps the last good
//!   frame under a "reconnecting" note rather than a broken image.

use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;

/// No frame for this long: the stream is stalled.
pub const STALL: Duration = Duration::from_secs(3);
/// The longest wait between attempts.
pub const MAX_BACKOFF: Duration = Duration::from_secs(5);
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// A still image is read again this often.
const STILL_EVERY: Duration = Duration::from_secs(1);
/// A frame larger than this isn't a camera frame: drop what was buffered.
const MAX_FRAME: usize = 8 << 20;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CameraState {
    /// Opening the stream (`attempt` 1 is the first).
    Connecting { attempt: u32 },
    /// Frames are arriving.
    Live,
    /// The stream broke or stalled; trying again in `retry_ms`.
    Reconnecting { reason: String, retry_ms: u64 },
}

pub enum CameraEvent {
    /// One JPEG.
    Frame(Vec<u8>),
    State(CameraState),
}

/// JPEGs out of a byte stream, by their start and end markers (the
/// multipart headers between them don't matter).
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    fn push(&mut self, bytes: &[u8], mut out: impl FnMut(Vec<u8>)) {
        self.buf.extend_from_slice(bytes);
        loop {
            let Some(start) = find(&self.buf, &[0xFF, 0xD8, 0xFF], 0) else {
                // Keep a possible marker split across reads.
                let keep = self.buf.len().min(2);
                self.buf.drain(..self.buf.len() - keep);
                return;
            };
            let Some(end) = find(&self.buf, &[0xFF, 0xD9], start + 2) else {
                self.buf.drain(..start);
                if self.buf.len() > MAX_FRAME {
                    self.buf.clear();
                }
                return;
            };
            out(self.buf[start..end + 2].to_vec());
            self.buf.drain(..end + 2);
        }
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// Why one attempt ended.
enum Ended {
    /// The view went away.
    Stopped,
    /// After at least one frame (back off from the bottom again).
    AfterFrames(String),
    /// Before any frame.
    Failed(String),
}

async fn attempt(
    client: &reqwest::Client,
    url: &str,
    sink: &impl Fn(CameraEvent) -> bool,
) -> Ended {
    let response = match client.get(url).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => return Ended::Failed(format!("the camera answered {}", response.status())),
        Err(error) => return Ended::Failed(describe(&error)),
    };
    let still = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.starts_with("image/"));
    let mut body = response.bytes_stream();
    let mut frames = Frames::default();
    let mut seen = false;
    loop {
        let chunk = match tokio::time::timeout(STALL, body.next()).await {
            Err(_) => {
                let why = format!("no frame for {} s", STALL.as_secs());
                return if seen {
                    Ended::AfterFrames(why)
                } else {
                    Ended::Failed(why)
                };
            }
            Ok(None) if still && seen => {
                // A still image: read it again in a moment.
                tokio::time::sleep(STILL_EVERY).await;
                return Ended::AfterFrames(String::new());
            }
            Ok(None) => {
                let why = "the camera closed the stream".to_owned();
                return if seen {
                    Ended::AfterFrames(why)
                } else {
                    Ended::Failed(why)
                };
            }
            Ok(Some(Err(error))) => {
                let why = describe(&error);
                return if seen {
                    Ended::AfterFrames(why)
                } else {
                    Ended::Failed(why)
                };
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        let mut stopped = false;
        frames.push(&chunk, |jpeg| {
            if !seen {
                seen = true;
                stopped |= !sink(CameraEvent::State(CameraState::Live));
            }
            stopped |= !sink(CameraEvent::Frame(jpeg));
        });
        if stopped {
            return Ended::Stopped;
        }
    }
}

fn describe(error: &reqwest::Error) -> String {
    if error.is_connect() {
        "the camera isn't answering".into()
    } else if error.is_timeout() {
        "the camera took too long to answer".into()
    } else {
        error.to_string()
    }
}

/// Follows the camera at `url` into `sink` until `sink` returns false.
/// Never gives up on its own: a camera that is away is tried again, at most
/// every [`MAX_BACKOFF`].
pub async fn follow(url: &str, sink: impl Fn(CameraEvent) -> bool) {
    let client = match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            sink(CameraEvent::State(CameraState::Reconnecting {
                reason: error.to_string(),
                retry_ms: 0,
            }));
            return;
        }
    };
    let mut backoff = FIRST_BACKOFF;
    let mut attempt_no = 1;
    loop {
        if !sink(CameraEvent::State(CameraState::Connecting {
            attempt: attempt_no,
        })) {
            return;
        }
        // The attempt's connection is dropped when it returns, before the
        // next one opens.
        let reason = match attempt(&client, url, &sink).await {
            Ended::Stopped => return,
            Ended::AfterFrames(reason) => {
                backoff = FIRST_BACKOFF;
                attempt_no = 1;
                if reason.is_empty() {
                    continue;
                }
                reason
            }
            Ended::Failed(reason) => reason,
        };
        if !sink(CameraEvent::State(CameraState::Reconnecting {
            reason,
            retry_ms: u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
        })) {
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
        attempt_no += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_out_whole_across_reads_and_between_headers() {
        let jpeg = |n: u8| vec![0xFF, 0xD8, 0xFF, 0xE0, n, n, 0xFF, 0xD9];
        let mut stream = Vec::new();
        for n in 0..3 {
            stream.extend_from_slice(b"--frame\r\nContent-Type: image/jpeg\r\n\r\n");
            stream.extend_from_slice(&jpeg(n));
            stream.extend_from_slice(b"\r\n");
        }
        let mut frames = Frames::default();
        let mut out = Vec::new();
        for piece in stream.chunks(5) {
            frames.push(piece, |frame| out.push(frame));
        }
        assert_eq!(out, vec![jpeg(0), jpeg(1), jpeg(2)]);
    }
}
