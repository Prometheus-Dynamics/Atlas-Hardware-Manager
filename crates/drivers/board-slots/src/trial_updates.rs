//! What a board says about trial updates (Lemnos BB1c, `system.slots.info`): one shared reading for
//! every Atlas consumer (the CLI's list, trial and install, the cycle tool, and any app).
//!
//! The wire value of `trial_block` is read into [`TrialBlock`] once, here. A value this build does not
//! know is kept as the board sent it and shown as unknown: it is never read as a reason, and no consumer
//! matches the string again.

use atlas_actions::{Action, BoardActions, Reported};
use serde_json::Value;

/// Why the boot loader will not run a trial on this board. The five known reasons are Lemnos's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialBlock {
    /// The boot loader's environment partition is missing.
    NoEnvPartition,
    /// The boot loader's environment cannot be read.
    EnvUnreadable,
    /// The environment is a FAT file, not the loader's own copy.
    FatEnvFile,
    /// The boot partition cannot be read.
    BootPartitionUnreadable,
    /// The boot loader does not carry the trial marker (`lemnos_env_rev=1`).
    LoaderNotUpdated,
    /// A value this build does not know, as the board sent it (its JSON text when it was not a string).
    Unknown(String),
}

impl TrialBlock {
    /// The typed reason for a wire value.
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "no-env-partition" => Self::NoEnvPartition,
            "env-unreadable" => Self::EnvUnreadable,
            "fat-env-file" => Self::FatEnvFile,
            "boot-partition-unreadable" => Self::BootPartitionUnreadable,
            "loader-not-updated" => Self::LoaderNotUpdated,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// The plain words for the reason, for a user: the typed reason, or the unknown value as sent.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::NoEnvPartition => "the boot loader's environment partition is missing".to_owned(),
            Self::EnvUnreadable => "the boot loader's environment cannot be read".to_owned(),
            Self::FatEnvFile => {
                "the environment is a FAT file, not the boot loader's copy".to_owned()
            }
            Self::BootPartitionUnreadable => "the boot partition cannot be read".to_owned(),
            Self::LoaderNotUpdated => "loader not updated".to_owned(),
            Self::Unknown(raw) => format!("unknown (as sent: {raw})"),
        }
    }
}

/// What the board says about trials. `NotReported`: the board does not carry BB1c's `trial_updates` key, so
/// nothing is said. `Clear`: no block is stated. `Blocked`: a typed reason, with the board's sentence and detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrialUpdates {
    NotReported,
    Clear,
    Blocked {
        block: TrialBlock,
        text: String,
        detail: String,
    },
}

impl TrialUpdates {
    /// The trial state in one `info` document. A `trial_block` that is neither null nor absent and not a
    /// string is blocked and unknown (never clear).
    #[must_use]
    pub fn of_info(document: &Value) -> Self {
        if document.get("trial_updates").is_none() {
            return Self::NotReported;
        }
        let text_of = |key: &str| {
            document
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        match document.get("trial_block") {
            None | Some(Value::Null) => Self::Clear,
            Some(Value::String(wire)) => Self::Blocked {
                block: TrialBlock::from_wire(wire),
                text: text_of("trial_block_text"),
                detail: text_of("trial_block_detail"),
            },
            Some(other) => Self::Blocked {
                block: TrialBlock::Unknown(other.to_string()),
                text: text_of("trial_block_text"),
                detail: text_of("trial_block_detail"),
            },
        }
    }

    /// The one line a user reads. A blocked board says that it cannot take trial updates, and why.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::NotReported => "not reported by the board (no trial_updates key)".to_owned(),
            Self::Clear => "trial updates: available".to_owned(),
            Self::Blocked {
                block,
                text,
                detail,
            } => {
                let mut words = format!("this board cannot take trial updates: {}", block.words());
                if !text.is_empty() {
                    words.push_str(&format!(" -- {text}"));
                }
                if !detail.is_empty() {
                    words.push_str(&format!(" ({detail})"));
                }
                words
            }
        }
    }

    /// Whether a trial must be refused up front: the board states a block.
    #[must_use]
    pub const fn blocks_trials(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }
}

/// The action that reads Lemnos's `system.slots` `info` (I5k, BB1c). Read with no arguments.
#[must_use]
pub fn info_action() -> Action {
    Action {
        name: "system.slots.info".to_owned(),
        label: Reported::NotReported,
        description: Reported::NotReported,
        args: Reported::Known(Vec::new()),
        long_running: Reported::Known(false),
    }
}

/// The board's trial updates, read now. A read that fails says nothing: the caller decides what that means.
pub async fn read(actions: &BoardActions) -> TrialUpdates {
    match actions.run(&info_action(), &[]).await {
        Ok(done) => serde_json::from_slice::<Value>(&done.output)
            .map_or(TrialUpdates::NotReported, |document| {
                TrialUpdates::of_info(&document)
            }),
        Err(_) => TrialUpdates::NotReported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_known_wire_reason_is_typed_and_any_other_value_stays_as_sent() {
        assert_eq!(
            TrialBlock::from_wire("no-env-partition"),
            TrialBlock::NoEnvPartition
        );
        assert_eq!(
            TrialBlock::from_wire("env-unreadable"),
            TrialBlock::EnvUnreadable
        );
        assert_eq!(
            TrialBlock::from_wire("fat-env-file"),
            TrialBlock::FatEnvFile
        );
        assert_eq!(
            TrialBlock::from_wire("boot-partition-unreadable"),
            TrialBlock::BootPartitionUnreadable
        );
        assert_eq!(
            TrialBlock::from_wire("loader-not-updated"),
            TrialBlock::LoaderNotUpdated
        );
        assert_eq!(
            TrialBlock::from_wire("a-future-reason"),
            TrialBlock::Unknown("a-future-reason".to_owned())
        );
    }

    #[test]
    fn a_document_without_trial_updates_says_nothing_and_null_or_absent_is_clear() {
        assert_eq!(
            TrialUpdates::of_info(&json!({"last_attempt": {}})),
            TrialUpdates::NotReported
        );
        assert_eq!(
            TrialUpdates::of_info(&json!({"trial_updates": true, "trial_block": null})),
            TrialUpdates::Clear
        );
        assert_eq!(
            TrialUpdates::of_info(&json!({"trial_updates": true})),
            TrialUpdates::Clear
        );
    }

    #[test]
    fn a_string_block_is_typed_and_a_non_string_block_is_unknown_never_clear() {
        assert_eq!(
            TrialUpdates::of_info(&json!({
                "trial_updates": true,
                "trial_block": "loader-not-updated",
                "trial_block_text": "the boot loader does not carry the trial marker",
                "trial_block_detail": "lemnos_env_rev=1 is absent"
            })),
            TrialUpdates::Blocked {
                block: TrialBlock::LoaderNotUpdated,
                text: "the boot loader does not carry the trial marker".to_owned(),
                detail: "lemnos_env_rev=1 is absent".to_owned(),
            }
        );
        match TrialUpdates::of_info(&json!({"trial_updates": true, "trial_block": 7})) {
            TrialUpdates::Blocked {
                block: TrialBlock::Unknown(raw),
                ..
            } => assert_eq!(raw, "7"),
            other => panic!("a numeric block must be blocked and unknown, got {other:?}"),
        }
    }

    #[test]
    fn the_line_names_the_typed_reason_and_an_unknown_value_as_sent() {
        let known = TrialUpdates::Blocked {
            block: TrialBlock::LoaderNotUpdated,
            text: String::new(),
            detail: String::new(),
        };
        assert_eq!(
            known.words(),
            "this board cannot take trial updates: loader not updated"
        );
        assert!(known.blocks_trials());
        let unknown = TrialUpdates::Blocked {
            block: TrialBlock::from_wire("new-reason"),
            text: String::new(),
            detail: String::new(),
        };
        assert!(
            unknown.words().contains("unknown (as sent: new-reason)"),
            "{}",
            unknown.words()
        );
        assert!(!TrialUpdates::Clear.blocks_trials());
    }
}
