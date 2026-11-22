//! The one way the UI asks for anything.
//!
//! A call has a name and JSON arguments and answers with JSON. Every call is async and none
//! waits on a device: a call that starts work on a device starts a job or a background read and
//! returns, and the result arrives as a topic ([`crate::view`]). Every failed call is written to
//! the app's log with what was asked (see [`redacted`]).
//!
//! The shell's calls are here. Each page adds its own in its module and its arm in [`route`].

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::core::Core;
use crate::settings::Settings;

/// Why a call failed, in words a person can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallError(pub String);

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CallError {}

impl From<String> for CallError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for CallError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

pub type CallResult = Result<Value, CallError>;

/// Reads a call's arguments into a type, naming the call when they do not fit.
pub fn args<T: DeserializeOwned>(name: &str, args: Value) -> Result<T, CallError> {
    serde_json::from_value(args).map_err(|error| CallError(format!("{name}: {error}")))
}

/// Runs one call and logs it when it fails.
pub async fn call(core: &Core, name: &str, arguments: Value) -> CallResult {
    let asked = redacted(&arguments);
    let result = route(core, name, arguments).await;
    if let Err(error) = &result {
        core.log().failed_call(name, &asked, &error.0);
    }
    result
}

async fn route(core: &Core, name: &str, arguments: Value) -> CallResult {
    match name {
        "snapshot" => Ok(json!({
            "version": crate::VERSION,
            "mode": core.mode().word(),
            "topics": core.topics(),
        })),
        "scan.pause" => {
            core.log().breadcrumb("scan paused");
            core.pause_scan();
            Ok(Value::Null)
        }
        "scan.resume" => {
            core.log().breadcrumb("scan resumed");
            core.resume_scan();
            Ok(Value::Null)
        }
        "settings.set" => {
            let settings: Settings = args(name, arguments)?;
            core.log().breadcrumb("settings changed");
            let saved = core.set_settings(settings).await?;
            serde_json::to_value(saved).map_err(|error| CallError(error.to_string()))
        }
        "log.ui_error" => {
            #[derive(serde::Deserialize)]
            struct UiError {
                place: String,
                message: String,
            }
            let UiError { place, message } = args(name, arguments)?;
            core.log().ui_error(&place, &message);
            Ok(Value::Null)
        }
        // The UI says its first frame is drawn. Written once, as a number to compare builds by.
        "log.first_paint" => {
            core.log().breadcrumb(&format!(
                "first paint {} ms after the process started",
                crate::started().elapsed().as_millis()
            ));
            Ok(Value::Null)
        }
        "log.breadcrumb" => {
            #[derive(serde::Deserialize)]
            struct Breadcrumb {
                message: String,
            }
            let Breadcrumb { message } = args(name, arguments)?;
            core.log().breadcrumb(&message);
            Ok(Value::Null)
        }
        #[cfg(feature = "sim")]
        _ if name.starts_with("sim.") => crate::sim::call(core, name, arguments).await,
        _ => Err(CallError(format!("no call named `{name}` in this build"))),
    }
}

/// The arguments as they are written to the log: a value under a key that names a secret is
/// replaced, at any depth. Calls never take a secret under another name.
#[must_use]
pub fn redacted(arguments: &Value) -> String {
    fn strip(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| {
                        let lower = key.to_ascii_lowercase();
                        let secret = ["password", "secret", "token", "private", "passphrase"]
                            .iter()
                            .any(|word| lower.contains(word));
                        (
                            key.clone(),
                            if secret {
                                Value::String("[redacted]".into())
                            } else {
                                strip(value)
                            },
                        )
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
            other => other.clone(),
        }
    }
    match arguments {
        Value::Null => String::new(),
        other => strip(other).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_never_written_to_the_log_at_any_depth() {
        let asked = redacted(&json!({
            "device": "a:b",
            "password": "hunter2",
            "nested": [{ "apiToken": "abc", "name": "x" }],
        }));
        assert!(
            !asked.contains("hunter2") && !asked.contains("abc"),
            "{asked}"
        );
        assert!(
            asked.contains("a:b") && asked.contains("\"name\":\"x\""),
            "{asked}"
        );
        assert_eq!(redacted(&Value::Null), "");
    }
}
