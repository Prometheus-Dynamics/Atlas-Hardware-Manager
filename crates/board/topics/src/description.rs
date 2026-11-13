//! The device description as Lemnos sends it: the text of `call describe` (Lemnos `describe 1`).
//!
//! The grammar, as Lemnos's encoder writes it (`service/orion/src/description.rs` at e5c0ff0e, the
//! I6h commit; the encoder is the definition, and `docs/daemon.md` states the same format):
//!
//! ```text
//! text    = "describe 1" LF record *(LF record)          no final LF
//! record  = word *(SP key "=" value)                     one space between pairs
//! value   = quoted | bare
//! quoted  = DQUOTE *(char / escape) DQUOTE               escapes: \\ \" \n \r \t
//! bare    = 1*(any byte but SP and DQUOTE)               numbers, enum words, none, unknown
//! ```
//!
//! Records: `device`, `contract`, `method`, `item`. A word this reader does not know is an unknown
//! record and is counted; a key it does not know is ignored and counted. A missing key is not
//! reported for that field, never a default. A line that is not a record, a value that does not
//! fit its key, a repeated key, or a reference to a contract or method the text does not give
//! refuses the WHOLE description with the line number and the reason: a half-read description is
//! never shown as if it were complete.
//!
//! Bounds are named constants below, each with its reason. Nothing is allocated from a length the
//! text claims: the text's own length is checked first, and the records are pushed as they are read.

use std::collections::BTreeSet;
use std::fmt;

use crate::device::DeviceState;
use crate::value::{ItemKind, ReadingKind, UnitState};

/// The first line of a description this reader understands.
pub const VERSION_LINE: &str = "describe 1";
/// The format version this reader understands. Any other version is refused by name.
pub const VERSION: u64 = 1;
/// The largest description read. Reason: a description is one reply through the Orion bridge,
/// which carries at most 60 KiB (`ORION_REPLY_MAX` in Lemnos's `service/orion/src/calls.rs`).
pub const MAX_DESCRIPTION_BYTES: usize = 60 * 1024;
/// The longest line read. Reason: the longest record the encoder writes is an item or a method
/// with a few long texts (name, unit, group, params); 4 KiB holds those and refuses a line that
/// is not one record.
pub const MAX_LINE_BYTES: usize = 4 * 1024;
/// The most records after the version line, of every kind. Reason: each record the encoder
/// writes is at least about 70 bytes, so 60 KiB holds about 900; 4,096 is a ceiling no real text
/// reaches.
pub const MAX_RECORDS: usize = 4096;
/// The most keys one record carries. Reason: the encoder writes at most 18 keys in an item; 64
/// refuses a line of junk.
pub const MAX_FIELDS: usize = 64;
/// The most contracts a device serves. Reason: a device serves a few; Lemnos ships 32 contracts
/// in all, so 256 is a ceiling for one device.
pub const MAX_CONTRACTS: usize = 256;
/// The most methods, of every contract. Reason: a contract has a few methods; 2,048 is a ceiling
/// for one device.
pub const MAX_METHODS: usize = 2048;
/// The most items. Reason: a device with a full set of controls has tens of items; 2,048 is a
/// ceiling for one device.
pub const MAX_ITEMS: usize = 2048;

/// A refused description: the line it is refused at (`0` for the text as a whole) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The line the reason is about, counted from the version line as line 1. `0` for the text.
    pub line: usize,
    /// Why the text is refused, in words a person can act on.
    pub reason: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            f.write_str(&self.reason)
        } else {
            write!(f, "line {}: {}", self.line, self.reason)
        }
    }
}

/// One description, read whole. A figure the device does not give is `None` in its record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description {
    /// The device record: its label, bus, state, generation and the rest.
    pub device: DeviceRecord,
    /// The contracts the device serves, by index.
    pub contracts: Vec<ContractRecord>,
    /// The methods of every contract, in the order the text gives them.
    pub methods: Vec<MethodRecord>,
    /// The items, in the order the text gives them.
    pub items: Vec<ItemRecord>,
    /// Records of a word this reader does not know, counted and not read.
    pub unknown_records: u32,
    /// Keys of a known record that this reader does not know, counted and not read.
    pub unknown_keys: u32,
}

/// The device record (`device`): what the device is, as the board describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceRecord {
    /// The description generation: the number of the device's generation topic.
    pub generation: Option<u64>,
    /// The board's name for the device.
    pub label: Option<String>,
    /// The grouping its contracts give it (`sensor`, `power`, ...).
    pub kind: Option<String>,
    /// The bus the device is on.
    pub bus: Option<String>,
    /// The 7-bit address on its bus.
    pub address: Option<u8>,
    /// Where the device is, in words, for what an address cannot say.
    pub location: Option<String>,
    /// Whether it is running, retrying or off.
    pub state: Option<DeviceState>,
    /// Why it is retrying or off, or its last failure.
    pub reason: Option<String>,
    /// The device's lease period in seconds, when the description gives one. Not used for the
    /// session's lease yet (see `atlas-session`).
    // LOCK: Lemnos I6g: `lease_seconds` on the device record. Not written at e5c0ff0e (the encoder
    // has no such key), so it reads as not reported until the lock move that takes I6g.
    pub lease_seconds: Option<u64>,
}

/// A contract the device serves (`contract`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractRecord {
    /// The line of the record.
    pub line: usize,
    /// Its position: an item names its contract by this index.
    pub index: usize,
    /// Its tag, for example `name.thing/1`.
    pub tag: Option<String>,
    /// Its kind.
    pub kind: Option<String>,
    /// Its hash: the identity of the contract's shape.
    pub hash: Option<u64>,
    /// How many methods the text says the contract has. Checked against the method records.
    pub methods: Option<usize>,
}

/// A method of a contract (`method`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodRecord {
    /// The line of the record.
    pub line: usize,
    /// The contract it belongs to, by index.
    pub contract: usize,
    /// Its id within the contract.
    pub id: u32,
    /// Its name.
    pub name: Option<String>,
    /// Its parameters, each `name:shape`, separated by a space.
    pub params: Option<String>,
    /// The shape it returns.
    pub returns: Option<String>,
    /// Whether it returns through an out buffer.
    pub out_buffer: Option<bool>,
    /// Whether it is a long job.
    pub job: Option<bool>,
}

/// An item of a device (`item`).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRecord {
    /// The line of the record.
    pub line: usize,
    /// Whether it is a value, a control or an action.
    pub kind: Option<ItemKind>,
    /// Its name.
    pub name: Option<String>,
    /// Its unit, as text (empty when none).
    pub unit: Option<String>,
    /// Its unit state. Absent from a description that predates it: the reader of the device
    /// model decides it then (see `device`).
    pub unit_state: Option<UnitState>,
    /// The power of ten its integer figures are scaled by: raw × 10^exponent, in its unit.
    pub exponent: Option<i8>,
    /// The least and greatest raw value (scaled by the exponent).
    pub min: Option<i64>,
    /// See `min`.
    pub max: Option<i64>,
    /// The least real value in the unit. Used in place of `min` when given.
    pub real_min: Option<f64>,
    /// The greatest real value in the unit. Used in place of `max` when given.
    pub real_max: Option<f64>,
    /// The real step in the unit. Used in place of `step` when given.
    pub real_step: Option<f64>,
    /// The step, raw (scaled by the exponent).
    pub step: Option<i64>,
    /// The lower warning limit, raw (scaled by the exponent).
    pub warn_low: Option<i64>,
    /// The upper warning limit, raw (scaled by the exponent).
    pub warn_high: Option<i64>,
    /// The contract that serves it, by index.
    pub contract: Option<usize>,
    /// The method it is read or set through, by id within its contract.
    pub method: Option<u32>,
    /// The topic field its value is published in. `None` when it is not published.
    pub field: Option<String>,
    /// What the reading is.
    pub reading: Option<ReadingKind>,
    /// The group (a three-axis vector) it is one part of.
    pub group: Option<String>,
}

/// What the device model holds of a device's description, and why it holds that.
#[derive(Clone, Debug, PartialEq)]
pub enum DescriptionState {
    /// Not read yet on this connection.
    Pending,
    /// Read, and whole.
    Described(Box<Description>),
    /// The board answered, and the text is refused: why.
    Unreadable(Refusal),
    /// The board did not give a description (refused the call, or the device is not running):
    /// its refusal, in its words.
    NotAvailable(String),
}

/// Reads a description from bytes. The bytes must be text.
///
/// # Errors
///
/// A [`Refusal`] for text that is not a description this reader understands.
pub fn parse_bytes(bytes: &[u8]) -> Result<Description, Refusal> {
    let text = std::str::from_utf8(bytes).map_err(|_| Refusal {
        line: 0,
        reason: "the description is not text".into(),
    })?;
    parse(text)
}

/// Reads a description text: the version line, then its records.
///
/// # Errors
///
/// A [`Refusal`]: an unknown format version (named, with the versions understood), a text over
/// [`MAX_DESCRIPTION_BYTES`], a malformed line, a value that does not fit its key, a repeated key,
/// too many records, or a reference the text does not resolve.
pub fn parse(text: &str) -> Result<Description, Refusal> {
    if text.len() > MAX_DESCRIPTION_BYTES {
        return Err(Refusal {
            line: 0,
            reason: format!(
                "the description is {} bytes; at most {MAX_DESCRIPTION_BYTES} are read",
                text.len()
            ),
        });
    }
    let mut lines = text.split('\n');
    check_version(lines.next().unwrap_or(""))?;

    let mut description = Description::default();
    let mut device_seen = false;
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        if line.is_empty() {
            return Err(Refusal {
                line: number,
                reason: "an empty line: the encoder writes no empty line and no final line break"
                    .into(),
            });
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(Refusal {
                line: number,
                reason: format!(
                    "the line is {} bytes; at most {MAX_LINE_BYTES} are read",
                    line.len()
                ),
            });
        }
        if number - 1 > MAX_RECORDS {
            return Err(Refusal {
                line: number,
                reason: format!("more than {MAX_RECORDS} records"),
            });
        }
        let record = read_record(number, line)?;
        match record.word.as_str() {
            "device" => {
                if device_seen {
                    return Err(record.refuse("a second device record"));
                }
                device_seen = true;
                description.unknown_keys += record.unknown_keys(DEVICE_KEYS);
                description.device = device_record(&record)?;
            }
            "contract" => {
                if description.contracts.len() == MAX_CONTRACTS {
                    return Err(record.refuse(format!("more than {MAX_CONTRACTS} contracts")));
                }
                description.unknown_keys += record.unknown_keys(CONTRACT_KEYS);
                description.contracts.push(contract_record(&record)?);
            }
            "method" => {
                if description.methods.len() == MAX_METHODS {
                    return Err(record.refuse(format!("more than {MAX_METHODS} methods")));
                }
                description.unknown_keys += record.unknown_keys(METHOD_KEYS);
                description.methods.push(method_record(&record)?);
            }
            "item" => {
                if description.items.len() == MAX_ITEMS {
                    return Err(record.refuse(format!("more than {MAX_ITEMS} items")));
                }
                description.unknown_keys += record.unknown_keys(ITEM_KEYS);
                description.items.push(item_record(&record)?);
            }
            _ => description.unknown_records += 1,
        }
    }
    if !device_seen {
        return Err(Refusal {
            line: 0,
            reason: "no device record: the description does not say what the device is".into(),
        });
    }
    link(description)
}

/// The first line: `describe <version>`, and only the version this reader understands.
fn check_version(first: &str) -> Result<(), Refusal> {
    let mut parts = first.split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("describe"), Some(version), None) => match decimal_u64(version) {
            Some(VERSION) => Ok(()),
            Some(found) => Err(Refusal {
                line: 1,
                reason: format!(
                    "the description is format version {found}; this Atlas reads version {VERSION} ({VERSION_LINE}) only"
                ),
            }),
            None => Err(Refusal {
                line: 1,
                reason: format!("the format version {version:?} is not a number"),
            }),
        },
        _ => Err(Refusal {
            line: 1,
            reason: format!(
                "not a device description: the first line must be {VERSION_LINE:?}, not {:?}",
                first.chars().take(40).collect::<String>()
            ),
        }),
    }
}

/// The checks that need the whole text: contract indexes, method references and counts, and
/// the contract of each item.
fn link(mut description: Description) -> Result<Description, Refusal> {
    description.contracts.sort_by_key(|contract| contract.index);
    for (position, contract) in description.contracts.iter().enumerate() {
        if contract.index != position {
            return Err(Refusal {
                line: contract.line,
                reason: "contract indexes must be 0, 1, 2 and so on, with none missing or repeated"
                    .into(),
            });
        }
    }
    let contracts = description.contracts.len();
    let mut seen = BTreeSet::new();
    for method in &description.methods {
        if method.contract >= contracts {
            return Err(Refusal {
                line: method.line,
                reason: format!(
                    "the method names contract {}, which the description does not give",
                    method.contract
                ),
            });
        }
        if !seen.insert((method.contract, method.id)) {
            return Err(Refusal {
                line: method.line,
                reason: format!(
                    "method {} of contract {} is given twice",
                    method.id, method.contract
                ),
            });
        }
    }
    for contract in &description.contracts {
        if let Some(count) = contract.methods {
            let have = description
                .methods
                .iter()
                .filter(|method| method.contract == contract.index)
                .count();
            if have != count {
                return Err(Refusal {
                    line: contract.line,
                    reason: format!(
                        "the contract says {count} methods; the description gives {have}"
                    ),
                });
            }
        }
    }
    for item in &description.items {
        if let Some(index) = item.contract
            && index >= contracts
        {
            return Err(Refusal {
                line: item.line,
                reason: format!(
                    "the item names contract {index}, which the description does not give"
                ),
            });
        }
        match (item.contract, item.method) {
            (Some(contract), Some(method)) => {
                if !seen.contains(&(contract, method)) {
                    return Err(Refusal {
                        line: item.line,
                        reason: format!(
                            "the item names method {method} of contract {contract}, which the description does not give"
                        ),
                    });
                }
            }
            (None, Some(_)) => {
                return Err(Refusal {
                    line: item.line,
                    reason: "the item names a method but no contract".into(),
                });
            }
            _ => {}
        }
    }
    Ok(description)
}

/// The keys each record kind knows. Any other key is ignored and counted.
const DEVICE_KEYS: &[&str] = &[
    "generation",
    "label",
    "kind",
    "bus",
    "address",
    "location",
    "state",
    "reason",
    "lease_seconds",
];
const CONTRACT_KEYS: &[&str] = &["index", "tag", "kind", "hash", "methods"];
const METHOD_KEYS: &[&str] = &[
    "contract",
    "id",
    "name",
    "params",
    "returns",
    "out_buffer",
    "job",
];
const ITEM_KEYS: &[&str] = &[
    "kind",
    "name",
    "unit",
    "unit_state",
    "exponent",
    "min",
    "max",
    "real_min",
    "real_max",
    "real_step",
    "step",
    "warn_low",
    "warn_high",
    "contract",
    "method",
    "field",
    "reading",
    "group",
];

fn device_record(record: &Record) -> Result<DeviceRecord, Refusal> {
    Ok(DeviceRecord {
        generation: record.number("generation", decimal_u64)?,
        label: record.text("label")?,
        kind: record.text("kind")?,
        bus: record.text("bus")?,
        address: record.number("address", hex_u8)?,
        location: record.text("location")?,
        state: record.word("state")?.and_then(DeviceState::from_word),
        reason: record.text("reason")?,
        lease_seconds: record.number("lease_seconds", decimal_u64)?,
    })
}

fn contract_record(record: &Record) -> Result<ContractRecord, Refusal> {
    let index = record
        .number("index", decimal_usize)?
        .ok_or_else(|| record.refuse("the contract record has no index"))?;
    Ok(ContractRecord {
        line: record.line,
        index,
        tag: record.text("tag")?,
        kind: record.text("kind")?,
        hash: record.number("hash", hex_u64)?,
        methods: record.number("methods", decimal_usize)?,
    })
}

fn method_record(record: &Record) -> Result<MethodRecord, Refusal> {
    let contract = record
        .number("contract", decimal_usize)?
        .ok_or_else(|| record.refuse("the method record names no contract"))?;
    let id = record
        .number("id", decimal_u32)?
        .ok_or_else(|| record.refuse("the method record has no id"))?;
    Ok(MethodRecord {
        line: record.line,
        contract,
        id,
        name: record.text("name")?,
        params: record.text("params")?,
        returns: record.text("returns")?,
        out_buffer: record.flag("out_buffer")?,
        job: record.flag("job")?,
    })
}

fn item_record(record: &Record) -> Result<ItemRecord, Refusal> {
    Ok(ItemRecord {
        line: record.line,
        kind: record.word("kind")?.and_then(ItemKind::from_word),
        name: record.text("name")?,
        unit: record.text("unit")?,
        unit_state: record.word("unit_state")?.and_then(UnitState::from_word),
        exponent: record.number("exponent", decimal_i8)?,
        min: record.number("min", decimal_i64)?,
        max: record.number("max", decimal_i64)?,
        real_min: record.number("real_min", real)?,
        real_max: record.number("real_max", real)?,
        real_step: record.number("real_step", real)?,
        step: record.number("step", decimal_i64)?,
        warn_low: record.number("warn_low", decimal_i64)?,
        warn_high: record.number("warn_high", decimal_i64)?,
        contract: record.number("contract", decimal_usize)?,
        method: record.number("method", decimal_u32)?,
        field: record.text("field")?,
        reading: record.word("reading")?.and_then(ReadingKind::from_word),
        group: record.text("group")?,
    })
}

/// A value of a record, as written.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    /// Text, unescaped from its quotes.
    Quoted(String),
    /// A word or a number, as written.
    Bare(String),
}

/// One record: its word, and its pairs in the order they came.
#[derive(Clone, Debug)]
struct Record {
    line: usize,
    word: String,
    fields: Vec<(String, Value)>,
}

/// The word a figure or a text takes when the device gives none.
const NONE: &str = "none";
/// The word a figure takes when the device does not give it.
const UNKNOWN: &str = "unknown";

impl Record {
    fn refuse(&self, reason: impl Into<String>) -> Refusal {
        Refusal {
            line: self.line,
            reason: reason.into(),
        }
    }

    fn value(&self, key: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// How many of its keys the record kind does not know.
    fn unknown_keys(&self, known: &[&str]) -> u32 {
        let count = self
            .fields
            .iter()
            .filter(|(key, _)| !known.contains(&key.as_str()))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Text: quoted, or the bare word `none` for none.
    fn text(&self, key: &str) -> Result<Option<String>, Refusal> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Quoted(text)) => Ok(Some(text.clone())),
            Some(Value::Bare(word)) if word == NONE => Ok(None),
            Some(Value::Bare(word)) => {
                Err(self.refuse(format!("{key} is not quoted text: {word}")))
            }
        }
    }

    /// A bare word. A word this reader does not know is passed on for the caller to map to none.
    fn word(&self, key: &str) -> Result<Option<&str>, Refusal> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Bare(word)) => Ok(Some(word.as_str())),
            Some(Value::Quoted(_)) => Err(self.refuse(format!("{key} is a word, not quoted text"))),
        }
    }

    /// A flag: `true` or `false`; any other word reads as not reported.
    fn flag(&self, key: &str) -> Result<Option<bool>, Refusal> {
        match self.word(key)? {
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            _ => Ok(None),
        }
    }

    /// A number in the form `parse` takes. `unknown` and `none` are not reported.
    fn number<T>(&self, key: &str, parse: fn(&str) -> Option<T>) -> Result<Option<T>, Refusal> {
        match self.value(key) {
            None => Ok(None),
            Some(Value::Bare(word)) if word == UNKNOWN || word == NONE => Ok(None),
            Some(Value::Bare(word)) => parse(word)
                .map(Some)
                .ok_or_else(|| self.refuse(format!("{key} is not a valid number: {word}"))),
            Some(Value::Quoted(_)) => {
                Err(self.refuse(format!("{key} is a number, not quoted text")))
            }
        }
    }
}

/// Reads one record line: a word, then `key=value` pairs, one space apart.
fn read_record(line: usize, text: &str) -> Result<Record, Refusal> {
    let refuse = |reason: String| Refusal { line, reason };
    let word_end = text.find(' ').unwrap_or(text.len());
    let word = &text[..word_end];
    if word.is_empty() || !word.bytes().all(is_key_byte) {
        return Err(refuse(format!(
            "no record word: {:?}",
            text.chars().take(24).collect::<String>()
        )));
    }
    let mut fields: Vec<(String, Value)> = Vec::new();
    let mut rest = &text[word_end..];
    while !rest.is_empty() {
        let after = rest
            .strip_prefix(' ')
            .ok_or_else(|| refuse("pairs are separated by one space".into()))?;
        if after.is_empty() || after.starts_with(' ') {
            return Err(refuse("pairs are separated by exactly one space".into()));
        }
        let key_end = after
            .bytes()
            .position(|byte| !is_key_byte(byte))
            .unwrap_or(after.len());
        let key = &after[..key_end];
        if key.is_empty() || !after[key_end..].starts_with('=') {
            return Err(refuse("expected key=value".into()));
        }
        let body = &after[key_end + 1..];
        let (value, remainder) = if let Some(quoted) = body.strip_prefix('"') {
            let (text, consumed) = unquote(quoted).map_err(|reason| refuse(reason.into()))?;
            let remainder = &quoted[consumed..];
            if !remainder.is_empty() && !remainder.starts_with(' ') {
                return Err(refuse(format!(
                    "text of {key} is followed by more than a space"
                )));
            }
            (Value::Quoted(text), remainder)
        } else {
            let end = body.find(' ').unwrap_or(body.len());
            let bare = &body[..end];
            if bare.is_empty() {
                return Err(refuse(format!("{key} has no value")));
            }
            if bare.contains('"') {
                return Err(refuse(format!("{key} has a quote inside a bare value")));
            }
            (Value::Bare(bare.to_owned()), &body[end..])
        };
        if fields.iter().any(|(name, _)| name == key) {
            return Err(refuse(format!("{key} is given twice")));
        }
        if fields.len() == MAX_FIELDS {
            return Err(refuse(format!("more than {MAX_FIELDS} keys in one record")));
        }
        fields.push((key.to_owned(), value));
        rest = remainder;
    }
    Ok(Record {
        line,
        word: word.to_owned(),
        fields,
    })
}

/// The text inside quotes, unescaped, and how many bytes of `after_quote` it took through the
/// closing quote. The escapes are the encoder's: `\\ \" \n \r \t`. Anything else is refused.
fn unquote(after_quote: &str) -> Result<(String, usize), &'static str> {
    let mut out = String::new();
    let mut chars = after_quote.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' => return Ok((out, index + 1)),
            '\\' => {
                let (_, escaped) = chars.next().ok_or("text is not closed")?;
                out.push(match escaped {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    _ => return Err("text has an escape the format does not have"),
                });
            }
            '\r' => return Err("text has a raw carriage return; the encoder writes \\r"),
            character => out.push(character),
        }
    }
    Err("text is not closed")
}

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn decimal_u64(text: &str) -> Option<u64> {
    is_digits(text).then(|| text.parse().ok()).flatten()
}

fn decimal_u32(text: &str) -> Option<u32> {
    decimal_u64(text).and_then(|value| u32::try_from(value).ok())
}

fn decimal_usize(text: &str) -> Option<usize> {
    decimal_u64(text).and_then(|value| usize::try_from(value).ok())
}

fn decimal_i64(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    is_digits(digits).then(|| text.parse().ok()).flatten()
}

fn decimal_i8(text: &str) -> Option<i8> {
    decimal_i64(text).and_then(|value| i8::try_from(value).ok())
}

/// A hash: `0x` and up to sixteen hex digits.
fn hex_u64(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("0x")?;
    if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

/// An address: `0x` and up to two hex digits (the 7-bit I2C address).
fn hex_u8(text: &str) -> Option<u8> {
    hex_u64(text).and_then(|value| u8::try_from(value).ok())
}

/// A real value: an optional minus, digits, and optionally a point and more digits. Not NaN and
/// not infinite: a description never says so, and a figure that is not finite is refused.
fn real(text: &str) -> Option<f64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let well_formed = match digits.split_once('.') {
        Some((whole, fraction)) => is_digits(whole) && is_digits(fraction),
        None => is_digits(digits),
    };
    if !well_formed {
        return None;
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimal valid text: a version line and a device record.
    const MINIMAL: &str = "describe 1\ndevice";

    #[test]
    fn the_minimal_text_reads_and_every_optional_field_is_absent() {
        let description = parse(MINIMAL).expect("reads");
        assert_eq!(description.device, DeviceRecord::default());
        assert!(description.contracts.is_empty());
        assert!(description.items.is_empty());
        assert_eq!(
            (description.unknown_records, description.unknown_keys),
            (0, 0)
        );
    }

    #[test]
    fn an_unknown_version_is_refused_by_name_with_the_one_understood() {
        let refusal = parse("describe 2\ndevice").unwrap_err();
        assert_eq!(refusal.line, 1);
        assert!(refusal.reason.contains("version 2"), "{refusal}");
        assert!(refusal.reason.contains("version 1"), "{refusal}");
        let refusal = parse("describe x\ndevice").unwrap_err();
        assert!(refusal.reason.contains("not a number"), "{refusal}");
        let refusal = parse("hello\ndevice").unwrap_err();
        assert!(refusal.reason.contains("describe 1"), "{refusal}");
    }

    #[test]
    fn a_text_without_a_device_record_is_refused_as_a_whole() {
        let refusal = parse("describe 1\ncontract index=0").unwrap_err();
        assert_eq!(refusal.line, 0);
        assert!(refusal.reason.contains("no device record"), "{refusal}");
    }

    #[test]
    fn a_second_device_record_is_refused() {
        let refusal = parse("describe 1\ndevice\ndevice").unwrap_err();
        assert_eq!(refusal.line, 3);
    }

    #[test]
    fn empty_lines_and_a_final_line_break_are_refused() {
        assert_eq!(parse("describe 1\ndevice\n").unwrap_err().line, 3);
        assert_eq!(parse("describe 1\n\ndevice").unwrap_err().line, 2);
    }

    #[test]
    fn a_malformed_line_refuses_the_whole_text_at_its_line() {
        for bad in [
            "device label",
            "device label=",
            "device =x",
            "device label=\"open",
            "device label=\"a\"b=1",
            "device label=a\"b",
            "device  label=\"a\"",
            "device label=\"a\\q\"",
            "device label=\"a\rb\"",
            "device=x",
            "device generation=1 generation=2",
            "device generation=abc",
            "device generation=\"1\"",
            "device address=0x1ff",
            "device address=68",
        ] {
            let text = format!("describe 1\n{bad}");
            let refusal = parse(&text).expect_err(bad);
            assert_eq!(refusal.line, 2, "{bad}: {refusal}");
        }
    }

    #[test]
    fn an_unknown_record_and_unknown_keys_are_ignored_and_counted() {
        let text = "describe 1\ndevice label=\"a\" future=\"x\" newer=3\nwhatever kind=\"y\"\nitem kind=value name=\"n\" later=1";
        let description = parse(text).expect("reads");
        assert_eq!(description.device.label.as_deref(), Some("a"));
        assert_eq!(description.unknown_keys, 3);
        assert_eq!(description.unknown_records, 1);
        assert_eq!(description.items[0].name.as_deref(), Some("n"));
    }

    #[test]
    fn a_record_over_the_line_bound_is_refused() {
        let long = "a".repeat(MAX_LINE_BYTES);
        let refusal = parse(&format!("describe 1\ndevice label=\"{long}\"")).unwrap_err();
        assert_eq!(refusal.line, 2);
        assert!(refusal.reason.contains("bytes"), "{refusal}");
    }

    #[test]
    fn a_text_over_the_size_bound_is_refused_before_it_is_read() {
        let text = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
        let refusal = parse(&text).unwrap_err();
        assert_eq!(refusal.line, 0);
        assert!(refusal.reason.contains("bytes"), "{refusal}");
    }

    #[test]
    fn too_many_records_are_refused() {
        let mut text = String::from("describe 1\ndevice");
        for _ in 0..MAX_RECORDS {
            text.push_str("\nwhatever");
        }
        let refusal = parse(&text).unwrap_err();
        assert!(refusal.reason.contains("records"), "{refusal}");
    }

    #[test]
    fn too_many_items_are_refused_at_the_one_past_the_bound() {
        let mut text = String::from("describe 1\ndevice");
        for _ in 0..=MAX_ITEMS {
            text.push_str("\nitem kind=value");
        }
        let refusal = parse(&text).unwrap_err();
        assert!(refusal.reason.contains("items"), "{refusal}");
    }

    #[test]
    fn a_reference_the_text_does_not_resolve_is_refused() {
        let contract_missing = "describe 1\ndevice\nitem kind=value contract=3 method=1";
        assert!(parse(contract_missing).is_err());
        let method_missing =
            "describe 1\ndevice\ncontract index=0 methods=0\nitem contract=0 method=1";
        assert!(parse(method_missing).is_err());
        let count_wrong = "describe 1\ndevice\ncontract index=0 methods=2\nmethod contract=0 id=1";
        assert!(parse(count_wrong).is_err());
        let index_gap = "describe 1\ndevice\ncontract index=1";
        assert!(parse(index_gap).is_err());
    }

    #[test]
    fn unknown_figures_and_options_are_not_reported_and_words_are_not_guessed() {
        let text = "describe 1\ndevice generation=unknown address=none state=dancing reason=\"\"\nitem kind=dial min=unknown real_min=none unit_state=maybe reading=dial";
        let description = parse(text).expect("reads");
        assert_eq!(description.device.generation, None);
        assert_eq!(description.device.address, None);
        assert_eq!(description.device.state, None);
        assert_eq!(description.device.reason.as_deref(), Some(""));
        let item = &description.items[0];
        assert_eq!(item.kind, None);
        assert_eq!(item.min, None);
        assert_eq!(item.real_min, None);
        assert_eq!(item.unit_state, None);
        assert_eq!(item.reading, None);
    }

    #[test]
    fn a_text_value_of_none_is_text_and_the_bare_word_is_the_option() {
        let description = parse("describe 1\ndevice label=\"none\" bus=none").expect("reads");
        assert_eq!(description.device.label.as_deref(), Some("none"));
        assert_eq!(description.device.bus, None);
    }

    #[test]
    fn real_values_refuse_what_the_encoder_never_writes() {
        assert_eq!(real("-1.5"), Some(-1.5));
        assert_eq!(real("2"), Some(2.0));
        assert_eq!(real("0.25"), Some(0.25));
        for bad in ["NaN", "inf", "-inf", "1e5", "2.", ".5", "+1", "--1"] {
            assert_eq!(real(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_hash_and_an_address_are_hex_only() {
        assert_eq!(hex_u64("0x00000000000000ab"), Some(0xab));
        assert_eq!(hex_u64("0x"), None);
        assert_eq!(hex_u64("ab"), None);
        assert_eq!(hex_u8("0x44"), Some(0x44));
        assert_eq!(hex_u8("0x100"), None);
    }
}
