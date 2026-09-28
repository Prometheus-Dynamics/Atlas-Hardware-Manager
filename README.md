# Atlas Hardware Manager (rebuild)

Atlas is the one tool for Prometheus Dynamics hardware: connect to a robot
once, see every PD device on it, and update, configure, or recover any of
them, on Linux, Windows, and macOS.

This branch is a ground-up rebuild. The shipping app lives on `dev` and
`main` until the v1 gate below passes.

## Status: phase 0 (foundations)

| Phase | Scope | Exit gate |
| --- | --- | --- |
| **0 Foundations** (this branch) | Workspace, driver contract, core, simulated devices, CLI, CI on 3 OSes | `atlas ls` and a simulated update job run in CI on Linux, Windows, macOS |
| 1 HeliOS + UI | HeliOS driver (new and legacy API), Tauri app, inventory and device panel | A HeliOS device on `dev` and overhaul images shows up and updates from the UI |
| 2 Bulk + robots | Robot profiles, bulk jobs, resume after restart | 10 devices on the test rig update from one profile |
| 3 Recover + sign | Rust rpiboot, privileged write helper, signed release manifests | USB recovery works on all 3 OSes; tampered artifacts are refused |
| 4 STM32 | Custom USB bootloader driver, gateway relay | An STM32 board updates directly and through a HeliOS gateway |

## Workspace layout

- `crates/atlas-driver`: the driver contract. Identity, links, capabilities, and the driver registry.
- `crates/atlas-core`: product logic. Inventory, scanning, the job engine, events, persistence.
- `crates/atlas-driver-mock`: simulated devices with gateways, recovery mode, and failure injection.
- `crates/atlas-cli`: the `atlas` command, a thin shell over `atlas-core`.
- `docs/`: design notes, including the UI [design language](docs/design-language.md).

Design rules:

- A device is keyed by `family:serial`, never by IP address or USB port.
- The UI and CLI only send intents and render the event stream; all logic is in `atlas-core`.
- A device family is added by writing a driver crate. Nothing else changes.
- No shelling out on the hot path. USB, serial, and block-device access live in Rust.

## Try it

No real drivers ship yet, so use the simulated robot:

```bash
cargo run -p atlas-cli -- --sim demo ls
cargo run -p atlas-cli -- --sim demo update --all --release sim-helios=2026.3.1 --release sim-mcu=1.5.0
cargo run -p atlas-cli -- --sim flaky update cam-rear cam-left --version 2026.3.1
cargo run -p atlas-cli -- --sim demo action cam-front locate
```

`--json` prints machine-readable output; `update --json` prints one event per line.
`--dry-run` shows the update plan without running it.

## Development

```bash
./scripts/ci.sh
```

That runs `cargo fmt --check`, the 600-line file-size check, clippy with
`-D warnings`, and the tests. CI runs the same checks, plus a CLI smoke
test, on Linux, Windows, and macOS.
