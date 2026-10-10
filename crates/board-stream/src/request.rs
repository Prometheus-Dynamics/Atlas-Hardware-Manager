//! The one request a connection makes: `GET /stream?topics=..&after_seq=..`.

use std::io::BufRead;

/// What a viewer asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamRequest {
    /// New events (`events`) and update-state changes (`update`).
    pub events: bool,
    pub update: bool,
    /// Replay events after this seq first (`after_seq=`, or the SSE
    /// `Last-Event-ID` header when it reconnects).
    pub after_seq: Option<u64>,
}

/// Why a request isn't served, as an HTTP status line and a reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: &'static str,
    pub reason: &'static str,
}

const MAX_HEAD: usize = 8 * 1024;

fn refuse(status: &'static str, reason: &'static str) -> Refusal {
    Refusal { status, reason }
}

fn number(value: &str) -> Result<u64, Refusal> {
    if value.is_empty() || value.len() > 19 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refuse(
            "400 Bad Request",
            "after_seq must be a whole number",
        ));
    }
    value
        .parse()
        .map_err(|_| refuse("400 Bad Request", "after_seq must be a whole number"))
}

/// Reads the request line and headers (at most 8 KiB). Unknown query keys and
/// headers are ignored; nothing from the request is ever evaluated.
pub fn read_request(input: &mut impl BufRead) -> Result<StreamRequest, Refusal> {
    let mut head = Vec::new();
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .map_err(|_| refuse("400 Bad Request", "unreadable request"))?;
        head.push(read);
        if head.iter().sum::<usize>() > MAX_HEAD {
            return Err(refuse(
                "431 Request Header Fields Too Large",
                "request too large",
            ));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if read == 0 || line.is_empty() {
            break;
        }
        lines.push(line);
    }
    let mut first = lines.first().map(String::as_str).unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    if method != "GET" {
        return Err(refuse("405 Method Not Allowed", "only GET"));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/stream" {
        return Err(refuse("404 Not Found", "the stream is at /stream"));
    }
    let mut request = StreamRequest::default();
    let mut topics = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "topics" => topics = Some(value.to_string()),
            "after_seq" => request.after_seq = Some(number(value)?),
            _ => {}
        }
    }
    for line in &lines[1.min(lines.len())..] {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("last-event-id")
        {
            request.after_seq = Some(number(value.trim())?);
        }
    }
    // No topics: everything there is (today, events and the update state).
    let topics = topics.unwrap_or_else(|| "events,update".to_string());
    for topic in topics.split(',') {
        match topic {
            "events" => request.events = true,
            "update" => request.update = true,
            _ => {}
        }
    }
    if !request.events && !request.update {
        return Err(refuse(
            "400 Bad Request",
            "no topic this board has (events, update)",
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<StreamRequest, Refusal> {
        read_request(&mut text.as_bytes())
    }

    #[test]
    fn topics_after_seq_and_last_event_id() {
        let request =
            parse("GET /stream?topics=events&after_seq=12 HTTP/1.1\r\nHost: b\r\n\r\n").unwrap();
        assert_eq!(
            request,
            StreamRequest {
                events: true,
                update: false,
                after_seq: Some(12)
            }
        );
        let request = parse("GET /stream HTTP/1.1\r\nLast-Event-ID: 40\r\n\r\n").unwrap();
        assert!(request.events && request.update);
        assert_eq!(request.after_seq, Some(40), "a reconnecting viewer resumes");
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(
            parse("POST /stream HTTP/1.1\r\n\r\n").unwrap_err().status,
            "405 Method Not Allowed"
        );
        assert_eq!(
            parse("GET /status HTTP/1.1\r\n\r\n").unwrap_err().status,
            "404 Not Found"
        );
        assert_eq!(
            parse("GET /stream?topics=nope HTTP/1.1\r\n\r\n")
                .unwrap_err()
                .status,
            "400 Bad Request"
        );
        for bad in ["-1", "1;id", "", "99999999999999999999"] {
            let text = format!("GET /stream?after_seq={bad} HTTP/1.1\r\n\r\n");
            assert_eq!(parse(&text).unwrap_err().status, "400 Bad Request", "{bad}");
        }
        let huge = format!("GET /stream HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(9000));
        assert_eq!(
            parse(&huge).unwrap_err().status,
            "431 Request Header Fields Too Large"
        );
    }
}
