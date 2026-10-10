//! A board's push channel (`endpoints.stream`, board-stream on TCP 5898):
//! Server-Sent Events that say something changed (a new event, the update
//! state) as it happens, so Atlas re-reads the board then instead of on its
//! next poll. The messages are signals: what they point at is fetched over
//! the ordinary endpoints, which catch up by seq.

use std::time::Duration;

use atlas_driver::{DeviceEvent, DriverError, FrameSink, HardwareFrame, PushSink, StatusPush};
use serde_json::Value;

/// A board sends a keepalive every 10 s: silence this long means it's gone.
const SILENCE: Duration = Duration::from_secs(30);
const CONNECT: Duration = Duration::from_secs(3);

/// One Server-Sent Events message: its `event` name and `data`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) event: String,
    pub(crate) data: String,
}

/// Splits a byte stream into messages (blank-line separated; `event:` and
/// `data:` fields, comments and other fields ignored).
#[derive(Default)]
pub(crate) struct Parser {
    pending: String,
}

impl Parser {
    pub(crate) fn feed(&mut self, text: &str) -> Vec<Message> {
        self.pending.push_str(&text.replace("\r\n", "\n"));
        let mut messages = Vec::new();
        while let Some(end) = self.pending.find("\n\n") {
            let block: String = self.pending.drain(..end + 2).collect();
            let mut message = Message::default();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    message.event = value.trim_start().to_string();
                } else if let Some(value) = line.strip_prefix("data:") {
                    if !message.data.is_empty() {
                        message.data.push('\n');
                    }
                    message
                        .data
                        .push_str(value.strip_prefix(' ').unwrap_or(value));
                }
            }
            if !message.event.is_empty() {
                messages.push(message);
            }
        }
        messages
    }
}

/// The push a message means, if it's one Atlas knows.
pub(crate) fn push_of(message: &Message) -> Option<StatusPush> {
    let data: Value = serde_json::from_str(&message.data).ok()?;
    match message.event.as_str() {
        "hello" => Some(StatusPush::Hello {
            boot_id: data.get("boot_id")?.as_str()?.to_string(),
            newest_seq: data.get("seq").and_then(Value::as_u64),
        }),
        "event" => Some(StatusPush::Event(Box::new(
            serde_json::from_value::<DeviceEvent>(data).ok()?,
        ))),
        "update" => Some(StatusPush::Update {
            state: data.get("state")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

/// Holds the stream at `url` until it ends, passing each push to `sink`.
/// `Ok(true)` when it ended (closed, or silent too long); an error when it
/// couldn't be opened.
pub(crate) async fn follow(
    http: &reqwest::Client,
    url: &str,
    sink: PushSink,
) -> Result<bool, DriverError> {
    follow_messages(http, url, |message| {
        if let Some(push) = push_of(message) {
            sink(push);
        }
    })
    .await
}

/// The live frame a `hardware` topic message means, if it's one.
pub(crate) fn frame_of(message: &Message) -> Option<HardwareFrame> {
    let data: Value = serde_json::from_str(&message.data).ok()?;
    match message.event.as_str() {
        "hardware" => Some(HardwareFrame::Devices {
            devices: serde_json::from_value(data.get("devices")?.clone()).ok()?,
        }),
        "samples" => Some(HardwareFrame::Samples {
            device: data.get("device")?.as_str()?.to_string(),
            samples: data
                .get("samples")?
                .as_array()?
                .iter()
                .filter_map(|row| {
                    let row = row.as_array()?;
                    let t_us = row.first()?.as_u64()?;
                    Some((t_us, row[1..].iter().map(Value::as_f64).collect()))
                })
                .collect(),
        }),
        "hardware-gone" => Some(HardwareFrame::Gone {
            reason: data.get("reason")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

/// Follows the board's live readings of `wanted` until the stream ends.
pub(crate) async fn follow_hardware(
    http: &reqwest::Client,
    stream_url: &str,
    wanted: &[(String, u32)],
    sink: FrameSink,
) -> Result<bool, DriverError> {
    let list: Vec<String> = wanted
        .iter()
        .map(|(device, period)| format!("{device}:{period}"))
        .collect();
    let separator = if stream_url.contains('?') { '&' } else { '?' };
    let url = format!(
        "{stream_url}{separator}topics=hardware&hardware={}",
        list.join(",")
    );
    follow_messages(http, &url, |message| {
        if let Some(frame) = frame_of(message) {
            sink(frame);
        }
    })
    .await
}

/// Holds an SSE stream open, passing each message to `on`.
async fn follow_messages(
    http: &reqwest::Client,
    url: &str,
    mut on: impl FnMut(&Message),
) -> Result<bool, DriverError> {
    let opened = tokio::time::timeout(
        CONNECT,
        http.get(url)
            // The client's request timeout would cut a stream short.
            .timeout(Duration::from_secs(365 * 24 * 3600))
            .header("Accept", "text/event-stream")
            .send(),
    )
    .await
    .map_err(|_| DriverError::Unreachable(format!("{url}: no answer")))?;
    let mut response =
        opened.map_err(|error| DriverError::Unreachable(format!("{url}: {error}")))?;
    if !response.status().is_success() {
        return Err(DriverError::Unreachable(format!(
            "{url}: {}",
            response.status()
        )));
    }
    let mut parser = Parser::default();
    loop {
        let chunk = match tokio::time::timeout(SILENCE, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            // Closed, failed, or silent past its keepalives: it's gone.
            Ok(Ok(None) | Err(_)) | Err(_) => return Ok(true),
        };
        for message in parser.feed(&String::from_utf8_lossy(&chunk)) {
            on(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn a_stream_is_followed_until_the_board_closes_it() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: hello\ndata: {\"boot_id\":\"b1\",\"seq\":3}\n\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            socket
                .write_all(b"event: update\ndata: {\"state\":\"trying\"}\n\n")
                .await
                .unwrap();
            // Then the board restarts.
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = seen.clone();
        let sink: PushSink = Arc::new(move |push| into.lock().unwrap().push(push));
        let ended = follow(&reqwest::Client::new(), &url, sink).await.unwrap();
        assert!(ended, "it ended: reconnect later");
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(
            seen.lock().unwrap()[1],
            StatusPush::Update {
                state: "trying".into()
            }
        );
    }

    #[test]
    fn hardware_frames_parse() {
        let mut parser = Parser::default();
        let messages = parser.feed(concat!(
            "event: hardware\ndata: {\"devices\":[{\"id\":\"imu\",\"class\":\"imu\",\"period_ms\":10,\"channels\":[{\"name\":\"acceleration.x\",\"unit\":\"m/s²\"}]},{\"id\":\"gps\",\"missing\":true}]}\n\n",
            "event: samples\ndata: {\"device\":\"imu\",\"samples\":[[1000,0.12],[11000,null]]}\n\n",
            "event: hardware-gone\ndata: {\"reason\":\"lemnosd: gone\"}\n\n",
        ));
        let frames: Vec<HardwareFrame> = messages.iter().filter_map(frame_of).collect();
        let HardwareFrame::Devices { devices } = &frames[0] else {
            panic!("devices first");
        };
        assert_eq!(
            (devices[0].period_ms, devices[0].channels[0].unit.as_str()),
            (10, "m/s²")
        );
        assert!(devices[1].missing);
        assert_eq!(
            frames[1],
            HardwareFrame::Samples {
                device: "imu".into(),
                samples: vec![(1000, vec![Some(0.12)]), (11000, vec![None])],
            }
        );
        assert_eq!(
            frames[2],
            HardwareFrame::Gone {
                reason: "lemnosd: gone".into()
            }
        );
    }

    #[tokio::test]
    async fn a_board_without_the_stream_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        drop(listener);
        let sink: PushSink = Arc::new(|_| {});
        assert!(follow(&reqwest::Client::new(), &url, sink).await.is_err());
    }

    #[test]
    fn messages_split_across_chunks_and_unknown_ones_are_skipped() {
        let mut parser = Parser::default();
        assert!(
            parser
                .feed("event: hello\ndata: {\"boot_id\":\"b1\",")
                .is_empty()
        );
        let messages = parser.feed("\"seq\":4}\n\n: keepalive\n\nid: 5\r\nevent: update\r\ndata: {\"state\":\"staging\"}\r\n\r\n");
        assert_eq!(messages.len(), 2);
        assert_eq!(
            push_of(&messages[0]),
            Some(StatusPush::Hello {
                boot_id: "b1".into(),
                newest_seq: Some(4)
            })
        );
        assert_eq!(
            push_of(&messages[1]),
            Some(StatusPush::Update {
                state: "staging".into()
            })
        );
        let event = parser.feed("id: 6\nevent: event\ndata: {\"t\":1,\"boot_id\":\"b1\",\"seq\":6,\"kind\":\"boot\",\"message\":\"boot 2\"}\n\n");
        let Some(StatusPush::Event(event)) = push_of(&event[0]) else {
            panic!("an event");
        };
        assert_eq!((event.seq, event.kind.as_str()), (Some(6), "boot"));
        assert_eq!(
            push_of(&Message {
                event: "later".into(),
                data: "{}".into()
            }),
            None
        );
        assert_eq!(
            push_of(&Message {
                event: "update".into(),
                data: "nope".into()
            }),
            None
        );
    }
}
