# T6b: the cycle tests, and the guards for a run number reused after a session change

## Cause

Two things share one name space: the node keeps finished answers by run number, and Atlas's client numbers
its runs from 1 on each connection. A number used on one session can name an older run's answer on a later
session. Atlas then read that old answer as its own result.

Proving lines, `t2run1.log` (traced cycle binary, load 146-169):

- session 1: `list call=4` ended with the list; `list call=6` never ended (the connection dropped under it).
- session 2 started at `list call=1`; the install's start was `call=4` and its follow returned the **list JSON**
  at once, not the slot id.
- the list check then refused: the "slot id" was a whole listing.

The sim reproduces the same collision without a knob (probe, now `until_orion_run_handles_a_same_session_follow_can_return_an_older_runs_answer`):
session 1 runs A (number 2), session 2 runs B (number 2); following B returns A's answer, `passed`, at once.

The cause is in Orion's stub router (`testing/stubs/src/router.rs`, attach at lines 197-205): an attach matches
the first detached run whose caller number equals the attach number, across sessions. The client-side session
check in Atlas does not reach a run started on the current session.

## Why T6 was reverted

T6 (#132) made `list()` a detached read. A detached read that ends unattended stays on the board with its number,
so the list became the first source of stored answers to collide. Before T6, `list()` was a direct run and left
nothing stored. The revert (#135) removed the trigger. It does not remove the hazard, which is in Orion.

## What this PR does

Guards live where a followed answer is used. None of them can see a same-session misroute of a generic job
(see "Not covered" below).

| Site | What it does | Covered |
|---|---|---|
| `board/actions/src/board.rs` `follow_inner` | Kind check (`Contract`) before any answer is used; `Followed::{Confirmed, Reconnected}` by session; the run's own refusal kept typed in `Unconfirmed { ended }` | cross-session and wrong-kind |
| `board/actions/src/board.rs` `cancel` | refuses a cancel by number from an earlier session (sends nothing) | cross-session (site 2) |
| `board/actions/src/jobs.rs` cancel confirm | `Confirmed` only from the issuing session (site 6) | cross-session |
| `board/actions/src/jobs.rs` job step | a reconnected end ends the step `Unknown`, never `Done` (site 3) | cross-session |
| `board/actions/src/jobs.rs` cancel confirm, job | a cancel read on a later session is not a confirmed cancel (site 4) | cross-session |
| `drivers/board-slots/src/install.rs` | answer and list agree on the same session; across a session change the list decides | installs, all sessions |
| `core/jobs` | `JobStatus::Unknown { reason, checking }`: non-terminal while `checking` is set; a step reports it through `Jobs::report_unknown` | the hook |
| `tools/next/src/commands.rs` | an install still Unknown after the 15 s bound exits 4, with what was checked | the tool |

### Install rule (the driver)

- A follow cut by a lost connection is repeated. It produced no answer; the install waits, the job retries with
  its backoff, and follows the same run once the session is back.
- The first answer is the only answer. After it, the run is never followed again.
- An answer counts only with the list's agreement:
  - a slot id on the same session: the list must show that slot, new and matching the bundle's product, board,
    version and variant. Otherwise the step fails: "the board answered slot-2; its list does not show slot-2 after the install".
  - a refusal with the board's words: accepted when the list shows no new matching slot ("the list confirmed nothing
    was installed"). If the list shows one, the step fails as a disagreement, with both facts.
  - a later session's answer, or none: the list alone decides. One new matching slot is Installed; none is Unknown,
    shown with what was checked, and the tool exits 4 at its bound.
- The snapshot of slots before the start is one extra list read. The hidden-slot test counts it: three list reads.

### Tests

- Sim: `a_follow_cut_by_a_lost_connection_is_repeated_and_the_first_answer_is_the_only_answer` (the connection dies
  under the follow, the follow is repeated, the run is not started twice).
- Sim: `a_follow_answered_at_once_is_checked_against_the_list`.
- Sim: `a_lapsed_hold_is_no_answer_and_the_run_is_followed_again_when_it_answers`.
- Decision table (unit): `one_new_slot_of_the_bundles_identity_is_the_install`, `a_slot_that_was_there_before_is_not_the_install`,
  `no_new_slot_yet_is_not_decided`, `two_new_matching_slots_are_ambiguous_and_none_is_picked`, and the kind check.
- Characterization, not ignored: `until_orion_run_handles_a_same_session_follow_can_return_an_older_runs_answer`.
  It asserts today's known-bad answer. It flips when the Orion pin moves to a13d4030 (C53b).
- Jobs: `a_step_that_is_checking_shows_unknown_with_what_it_checks_and_then_resolves`.
- Cycle: all six tests, including step 7c (the bad trial restart drops the session during the install, then the
  repeated follow returns the board's refusal, and the list confirms nothing was installed: exit 1).

### Bounds

- The tool's bound for an Unknown install is its existing `env.wait` (15 s), applied only to the cross-session
  Unknown. No new wait is added.
- A follow cut by a lost connection is repeated under the job's retry backoff, bounded by the caller's existing
  step bound, as before.
- The 24 h step timeout and the tool's install loop are deliberately unchanged in this PR.
- The stalled-install bound is Lemnos M40 (a per-job stall bound, ending TimedOut 110). Atlas takes it when it lands.

## Not covered until the Orion pin

1. **Same-session follow or cancel by bare number, for generic jobs** (sites 1-4 on the same session). The answer
   carries no run identity and the session is the one the run was started on, so no Atlas guard can see a
   misroute. The characterization test documents it. The fix is Orion a13d4030 (C53b): every start takes its
   number from the user's earlier runs, a bare attach is refused when ambiguous, and run handles are added. It
   arrives with the second pin move.
2. **A misrouted older refusal while the real install is still running.** On the same session, an older run's
   refusal can be accepted as the install's outcome, and the slot appears later. The list agreement check runs
   only on accept: if a later list read after an accepted refusal shows the slot, that is the typed disagreement.
   That read is the one the code already makes; no polling is added, so the case is caught only if a read happens
   after the slot appears. It is not otherwise detected.
3. **Same-session cancel by number for installs**: the cancel reaches a run by number. Only the session check
   covers it, not a same-session misroute (item 1).

## Not in this PR

- The sim does not model Orion's run identity. The collision tests above use the sim as Orion's stub behaves.
- No new install budget. The Unknown window is the tool's existing 15 s bound.

## Proof

- Cycle binary, whole, 10 runs under the 30-process CPU load (cores 6-31), each run six of six green: **10 of 10
  runs passed**. Logs: `proof-1..10.log` in the T6b scratch.
- Gate (`scripts/gate.sh`, `ATLAS_SCRATCH_ROOT` set to the T6b scratch): rc=0, every step.
- Actions, jobs, session and board-slots suites pass (board-slots: 19 slots tests, including the lost-connection
  and answered-at-once cases).
