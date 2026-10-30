//! Recognising a device that serves no identity contract, from its
//! package's data (docs/devices-as-data.md): `fingerprints` tell that what
//! answers at an address is this model, `info` reads what it is, and `key`
//! names the value that keys it. Every read is a GET or a TCP connect to
//! the address it was found at; the data holds no command, no write and no
//! other host.
//!
//! The sources are a closed set ([`Source`]). A value that can't be read is
//! left out (or takes its `default`), never an error: only a device that
//! fits no fingerprint, or gives no key, isn't recognised.

use std::collections::BTreeMap;
use std::future::Future;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DeviceManifest;

/// One way to tell the model: a GET whose answer passes every check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub id: String,
    pub get: HttpGet,
    #[serde(default)]
    pub expect: Expect,
}

/// `GET http://<address>:<port><path>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpGet {
    pub port: u16,
    pub path: String,
}

/// What a fingerprint's answer must have. A check this Atlas doesn't know
/// fails the package's parse rather than being skipped, so a fingerprint
/// never matches more than its author meant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default)]
    pub status: Option<u16>,
    /// By header name (any case).
    #[serde(default)]
    pub header: BTreeMap<String, TextMatch>,
    #[serde(default)]
    pub body_contains: Vec<String>,
}

/// A text check: every rule given must hold.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextMatch {
    #[serde(default)]
    pub equals: Option<String>,
    #[serde(default)]
    pub starts_with: Option<String>,
    #[serde(default)]
    pub contains: Option<String>,
}

impl TextMatch {
    pub fn holds(&self, text: &str) -> bool {
        self.equals.as_ref().is_none_or(|want| text == want)
            && self
                .starts_with
                .as_ref()
                .is_none_or(|want| text.starts_with(want.as_str()))
            && self
                .contains
                .as_ref()
                .is_none_or(|want| text.contains(want.as_str()))
    }
}

/// One `info` value: where it comes from, then how it's trimmed.
///
/// Its name says where it goes: `model` and `name` name the device,
/// `versions.<x>` is a version, anything else an attribute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoValue {
    #[serde(flatten)]
    pub source: Source,
    /// The n-th whitespace-separated word (`2026.3.0` of
    /// `2026.3.0 (May 22 2026)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word: Option<usize>,
    /// Kept only when it starts with this, which is taken off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip_prefix: Option<String>,
    /// When the source gives nothing (or an empty string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Where a value comes from: a closed, read-only set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A GET's JSON answer, then `pick`: `$.a.b`.
    HttpJson {
        port: u16,
        path: String,
        pick: String,
    },
    /// A fact of the address it was found at, as the finder gives it
    /// (`team` from a `10.TE.AM.x` address).
    Address(String),
    /// Which of these ports accept a connection, by name, comma-separated
    /// in the listed order.
    TcpOpen(Vec<NamedPort>),
    /// Built from `{address}` and other values by name; left out while
    /// one is missing.
    Template(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedPort {
    pub name: String,
    pub port: u16,
}

/// An HTTP answer, as a [`Probe`] got it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HttpAnswer {
    pub status: u16,
    /// By lowercase name.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// How the engine reaches the address being recognised. `get` gives
/// `None` when nothing answered; `open` checks the ports at once.
pub trait Probe {
    fn get(&self, port: u16, path: &str) -> impl Future<Output = Option<HttpAnswer>> + Send;
    fn open(&self, ports: Vec<u16>) -> impl Future<Output = Vec<bool>> + Send;
}

/// Where something was found: its address, and what that address says
/// (`team`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub address: String,
    pub facts: BTreeMap<String, String>,
}

/// A device the data recognised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recognized {
    /// The fingerprint that held.
    pub fingerprint: String,
    /// The value of the manifest's `key`.
    pub key: String,
    pub model: Option<String>,
    pub name: Option<String>,
    pub versions: BTreeMap<String, String>,
    pub attributes: BTreeMap<String, String>,
    /// Every GET's answer, by port and path, for a driver that reads more
    /// from one than the data does.
    pub answers: BTreeMap<(u16, String), HttpAnswer>,
}

impl Recognized {
    pub fn answer(&self, port: u16, path: &str) -> Option<&HttpAnswer> {
        self.answers.get(&(port, path.to_string()))
    }
}

/// Why something wasn't recognised.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NotRecognized {
    /// No fingerprint's GET got an answer.
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    NotThisModel(String),
    #[error("{0}")]
    NoKey(String),
}

/// Why `answer` fails `expect`, or `None` when it passes.
pub fn check(expect: &Expect, answer: &HttpAnswer) -> Option<String> {
    if let Some(status) = expect.status
        && answer.status != status
    {
        return Some(format!("status {}, not {status}", answer.status));
    }
    for (name, rule) in &expect.header {
        match answer.headers.get(&name.to_ascii_lowercase()) {
            None => return Some(format!("no {name} header")),
            Some(value) if !rule.holds(value) => return Some(format!("{name}: {value}")),
            Some(_) => {}
        }
    }
    expect
        .body_contains
        .iter()
        .find(|text| !answer.body.contains(text.as_str()))
        .map(|text| format!("its page lacks {text}"))
}

/// `$.a.b` of `json`, as text: strings as they are, numbers and booleans
/// written out, anything else (or nothing there) `None`.
pub fn pick(json: &Value, path: &str) -> Option<String> {
    let rest = path.strip_prefix('$')?;
    let mut at = json;
    for field in rest.split('.').filter(|field| !field.is_empty()) {
        at = at.get(field)?;
    }
    match at {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// `template` with `{name}` filled from `values`, or `None` while one is
/// missing.
pub fn fill(template: &str, values: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        out.push_str(values.get(&rest[open + 1..close])?);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn shaped(value: &InfoValue, raw: Option<String>) -> Option<String> {
    let mut text = raw.filter(|text| !text.is_empty());
    if let Some(n) = value.word {
        text = text.and_then(|text| text.split_whitespace().nth(n).map(str::to_string));
    }
    if let Some(prefix) = &value.strip_prefix {
        text = text.and_then(|text| text.strip_prefix(prefix.as_str()).map(str::to_string));
    }
    text.filter(|text| !text.is_empty())
        .or_else(|| value.default.clone())
}

/// Recognises what answers at `seen` as `manifest`'s model: the first
/// fingerprint that holds, then every `info` value (the GETs and port
/// checks at once), keyed by the value `key` names.
pub async fn recognize(
    manifest: &DeviceManifest,
    probe: &impl Probe,
    seen: &Seen,
) -> Result<Recognized, NotRecognized> {
    let address = &seen.address;
    let what = manifest.display_name();
    let mut answers: BTreeMap<(u16, String), Option<HttpAnswer>> = BTreeMap::new();
    let mut held = None;
    let mut why = Vec::new();
    for fingerprint in &manifest.fingerprints {
        let target = (fingerprint.get.port, fingerprint.get.path.clone());
        if !answers.contains_key(&target) {
            let answer = probe.get(target.0, &target.1).await;
            answers.insert(target.clone(), answer);
        }
        match &answers[&target] {
            None => why.push(format!("nothing answers on port {}", target.0)),
            Some(answer) => match check(&fingerprint.expect, answer) {
                None => {
                    held = Some(fingerprint.id.clone());
                    break;
                }
                Some(reason) => why.push(reason),
            },
        }
    }
    let Some(fingerprint) = held else {
        let why = why.join("; ");
        return Err(if answers.values().all(Option::is_none) {
            NotRecognized::Unreachable(format!("{address}: {why}"))
        } else {
            NotRecognized::NotThisModel(format!("{address} is not a {what} ({why})"))
        });
    };

    // The info reads: every GET not made yet, and the ports, at once.
    let gets: Vec<(u16, String)> = manifest
        .info
        .values()
        .filter_map(|value| match &value.source {
            Source::HttpJson { port, path, .. } => Some((*port, path.clone())),
            _ => None,
        })
        .filter(|target| !answers.contains_key(target))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut ports: Vec<u16> = manifest
        .info
        .values()
        .filter_map(|value| match &value.source {
            Source::TcpOpen(named) => Some(named.iter().map(|n| n.port)),
            _ => None,
        })
        .flatten()
        .collect();
    ports.sort_unstable();
    ports.dedup();
    let fetches = futures::future::join_all(gets.iter().map(|(port, path)| probe.get(*port, path)));
    let opening = async {
        if ports.is_empty() {
            Vec::new()
        } else {
            probe.open(ports.clone()).await
        }
    };
    let (fetched, open) = futures::join!(fetches, opening);
    answers.extend(gets.into_iter().zip(fetched));
    let open: Vec<u16> = ports
        .iter()
        .zip(open)
        .filter_map(|(port, open)| open.then_some(*port))
        .collect();

    let mut values = BTreeMap::new();
    values.insert("address".to_string(), address.clone());
    let mut templates = Vec::new();
    for (name, value) in &manifest.info {
        let raw = match &value.source {
            Source::HttpJson {
                port,
                path,
                pick: at,
            } => answers
                .get(&(*port, path.clone()))
                .and_then(Option::as_ref)
                .filter(|answer| (200..300).contains(&answer.status))
                .and_then(|answer| serde_json::from_str::<Value>(&answer.body).ok())
                .and_then(|json| pick(&json, at)),
            Source::Address(fact) => seen.facts.get(fact).cloned(),
            Source::TcpOpen(named) => Some(
                named
                    .iter()
                    .filter(|n| open.contains(&n.port))
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            Source::Template(_) => {
                templates.push((name, value));
                continue;
            }
        };
        // A port list keeps its empty answer: "none open" is a reading.
        let kept = match &value.source {
            Source::TcpOpen(_) => raw,
            _ => shaped(value, raw),
        };
        if let Some(text) = kept {
            values.insert(name.clone(), text);
        }
    }
    // Templates may use each other: fill until nothing more fills.
    while !templates.is_empty() {
        let before = templates.len();
        templates.retain(|(name, value)| {
            let Source::Template(template) = &value.source else {
                return false;
            };
            match fill(template, &values) {
                Some(text) => {
                    if let Some(text) = shaped(value, Some(text)) {
                        values.insert((*name).clone(), text);
                    }
                    false
                }
                None => true,
            }
        });
        if templates.len() == before {
            for (name, value) in templates.drain(..) {
                if let Some(text) = value.default.clone() {
                    values.insert(name.clone(), text);
                }
            }
        }
    }
    values.remove("address");

    let key_name = manifest.key.as_deref().unwrap_or("serial");
    let key = values.get(key_name).cloned().ok_or_else(|| {
        NotRecognized::NoKey(format!("{address} gave no {key_name} to key it by"))
    })?;
    let mut found = Recognized {
        fingerprint,
        key,
        answers: answers
            .into_iter()
            .filter_map(|(target, answer)| answer.map(|answer| (target, answer)))
            .collect(),
        ..Recognized::default()
    };
    for (name, text) in values {
        match name.as_str() {
            "model" => found.model = Some(text),
            "name" => found.name = Some(text),
            _ => match name.strip_prefix("versions.") {
                Some(version) => {
                    found.versions.insert(version.to_string(), text);
                }
                None => {
                    found.attributes.insert(name, text);
                }
            },
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers from a table; ports listed in `open` accept connections.
    struct Table {
        answers: BTreeMap<(u16, &'static str), HttpAnswer>,
        open: Vec<u16>,
    }

    impl Probe for Table {
        async fn get(&self, port: u16, path: &str) -> Option<HttpAnswer> {
            self.answers
                .iter()
                .find(|((p, at), _)| *p == port && *at == path)
                .map(|(_, answer)| answer.clone())
        }

        async fn open(&self, ports: Vec<u16>) -> Vec<bool> {
            ports.iter().map(|port| self.open.contains(port)).collect()
        }
    }

    fn page(server: &str, body: &str) -> HttpAnswer {
        HttpAnswer {
            status: 200,
            headers: [("server".to_string(), server.to_string())].into(),
            body: body.into(),
        }
    }

    fn manifest(json: Value) -> DeviceManifest {
        serde_json::from_value(json).unwrap()
    }

    fn widget() -> DeviceManifest {
        manifest(serde_json::json!({
            "contract": 1,
            "model": "widget",
            "display_name": "Widget",
            "key": "team",
            "fingerprints": [{
                "id": "web",
                "get": { "port": 80, "path": "/" },
                "expect": { "status": 200, "header": { "Server": { "starts_with": "Wid" } },
                            "body_contains": ["<title>Widget</title>"] }
            }],
            "info": {
                "team": { "address": "team" },
                "model": { "http_json": { "port": 81, "path": "/v", "pick": "$.board.kind" }, "default": "Widget" },
                "versions.fw": { "http_json": { "port": 81, "path": "/v", "pick": "$.fw" }, "word": 0 },
                "versions.season": { "http_json": { "port": 81, "path": "/v", "pick": "$.season" }, "strip_prefix": "FRC" },
                "services": { "tcp_open": [{ "name": "ssh", "port": 22 }, { "name": "web", "port": 80 }] },
                "hostname": { "template": "widget-{team}" },
                "name": { "template": "{hostname}" },
                "manage_url": { "template": "http://{address}/" }
            }
        }))
    }

    fn seen() -> Seen {
        Seen {
            address: "10.63.90.2".into(),
            facts: [("team".to_string(), "6390".to_string())].into(),
        }
    }

    #[tokio::test]
    async fn the_data_recognises_and_reads_a_device() {
        let table = Table {
            answers: [
                ((80, "/"), page("Widgetd/2", "<title>Widget</title>")),
                ((81, "/v"), HttpAnswer {
                    status: 200,
                    body: r#"{"board":{"kind":"Widget Pro"},"fw":"1.2.3 (May 1)","season":"FRC2026"}"#.into(),
                    ..HttpAnswer::default()
                }),
            ]
            .into(),
            open: vec![80],
        };
        let found = recognize(&widget(), &table, &seen()).await.unwrap();
        assert_eq!(found.fingerprint, "web");
        assert_eq!(found.key, "6390");
        assert_eq!(found.model.as_deref(), Some("Widget Pro"));
        assert_eq!(found.name.as_deref(), Some("widget-6390"));
        assert_eq!(found.versions["fw"], "1.2.3");
        assert_eq!(found.versions["season"], "2026");
        assert_eq!(found.attributes["services"], "web");
        assert_eq!(found.attributes["hostname"], "widget-6390");
        assert_eq!(found.attributes["manage_url"], "http://10.63.90.2/");
        assert!(found.answer(81, "/v").is_some());
    }

    #[tokio::test]
    async fn missing_values_are_left_out_or_take_their_default() {
        let table = Table {
            answers: [((80, "/"), page("Widgetd", "<title>Widget</title>"))].into(),
            open: vec![],
        };
        let found = recognize(&widget(), &table, &seen()).await.unwrap();
        assert_eq!(found.model.as_deref(), Some("Widget"));
        assert!(found.versions.is_empty());
        assert_eq!(found.attributes["services"], "");
    }

    #[tokio::test]
    async fn another_device_or_none_is_not_recognised() {
        let other = Table {
            answers: [((80, "/"), page("nginx", "<title>Widget</title>"))].into(),
            open: vec![],
        };
        let refused = recognize(&widget(), &other, &seen()).await.unwrap_err();
        assert_eq!(
            refused,
            NotRecognized::NotThisModel("10.63.90.2 is not a Widget (Server: nginx)".into())
        );
        let nothing = Table {
            answers: BTreeMap::new(),
            open: vec![],
        };
        assert!(matches!(
            recognize(&widget(), &nothing, &seen()).await,
            Err(NotRecognized::Unreachable(_))
        ));
        let no_team = Table {
            answers: [((80, "/"), page("Widgetd", "<title>Widget</title>"))].into(),
            open: vec![],
        };
        let off_net = Seen {
            address: "192.168.1.2".into(),
            facts: BTreeMap::new(),
        };
        assert_eq!(
            recognize(&widget(), &no_team, &off_net).await.unwrap_err(),
            NotRecognized::NoKey("192.168.1.2 gave no team to key it by".into())
        );
    }

    #[test]
    fn an_unknown_check_fails_the_parse() {
        let parsed = serde_json::from_value::<DeviceManifest>(serde_json::json!({
            "contract": 1, "model": "x",
            "fingerprints": [{ "id": "a", "get": { "port": 80, "path": "/" },
                               "expect": { "body_matches": "x" } }]
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn picks_and_templates() {
        let json = serde_json::json!({ "a": { "b": 5, "c": true, "d": null } });
        assert_eq!(pick(&json, "$.a.b").as_deref(), Some("5"));
        assert_eq!(pick(&json, "$.a.c").as_deref(), Some("true"));
        assert_eq!(pick(&json, "$.a.d"), None);
        assert_eq!(pick(&json, "$.a"), None);
        assert_eq!(pick(&json, "a.b"), None);
        let values = [("team".to_string(), "6390".to_string())].into();
        assert_eq!(fill("r-{team}-x", &values).as_deref(), Some("r-6390-x"));
        assert_eq!(fill("{nope}", &values), None);
        assert_eq!(fill("{team", &values), None);
    }
}
