//! The board's action list, and one action in it that no part of Atlas names (C3, Actions).
//!
//! A board lists the actions it offers as one JSON document, read by running [`CATALOG`]. Each
//! entry gives a name and what it does, and may give a human label, its arguments with their
//! types, and whether it is long-running. A field the board does not give is absent; the app
//! shows it as not reported. Nothing about an entry is hard-coded in Atlas.
//!
//! The actions here are the node's own additions for the C3 brief. The others the node answers
//! (`reboot`, `locate`, `self-test`, `set-clock`, `status`, `support-bundle`) are in [`crate::names`].

use crate::slots::{Status, code};

/// The board's action list. Its answer is the JSON document. LOCK: Lemnos M8 (5dcf1a78) lets the
/// bridge's `call` take a method that returns through an out buffer; the Atlas lock must move to
/// that rev before a real board answers this. The name is ours, provisional, until Lemnos names the list.
pub const CATALOG: &str = "actions.list";
/// An action the board lists that no part of Atlas names. It takes a text label and a count of
/// steps (1 to 100), runs one step per tick, and reports how many it ran.
pub const DRY_RUN: &str = "vendor.dry-run";

/// The actions this module adds to the node.
pub const NAMES: &[&str] = &[CATALOG, DRY_RUN];

/// Largest number of steps a dry run takes.
pub const MAX_STEPS: u32 = 100;

/// The catalog the node answers with. `self-test` gives no label and no arguments, and
/// `vendor.dry-run` gives no label and no long-running flag in the sense of its type, so the
/// gaps are real ones the app must show.
pub const CATALOG_JSON: &str = r#"{"actions":[
{"name":"system.reboot.reboot","label":"Restart","description":"Restarts the board.","args":[],"long_running":false},
{"name":"locate","label":"Find it","description":"Blinks the board's locate light.","args":[],"long_running":false},
{"name":"self-test","description":"Checks the board's parts and reports passed or failed.","long_running":true},
{"name":"set-clock","label":"Set clock from this computer","description":"Sets the board's clock from the computer that asks.","args":[],"long_running":false},
{"name":"status","label":"Status","description":"Reports the boot and slot state.","args":[],"long_running":false},
{"name":"support-bundle","label":"Support bundle","description":"Returns the state, the slots and the last log lines as text.","args":[],"long_running":false},
{"name":"vendor.dry-run","description":"A dry run of the board's steps; reports how many it ran.","args":[{"name":"label","type":"text"},{"name":"steps","type":"int"}],"long_running":false}
]}"#;

/// Whether this module answers `name`.
#[must_use]
pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// How many ticks a run of `name` with `args` takes, for the actions this module answers.
#[must_use]
pub fn ticks(name: &str, args: &[u8]) -> Option<u32> {
    (name == DRY_RUN).then(|| dry_run_steps(args).unwrap_or(1))
}

/// The steps a dry run is asked for: its second word, an int from 1 to [`MAX_STEPS`].
fn dry_run_steps(args: &[u8]) -> Result<u32, Status> {
    let text = String::from_utf8_lossy(args);
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some(word) = words.get(1) else {
        return Err(Status::new(
            code::INVALID_ARGUMENT,
            "vendor.dry-run takes a label and a count of steps",
        ));
    };
    match word.parse::<u32>() {
        Ok(steps) if (1..=MAX_STEPS).contains(&steps) => Ok(steps),
        _ => Err(Status::new(
            code::INVALID_ARGUMENT,
            format!("steps must be a number from 1 to {MAX_STEPS}, not {word:?}"),
        )),
    }
}

/// Answers a run of `name` with `args`, once it has finished its ticks.
///
/// # Errors
///
/// A refusal in the node's own words, when the arguments do not fit.
pub fn apply(name: &str, args: &[u8]) -> Result<Vec<u8>, Status> {
    match name {
        // LOCK: Lemnos M8 (5dcf1a78): this answer reaches a real board once the Atlas lock moves to a rev
        // that has it; the simulated node answers as M8 does.
        CATALOG => Ok(CATALOG_JSON.as_bytes().to_vec()),
        DRY_RUN => {
            let steps = dry_run_steps(args)?;
            let text = String::from_utf8_lossy(args);
            let label = text.split_whitespace().next().unwrap_or_default();
            Ok(format!("dry run {label}: {steps} steps").into_bytes())
        }
        other => Err(Status::new(
            code::NOT_FOUND,
            format!("no simulated action {other}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_action_is_one_the_node_registers() {
        let listed: serde_json::Value = serde_json::from_str(CATALOG_JSON).unwrap();
        let entries = listed["actions"].as_array().unwrap();
        assert!(entries.len() >= 7);
        for entry in entries {
            let name = entry["name"].as_str().unwrap();
            assert!(
                crate::names::ACTIONS.contains(&name) || NAMES.contains(&name),
                "{name} is listed but not registered"
            );
        }
    }

    #[test]
    fn a_dry_run_takes_one_tick_per_step_and_refuses_a_bad_count() {
        assert_eq!(ticks(DRY_RUN, b"label 7"), Some(7));
        assert_eq!(ticks(DRY_RUN, b"label 0"), Some(1));
        assert!(apply(DRY_RUN, b"label 0").is_err());
        assert!(apply(DRY_RUN, b"label many").is_err());
        assert_eq!(apply(DRY_RUN, b"run 2").unwrap(), b"dry run run: 2 steps");
        assert_eq!(ticks("status", b""), None);
    }
}
