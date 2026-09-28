use serde::{Deserialize, Serialize};

use crate::{WriteProgress, WriteReport};

/// One JSON line from `atlas-helper` to the app. The helper appends these
/// to a progress file because elevation tools (pkexec, osascript, UAC)
/// do not reliably pass stdout back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum HelperMessage {
    Progress { progress: WriteProgress },
    Done { report: WriteReport },
    Error { message: String },
}

impl HelperMessage {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"error","message":"could not encode a helper message"}"#.to_string()
        });
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_single_lines() {
        let message = HelperMessage::Progress {
            progress: WriteProgress::Writing {
                input_done: 1,
                input_total: 2,
                written: 3,
            },
        };
        let line = message.to_line();
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(
            serde_json::from_str::<HelperMessage>(line.trim()).unwrap(),
            message
        );
    }
}
