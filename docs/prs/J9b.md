# J9b: the sim's lease-ran-out run ends with the bridge's words

Follows J9. J9 left the lease cases untested because the sim had no contested lease. The coordinator's
ruling narrowed this PR to the case the locked bridge really has: a caller silent past its lease, whose
running run ends.

## Cause

`crates/testing/sim-node/src/lease.rs` ended a silent caller's run with `CALLER_WORDS`, a sentence
written for the sim, not the bridge's text. The bridge's words are in Lemnos at the rev Atlas pins
(`bded77a0`, the `shm-*` crates in `Cargo.lock`), in `service/orion/src/calls.rs`.

- The lease sweep (`calls.rs` :314) ends each of the caller's running runs with `ended_with`, using the
  string at :318: `its lease ran out; the job ended with its connection`.
- Those runs finish `Outcome::Failed` with that string as `detail` (`ended_with`, :595-601).
- Nothing else is sent to the caller. It is not a comm-core refusal and it has no code.

## What changed

1. **`CALLER_WORDS` is the bridge's text, verbatim**, with a `LOCK:` comment naming `calls.rs:318`,
   the call at :314, and the rev `bded77a0`.
2. **`actions.rs` test `a_board_that_drops_the_hold_on_a_long_action...`** asserts the exact words
   (it matched a substring of the old sentence).
3. **Client-level test** `a_caller_silent_past_its_lease_has_its_run_end_failed_with_the_boards_words`
   (`crates/board/actions/tests/actions.rs`). A run is started through `BoardActions::start` and
   stalls on the board; the caller asks for nothing; the board's lease clock moves 61 s; the follow
   ends. The test checks:
   - the error is `ActionError::Failed`, not `Refused` and not `Denied`;
   - `board_reason()` and `Failure::text()` are exactly the bridge's words;
   - the `Failure` kind is `Other`, the code is `None` (none is sent), and `is_denied()` is false.

Product code is unchanged: `Outcome::Failed` already maps to `ActionError::Failed(reason)` with the
board's detail (`crates/board/actions/src/board.rs`, `ended`). No word is matched anywhere in product code.

## Where the words become a code

The test compares the exact text. That is the only place it is matched. Once Orion's typed `Finished`
carries a `code` and Lemnos sets one for "lease ran out", Atlas should match the code and the test
should check it too. Until then the words are the only identifier the board gives, and they stay the
board's words.

## Not done, and why

- **Busy (11) and LeaseLost (22) through Orion.** They come from Orion's node logic for resource leases
  (`crates/node/resources`, `contest.rs`), not from the bridge. The sim runs stand-in node logic
  (`for_node: false`), and its provider API cannot send a comm-core refusal, so the sim cannot produce
  them. Coordinator ruling: Orion will bring real node logic in-process later.
- **Busy for a second bridge caller.** No locked source has the bridge refuse a second caller. Dropped
  (coordinator ruling).
- **`STALL_WORDS` (`lease.rs`) is still the sim's own sentence.** The bridge's stall rule is locked
  (Lemnos I6g, 600 s) but no locked source gives its words for a stalled run, so it is not changed here.
- **The bridge's `lease-ended:` note** on a caller's first reply after its lease ended (`calls.rs`,
  `LEASE_ENDED_NOTE`) is not modelled by the sim.
