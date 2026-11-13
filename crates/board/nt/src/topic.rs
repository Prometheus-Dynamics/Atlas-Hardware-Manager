use orion_nt4::{Properties, Value};

/// A topic as a viewer or a local server holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Topic {
    /// The topic's name, such as `/SmartDashboard/speed`.
    pub name: String,
    /// The NT4 type string, such as `double` or `string[]`.
    pub type_name: String,
    /// The topic's properties. `null` never appears here: it removes a key.
    pub properties: Properties,
    /// The last value, if one has arrived.
    pub sample: Option<Sample>,
}

impl Topic {
    /// The `persistent` flag: kept by a server across restarts.
    pub fn persistent(&self) -> bool {
        self.flag("persistent")
    }

    /// The `retained` flag: kept announced after its publisher leaves.
    pub fn retained(&self) -> bool {
        self.flag("retained")
    }

    fn flag(&self, key: &str) -> bool {
        self.properties
            .get(key)
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    }
}

/// One value and the server's clock when it was stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Microseconds on the server's clock.
    pub t_us: i64,
    /// The value.
    pub value: Value,
}

/// What a [`Viewer`](crate::Viewer) reports, in the order it happened. The viewer's table is
/// already updated when an update is returned.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// The connection came up (`connected`) or went down (`reason`). Going down first reports
    /// each topic as [`Update::Unannounced`], because the table is emptied: the server
    /// announces its topics again after the next connect.
    Connection {
        /// Whether the connection is up.
        connected: bool,
        /// Why it went down.
        reason: Option<String>,
    },
    /// A topic exists (or exists again after a reconnect).
    Announced(Topic),
    /// A topic is gone.
    Unannounced(String),
    /// Properties of a topic changed. `update` holds the keys that changed; `null` removes one.
    Properties {
        /// The topic.
        name: String,
        /// The changed keys.
        update: Properties,
    },
    /// A new value for a topic.
    Value {
        /// The topic.
        name: String,
        /// The value and its time.
        sample: Sample,
    },
    /// A message the client could not use. The connection stays up.
    Warning(String),
}

/// Merges a property update into a topic's properties (`null` removes the key).
pub(crate) fn merge_properties(into: &mut Properties, update: &Properties) {
    for (key, value) in update {
        if value.is_null() {
            into.remove(key);
        } else {
            into.insert(key.clone(), value.clone());
        }
    }
}
