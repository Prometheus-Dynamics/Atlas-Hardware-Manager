# PINS-2: Orion 3dde304 (C54), the image's Orion component, and the systemd account rule

**Status: DRAFT. Do not merge.** The pair in this tree is mismatched on purpose.

## The pair (final, approved by the coordinator 2026-11-16)

| Pin | Rev | Where |
|---|---|---|
| Lemnos (`lemnos-slot-store`; every Lemnos component; the image's Lemnos packaging) | `f14b833eb2adf911225aa54199f25f93fb4cb89c` (M39-PIN, PR #396) | `Cargo.toml`, `devices/components/lemnos*`, `raze-lemnos-board`, `devices/products/raze-base/gaia.lock` |
| Orion (workspace, `atlas-nt`, `atlas-topics`, `atlas-sim-node`, the image's `orion` and `orion-recovery`) | `3dde304e396f5769d3aa25884b5a9ff5bd4b9180` (C54) | as before |

The pair is Lemnos f14b833e with its bridge's Orion pin (3dde304e, the same Orion this PR carries). PINS-2 stays a DRAFT:
it waits for Orion's stub to issue run handles, and the three handle sim tests and step 7c must pass before it leaves draft.

`origin/next` (d1adf855) is merged into this branch; it moved `devices/gaia.lock` to Gaia `9efa797f`, and the lock
regeneration uses that Gaia binary (`/mnt/ssd-samsung/gaia/target/gaia-pr-patchappend/release/gaia`, which reports 9efa797f).

## Lemnos f14b833e: consumer-visible changes and where Atlas reads them

| Lemnos change | Atlas consumer (file:line) | What changed in Atlas | Test |
|---|---|---|---|
| A deadline-ended run is `TimedOut` with code 110 (was 0) | `crates/board/actions/src/board.rs:592` (`Outcome::TimedOut` becomes `Failed` with the run's code) | None needed: the code is read from the run's end, so 110 flows through. Nothing in Atlas keys on code 0 | `board.rs` test `a_failed_or_cancelled_run_keeps_its_code_when_it_has_one` (its TimedOut case asserts code 110) |
| The bridge reads `detached` from the request; a cancel matches on the node's run number | bridge-internal. Atlas sends `detached: true` for starts (`board.rs` `start_encoded`) and `false` for direct runs (`board.rs` `direct`) | None: the bridge now honours both | the action and job tests (`crates/board/actions/tests/actions.rs`) on the gate |
| BB1c: `info` gains `trial_updates`, `trial_block`, `trial_block_text`, `trial_block_detail` | `crates/tools/next/src/cycle.rs`: `trial_state` (one `info` read, only where `info_in_pin`), `trial_state_of`, `TrialBlock::from_wire`, `BoardTrialState` | The wire value is read once into the typed `TrialBlock` (five known reasons). An unknown value, or a non-string value, is kept as sent and shown as unknown; it is never matched as a string elsewhere. The state is printed in step 6 before the trial | `trial_block_tests` in `cycle.rs` (five unit tests: every reason maps; absent updates say nothing; null is clear; a string is typed; a non-string block is unknown and never clear; the words show the typed block and the unknown value) |
| BB1c: a `uboot` board without `lemnos_env_rev=1` refuses trials with NOT_PERMITTED and reports Degraded; `uboot_env_size` below 394 is refused | none in `devices/`: no board or product uses `record = "uboot"` yet (the Raze uses `tryboot`), and no board sets `uboot_env_size` | no device rule added: there is no subject to check. The image-side facts (the marker in `u-boot.bin`, the `uboot-env` partition, no FAT `uboot.env`) are for Gaia's image build and the first `uboot` board's description | none |
| R1: status `TORN` (-4101) and `Error::StreamTorn` | none: Atlas links no `lemnos-client` and reads no stream frames. The only Lemnos crate Atlas links is `lemnos-slot-store` (the bundle format), which R1 does not touch | None | none |

### Trial state for users (BB1c, the CLI; the app is not in this repository)

- **Shared reading**: `crates/drivers/board-slots/src/trial_updates.rs`: `TrialBlock` (the five known reasons, and
  `Unknown(as sent)`), `TrialUpdates` (`NotReported` / `Clear` / `Blocked`), `of_info`, `words`, `read`. One copy: the
  cycle tool (`crates/tools/next/src/cycle.rs`) uses it too; its local copy is gone.
- **The CLI, where a user looks**:
  - `list` (`commands.rs` `list`): a line "this board cannot take trial updates: <reason>" when the board states a block;
    nothing is shown for a clear board.
  - `trial` (`commands.rs` `trial`): **refused up front** when the board states a block (`trial_refusal`), before
    `Follower::open` and before any write, with the typed reason. An unknown value is shown as "unknown (as sent: …)".
  - `install` (`commands.rs` `install`): **allowed, and it promises no boot.** It writes the bay only. On a blocked board
    the block line and "the slot is installed, but the boot loader will not start it as a trial until the board takes
    trial updates" print **before** the transfer and **again** after the slot is installed. The exit status stays 0:
    `install` alone never promises a boot. The help text says "install writes a slot and never starts it: `trial <slot id>`
    starts it".
  - `cycle` (`cycle.rs` `step_install_good`): **the one combined command** (install and start a trial). A blocked board
    refuses it **up front**, before the first transfer, with the typed reason (`trial_refusal`).
  - Today (before this change) an install onto a blocked board was attempted and succeeded, and a trial failed with the
    board's own NOT_PERMITTED. That is (b). It is now (a) for the trial, and the install's note is up front.
- **Ruling (2026-11-16)**: `install` stays allowed with the warnings above. Any command or flow that means "install AND
  start it" (now or later, including the app's update action) **refuses up front, before any transfer**, with the typed
  reason. The one source is `trial_updates::read` (the reading) and `TrialUpdates::words()` (the line); no consumer
  matches the wire string or words of its own.
- **The board's own refusal** still handled: the trial's refusal from the board is reported as it always was.
- **The app**: `apps/` is not in this repository, so the app's board view is not changed here. It should call the same
  `trial_updates::read` and show `TrialUpdates::words()` on one line.
- **Tests**: `board-slots/tests/slots.rs` (`trial_updates_read_each_known_block_as_its_typed_reason`,
  `trial_updates_keep_an_unknown_block_as_sent_and_a_clear_or_absent_board_says_so`) on the sim board (`slot_set_trial_block`,
  `slot_set_trial_updates_key`); unit tests in `trial_updates.rs` and in `commands.rs` (`trial_refusal_tests`).
- **Image-side facts** (`lemnos_env_rev=1` in `u-boot.bin`, a `uboot-env` partition, no FAT `uboot.env`, `uboot_env_size` ≥ 394):
  their subjects are the bench branches that use `record = "uboot"` (Z2, R1, C1). Each gains that device rule when it takes
  this pin. Not added here.

BRIDGE-BOUNCE and the missing refusal key are open on this pair (the second session after a gadget bounce can time out; there
is no refusal key yet). Both come with the next pair. No image is built from this PR.

## Earlier provisional pins (superseded by the pair above)

| Pin | Rev on this branch | Final |
|---|---|---|
| Orion (workspace `orion`, `atlas-nt`, `atlas-topics`, `atlas-sim-node`, image `orion` and `orion-recovery`) | `3dde304e396f5769d3aa25884b5a9ff5bd4b9180` (C54) | provisional |
| Lemnos (`lemnos-slot-store`, every Lemnos component) | `31e0f736a334260a7df2ea86bcfa358e9a640f08` (unchanged) | provisional |

The final pair is the Lemnos rev whose bridge pins the Orion rev that carries the pending node fix (the
fix for a link bounce that drops a new hello, INPUTS 2026-11-16). It will be announced; this PR is then
re-pinned to that pair before it can merge. Until then the tree carries Lemnos 31e0f736, whose bridge pins
Orion 671d29d, next to Orion 3dde304. The coordinator approved building the content on 3dde304 now.

Orion's own Lemnos dependency (its `shm-*` crates) moves with Orion's manifest from bded77a0 to b0a75d93.
Atlas's Lemnos pin does not move, so the lock names one Lemnos rev per package (the one-copy rule holds).

## Changes, in order

1. **Compile on Orion 3dde304** (`e4258407`). Every Orion pin moves from 671d29d; `cargo update -p` for the
   41 Orion packages changes `Cargo.lock` only. `cargo check --workspace --all-targets --locked` passes with
   no code change. The Orion-using crates' tests pass: atlas-actions, atlas-nt, atlas-topics, atlas-sim-node,
   atlas-board-slots.

2. **Run handles on T6b's guards** (`ddb6d414` merges `origin/pr/T6b`, since #141 is not on next yet; the handle
   commits sit on top of it). Each stored run (`Issued`, the one a job or an install keeps) carries the run's handle
   in memory beside its number, shared by every clone. Nothing is saved (no job-file or core/jobs change).
   - The handle is learned at the start (the node's started frame, waited for briefly: `START_HANDLE_WAIT`), and again
     at every follow or cancel that reads it.
   - Follow and cancel name the run by its handle when it is known. A handle names the run whatever the session.
   - With no handle, a number is never used across a session change (`may_use_number`): the follow or cancel is
     refused as `Unconfirmed` (the outcome is unknown), and nothing is sent. Same-session use of a bare number is
     unchanged.
   - Orion's `RunGone` (a handle from before a node-logic restart) becomes the typed unknown outcome
     `Unconfirmed` with the words "the board restarted while this ran; outcome unknown", which is T6b's Unknown state.
   - Orion's `NotFound` becomes "no such run", kept as a refusal (`Failure` kind Refused).
   - A cancel by handle is sent only after the attach is not refused (`CANCEL_CHECK`, half a second for the node's
     answer to the attach).
   - Unit tests (run now): the rule (`may_use_number`), the shared handle across clones, `RunGone` and `NotFound`
     mapping for follow and cancel.
   - **Sim tests, ignored, waiting on Orion's stub** (`crates/board/actions/tests/actions.rs`), each with
     `#[ignore = "needs Orion's stub to issue run handles (asked 2026-11-16)"]`. They must be un-ignored before this
     PR leaves draft:
     - `the_exact_collision_a_stored_handle_never_returns_the_newer_runs_answer`
     - `a_handle_from_before_a_node_logic_restart_is_refused_as_restarted`
     - `a_stale_handle_cancel_is_refused_and_the_newer_run_keeps_running`
   - The characterization test `until_orion_run_handles_a_same_session_follow_can_return_an_older_runs_answer` is
     unchanged: it asserts today's known-bad answer on the stub, which issues no handles, and it flips only when the
     stub does.
   - **T6b install tests changed on this pin (no handle on the stub).** A follow after a session change is no longer
     repeated by number, so the install is `Unknown` while its list is checked, and the list decides:
     - `a_follow_cut_by_a_lost_connection_is_repeated_and_the_first_answer_is_the_only_answer` is renamed
       `a_follow_cut_by_a_lost_connection_is_not_repeated_by_number_and_the_list_decides`: the install is shown
       `Unknown` (not `WaitingForBoard`), and once the board finishes, the list shows the slot.
     - `an_install_interrupted_by_a_replaced_board_ends_and_sends_nothing_to_the_other` is renamed
       `..._is_unknown_or_failed_and_sends_nothing_to_the_other`. The install is `Unknown` while the list is checked,
       or `Failed` on the replacement (`REPLACED`) once the list read sees it, which depends on timing; either is
       accepted. It is never done, and it sends nothing to the other board.
     - The cycle test's refusal-after-restart (step 7c, exit 1) may change; see the gate result.
     - With a handle on a later pin the first of these returns to a follow by handle.
   - **Known limit (not built here):** nothing is persisted. After an Atlas restart a running board job cannot be
     re-followed, because neither the number nor the handle is saved. Persisting the handle would make that possible;
     it is a possible later item.

3. **The image's Orion component** (`57f69fd4`), `devices/components/orion` and `orion-recovery`:
   - Source ref is `3dde304e`. `scripts/check-devices.sh --write-locks` (GAIA_BIN at Gaia a6299c62, which
     reports `gaia 2.0.6-dev (a6299c62…)`) regenerated `devices/products/raze-base/gaia.lock`; it moved only
     the two Orion refs. The other product locks are unchanged.
   - Accounts, declared in this component only: `orion` (901, group orion, groups dialout) and
     `orion-comm` (902, group orion, groups dialout), with Gaia's `[[users]]` fields; the `orion` group (901)
     is declared by `[[groups]]`.
   - Units, as Orion installs them at this rev: `orion-comm.service` (User=orion-comm, RuntimeDirectory=orion),
     `orion-node.service`, `orion-login.socket` and `orion-login.service`. Nothing ships `/run/orion` or
     `/var/lib/orion`: the units make `/run/orion` at runtime, and `/var/lib/orion` is not used (see the
     state override below).
   - **The state override stays, with its reason.** `state.dir`, `keys.dir` and `ssh.host_key` point at
     `/data/orion` (orion.conf). `/var` is inside the slot root on a managed board, so a key there changes at
     a reflash or a slot change; `/data` is the same for every slot. It still works with `StateDirectory=`:
     the unit's `StateDirectory=orion` would create `/var/lib/orion` for nothing, so the drop-in clears it
     (`StateDirectory=`), and the drop-in makes `/data/orion` itself. That directory was root's, which the
     unit running as orion-comm could not write, so it is now owned by orion-comm with group orion, so
     node logic can read the keys. The node's own state is `/data/orion/node` the same way.
     `RequiresMountsFor=/data` is kept (Orion's unit has no mount ordering for it).
   - `orion-login.socket` is enabled. It starts `orion-login.service` on the first question, and it names
     `orion-comm` as its SocketUser, so the account must exist (see change 4).
   - Settings: `socket.node_uids = 901` kept; `login.comm_user = orion-comm` written out (Orion's default,
     the only user the login helper answers).
   - **Bridge address.** `lemnos-orion` gets a drop-in, `ORION_ADDRESS=unix:/run/orion/orion.sock`, the
     node's local socket. The bridge refuses a network or signed-serial address at start (Lemnos
     `service/orion/src/config.rs`), and an unset address already means this socket; the drop-in writes it out.

4. **The product check for accounts** (`4fd44bb2`), in `scripts/check-devices.sh`. For each product, every
   unit a component installs (drop-ins included) is read, and each `User=`, `Group=`, `SocketUser=` and
   `SocketGroup=` that is not `root` and not numeric must be declared by a component of the product (a
   `[[users]]` name, or a `[[groups]]` name, a user's group counting). A refusal names the unit and the
   account. The units come from `file =` beside the component, or from `source =` at the commit the lock pins,
   read from a bare clone under `ATLAS_UNIT_CACHE` (default `~/.cache/atlas/units`), which a first run clones.
   Self-test: the Raze product passes as the tree has it, and on a scratch copy with the orion-comm block
   removed it is refused, naming `orion-comm.service` and `orion-comm` (and `orion-login.socket`'s SocketUser).
   The rule's refusal today: `FAIL products/raze-base: orion-comm.service names User=orion-comm, which no
   component of the product declares as an account …`.

5. **This document.**

## What the next pin move adds (not in this PR)

- **T8, session liveness on `Reply::Alive`** (INPUTS 2026-11-16, C53): `Options.reply = Reply::Alive{ceiling}`
  with `set_reply` and `silent_for()`, so a healthy but slow reply is not `Gone("timed out")`. Lands on the
  pin after this one, with the sim's own idle window.
- **The return of the detached slot list (T6)**, on the fixed Orion rev, in one PR with a deterministic test
  (T6 was reverted by #135 after the kept-answer collision, INPUTS 2026-11-16).
- **C55 (typed too-long socket path)** is in Orion after C54 (`ee05527e`); Atlas's SUN-ROOT side uses it once
  the Lemnos bridge carries it. Not in this pin.

## Open

- Orion's stub at 3dde304 issues no handles, so the three ignored sim tests cannot run. Orion has been asked for a
  stub that does (2026-11-16). They must be un-ignored, and the characterization test then flips, before this PR
  leaves draft.
- The pair (Lemnos rev + Orion rev) is announced later; this PR is re-pinned then and goes to ready.
