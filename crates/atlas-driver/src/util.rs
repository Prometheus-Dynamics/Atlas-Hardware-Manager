//! Small helpers every driver crate needs, in one place.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch, saturating at zero for clocks set before 1970.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A SHA-256 as 64 hex digits, in either case.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The host in a device address, unbracketed: a URL's
/// (`scheme://[user@]host[:port]/...`), `host:port`, or a bare host or IP
/// (an IPv6 one in brackets or not).
pub fn address_host(address: &str) -> Option<&str> {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']')?.0,
        // More than one colon: a bare IPv6 address.
        None if authority.matches(':').count() > 1 => authority,
        None => authority.split(':').next().unwrap_or(authority),
    };
    (!host.is_empty()).then_some(host)
}

/// `host:port` for a URL, an IPv6 host in brackets.
pub fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(feature = "http")]
/// The response when it is a 2xx; otherwise what the server said, for an
/// error: its status and its own words (an `{"error": ...}` field, or the
/// start of the body), so atlas.log shows why it refused. The caller names
/// the URL.
pub async fn answered(response: reqwest::Response) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(refusal(status.as_u16(), &body))
}

#[cfg(feature = "http")]
/// `answered <status> <reason>: <words>`, from a non-2xx reply; an HTML
/// error page is reduced to its text.
pub fn refusal(status: u16, body: &str) -> String {
    const WORDS: usize = 160;
    let words = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json.get("error")?.as_str().map(str::to_string))
        .unwrap_or_else(|| text_of(body));
    let mut words: String = words.chars().take(WORDS).collect();
    if words.chars().count() == WORDS {
        words.push('…');
    }
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .map_or(String::new(), |reason| format!(" {reason}"));
    if words.is_empty() {
        format!("answered {status}{reason}")
    } else {
        format!("answered {status}{reason}: {words}")
    }
}

#[cfg(feature = "http")]
/// A body's text on one line: tags, a page's head and repeated spaces out.
fn text_of(body: &str) -> String {
    let body = match (body.find("<body"), body.find("<head")) {
        (Some(start), _) => &body[start..],
        (None, Some(head)) => body
            .find("</head>")
            .map_or(&body[head..], |end| &body[end..]),
        _ => body,
    };
    let mut text = String::with_capacity(body.len());
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `value` rounded to `places` decimals.
pub fn round_to(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_sha256() {
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
        assert!(is_sha256_hex(&"aB".repeat(32)));
        assert!(!is_sha256_hex(&"ab".repeat(31)));
        assert!(!is_sha256_hex(&format!("{}zz", "ab".repeat(31))));
        assert_eq!(round_to(1.23456, 2), 1.23);
        assert!(now_ms() > 1_700_000_000_000);
    }

    #[test]
    fn the_host_of_an_address() {
        for (address, host) in [
            (
                "http://172.31.250.1:5899/.well-known/pd-device",
                Some("172.31.250.1"),
            ),
            ("http://user@raze.local/x?y#z", Some("raze.local")),
            ("http://[fe80::1]:80/x", Some("fe80::1")),
            ("[fe80::1]", Some("fe80::1")),
            ("fe80::1", Some("fe80::1")),
            ("10.0.0.2", Some("10.0.0.2")),
            ("10.0.0.2:22", Some("10.0.0.2")),
            ("usb:001-4", Some("usb")),
            ("http://", None),
            ("http://[fe80::1", None),
        ] {
            assert_eq!(address_host(address), host, "{address}");
        }
        assert_eq!(host_port("fe80::1", 22), "[fe80::1]:22");
        assert_eq!(host_port("raze.local", 22), "raze.local:22");
    }

    #[cfg(feature = "http")]
    #[test]
    fn a_refusal_says_what_the_server_said() {
        assert_eq!(
            refusal(409, r#"{"error":"a flash is running"}"#),
            "answered 409 Conflict: a flash is running"
        );
        assert_eq!(
            refusal(
                404,
                "<!DOCTYPE HTML>\n<html><head><title>Error response</title></head>\n<body><h1>Error</h1>\n<p>Message: File not found.</p></body></html>"
            ),
            "answered 404 Not Found: Error Message: File not found."
        );
        assert!(refusal(500, &"x".repeat(500)).ends_with(&format!(": {}…", "x".repeat(160))));
        assert_eq!(refusal(502, ""), "answered 502 Bad Gateway");
    }
}
