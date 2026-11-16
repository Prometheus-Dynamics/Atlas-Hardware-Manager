# PINS: Lemnos 31e0f736 and Orion 671d29d, with the code that goes with them

## Pins (resolved and verified)

| Pin | Full sha | Source |
|---|---|---|
| Lemnos (next) | `31e0f736a334260a7df2ea86bcfa358e9a640f08` | `git ls-remote` on Lemnos: `refs/heads/next` |
| Orion | `671d29d626f49e06ab7332417275ef1b0ac31b01` | `gh api repos/Prometheus-Dynamics/Orion/commits/671d29d` resolves it; Lemnos `service/orion/Cargo.toml` at 31e0f736 pins every Orion crate to this rev, and `testing/crates/orion-link-check/Cargo.toml` does too |

The pair is 31e0f736 with Orion 671d29d, never mixed.

## What moves

- `Cargo.toml`: `lemnos-slot-store` to 31e0f736, `orion` to 671d29d.
- `crates/board/nt`, `crates/board/topics`, `crates/testing/sim-node`: Orion to 671d29d.
- `Cargo.lock`: `cargo update -p` for the Lemnos and Orion git crates only (41 Orion entries and 1 Lemnos entry now at the new revs; none left at the old).
- Product-local Lemnos components on next: `lemnosd`, `lemnos`, `lemnos-ctl`, `lemnos-slot-init`, `lemnos-orion`, `raze-lemnos-board` (all were f4a295c6, now 31e0f736).
- `devices/products/raze-base/gaia.lock`: regenerated with `scripts/check-devices.sh --write-locks` and `GAIA_BIN` at Gaia a6299c62. Its Lemnos sources now name 31e0f736. The other product locks regenerated to identical content; only raze-base changed.
- `crates/core/bundle/tests/fixtures/lemnos-f4a295c6`: unchanged. It is a recorded fixture of the bundle format made at f4a295c6; it still passes.

## Not moved (needs your word)

- `devices/components/orion/component.toml` and `orion-recovery`: still pin Orion `455a956e13e62c97081d66e0e8f3bf2aced1eac8`, and the comment there is stale. Recommendation: move them to 671d29d with this PR, so the board's comm core and the Lemnos bridge run one Orion. Not moved: waiting for your word.

## Gaia: moved to a6299c62

Gaia next is now `a6299c62a036b83172fdeb1c7b43aa3eaa719d30` (c0d084fa plus one file, `crates/nodes/os-buildroot/src/contract.rs`: the reader accepts Lemnos's `[crash_log]` table). Moved in `devices/gaia.lock`, in the `usb-gadget-network` component's ref, and in the `raze-base`, `cubie-a7-bringup` and `zero-3e-bringup` locks (regenerated with `scripts/check-devices.sh --write-locks`, `GAIA_BIN` = `/mnt/ssd-samsung/gaia/target/gaia-pr-crashlog/release/gaia`, which prints `gaia 2.0.6-dev (a6299c62...)`). The recipes came from the script's own clone of the locked commit. The device check passes (3 families, 7 boards, 17 components, 8 products), and so does its self-check.

Before the Gaia move, the check passed with c0d084fa against Lemnos 31e0f736 too, so no product on next was affected by `[crash_log]` at that rev; the move is the one the reader needs for the table.

## Code changes that go with this rev

1. **Busy typed from the code (T4-7c).** `crates/board/actions/src/board.rs` `ended()` keeps the run's status code on Rejected, Failed and TimedOut endings (zero is none). `ActionError::is_busy()` and `BUSY_CODE = 16` in `error.rs`. `crates/tools/next`: `Refusal.busy` is set from the typed code (the direct path from the error, the install path from the run's `failure`). `cycle.rs` now matches `refusal.busy`, not the words "is on trial with". The words are still printed.
2. **Sim follows the M39 outcomes.** `crates/testing/sim-node/src/node.rs`: `finish_coded` sends `finish_with_code`; `ending_for` maps statuses as Lemnos's `ending_of_status` does: Cancelled 125, the refused list Rejected with its code (1, 2, 11, 16, 19, 22, 95, 4098), others Failed with code zero. A missing pushed file is Rejected 2. `slots.rs` gains `AGAIN` and `BUFFER_TOO_SMALL`.
3. **Lease rule.** `lease.rs`: `advance` moves the clock and the stall drops; new `expire(busy)` drops only a silent caller with no run in flight, and renews a busy caller. The node passes the callers with runs (and the install's caller) in flight. `CALLER_WORDS` is removed. A dropped caller's next reply begins `lease-ended: holds released` (`LEASE_ENDED_NOTE`, once, `lease::with_note`). Tests: `the_run_survives_the_callers_silence` (replacing the "failed with the boards' words" test) and the lapse test that now asserts the run survives; new `a_caller_whose_lease_lapsed_with_nothing_in_flight_is_told_its_holds_were_released`.
4. **Atlas reads the note.** `board.rs` `lease_ended()` / `LEASE_ENDED_PREFIX = "lease-ended:"`: a follow whose reply carries the prefix returns `ActionError::HoldLapsed`. TEXT MATCH, marked for replacement by the typed field in M40.
5. **Lemnos API break (M37+).** `crates/core/bundle/src/manifest.rs`: Lemnos's slot manifest gains `boot.config` (`boot_config`). `BootEntry` now carries `config: Option<String>` (serde default, skipped when absent), read from and written to Lemnos's manifest. This was the only compile break; `DeviceEntry`/`Event` `refusal` and `Backoff.kind` needed no Atlas change.
6. **Bridge address (item 4).** Checked every image and unit file under `devices/`. No image or unit sets `ORION_ADDRESS`. Lemnos's example env file has it commented out (`#ORION_ADDRESS=`), and the component installs that example. The bridge therefore publishes on the board's local socket. Nothing to fix.

## Deliberately left for the second pin move (Orion C53)

- Session liveness, the session monitor's reply timeout (T8), run handles and the detached-run kept answers (T6/T6b). Not touched.
- The `lease-ended:` text match becomes the typed reply field (Lemnos M40).
- Orion's `devices/components/orion` pin (see above).

## Gate

`scripts/gate.sh` on `pr/PINS` (see the PR for the last lines). Earlier runs: one failed on a socket path limit caused by the scratch root (not the change), fixed by a shorter scratch root; the rerun passed. The known unrelated red (`atlas-files` tests/push.rs timeouts under load) was not seen in these runs.
