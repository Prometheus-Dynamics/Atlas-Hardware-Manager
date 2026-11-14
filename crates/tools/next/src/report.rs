//! What a command shows: words as they happen (text mode), or one JSON document at the end.

use std::io::Write;

use serde_json::{Map, Value, json};

pub struct Report<'a, W: Write> {
    out: &'a mut W,
    json: bool,
    messages: Vec<String>,
    fields: Map<String, Value>,
}

impl<'a, W: Write> Report<'a, W> {
    pub fn new(out: &'a mut W, json: bool) -> Self {
        Self {
            out,
            json,
            messages: Vec::new(),
            fields: Map::new(),
        }
    }

    /// Words for the person: printed at once in text mode, kept for the JSON document otherwise.
    pub fn say(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.json {
            self.messages.push(text);
        } else {
            let _ = writeln!(self.out, "{text}");
        }
    }

    /// A progress line (one per change): text mode only.
    pub fn progress(&mut self, text: impl Into<String>) {
        if !self.json {
            let _ = writeln!(self.out, "{}", text.into());
        }
    }

    /// A value for the JSON document.
    pub fn field(&mut self, key: &str, value: impl Into<Value>) {
        self.fields.insert(key.to_owned(), value.into());
    }

    /// Ends the command: writes the JSON document when asked for, and returns the status.
    pub fn finish(mut self, code: i32) -> i32 {
        if self.json {
            self.fields.insert("ok".into(), json!(code == 0));
            self.fields.insert("code".into(), json!(code));
            self.fields.insert("messages".into(), json!(self.messages));
            let _ = writeln!(self.out, "{}", Value::Object(self.fields));
        }
        code
    }
}
