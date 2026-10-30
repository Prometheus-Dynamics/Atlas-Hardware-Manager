//! The built `slot-init` binary, as the slots writer runs it: every command
//! its doc header lists is one it takes, and `merge-layers` makes a real
//! Gaia build's layered bundle a bootable slot.json.

use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_slot-init");
/// A layered bundle's slot.json and layers/layers.json, as Gaia built them
/// (HeliOS dev.795, the bundle the bench Raze refused without merge-layers).
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("SLOT_SYS", "/nonexistent-sys")
        .env("SLOT_DEV", "/nonexistent-dev")
        .output()
        .unwrap()
}

#[test]
fn every_documented_command_is_taken() {
    let source = include_str!("../src/main.rs");
    let mut checked = 0;
    for line in source.lines() {
        let Some(rest) = line.strip_prefix("//!   slot-init ") else {
            continue;
        };
        // The signature ends where the description's column starts (two
        // spaces); `[x]` is optional, and `<a b>` is one argument.
        let signature = rest.split("  ").next().unwrap();
        let mut words = signature.split_whitespace();
        let command = words.next().unwrap();
        // `boot` runs /init's steps; `reboot-tryboot` restarts this computer.
        if command == "boot" || command == "reboot-tryboot" {
            continue;
        }
        let mut args = Vec::new();
        let mut open = false;
        for word in words {
            if word.starts_with('[') {
                break;
            }
            if !open {
                args.push("/nonexistent");
            }
            open = word.starts_with('<') && !word.ends_with('>') && !word.ends_with(">...")
                || open && !word.ends_with('>');
        }
        let mut argv = vec![command];
        argv.extend(args);
        let out = run(&argv);
        assert_ne!(
            out.status.code(),
            Some(2),
            "`slot-init {}` is documented but answered with usage: {}",
            argv.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        checked += 1;
    }
    assert!(checked >= 14, "only {checked} documented commands found");
}

#[test]
fn merge_layers_makes_a_gaia_bundle_bootable() {
    let slot = format!("{FIXTURES}/layered-slot.json");
    let layers = format!("{FIXTURES}/layers.json");
    let out = run(&["merge-layers", &slot, &layers]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dir = std::env::temp_dir().join(format!("slot-init-merge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let merged = dir.join("slot.json");
    std::fs::write(&merged, &out.stdout).unwrap();

    // What the writer checks next: the layers, lowest first, each with its
    // size, sha256 and verity root hash.
    let out = run(&["layers", merged.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let names: Vec<&str> = text
        .lines()
        .map(|line| line.split(' ').next().unwrap())
        .collect();
    assert_eq!(names, ["base", "platform", "app"], "{text}");
    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        assert_eq!(fields.len(), 5, "{line}");
        assert!(fields[2].parse::<u64>().unwrap() > 0, "{line}");
        assert!(Path::new(&slot).exists());
        assert_eq!(fields[4].len(), 64, "{line}");
    }
}
