# L2c: the slot bundle as Lemnos L2 accepts it

Held for the L2 lock: not to be merged before the Lemnos lock moves (`d6b9361d` today).

## What

The slot bundle's signature and its checks move to the final form in INPUTS (`Slot bundle signature, FINAL`, `Slot bundle signature at L2`, `Lemnos slot stack, old -> new`, `boot_sources, FINAL FORM`, `Lemnos S2`):

- `slot.sig` is the tar's second member and exactly 108 bytes: `LMNSSIG1` | alg 1 | 3 zero bytes | key id (SHA-256 of the public key) | the 64-byte signature. It is `atlas_image::Record`, the first-flash record code, so the two are byte-identical by construction. Nothing writes the record a second way.
- The signed message is built by ONE function per crate: `slot_message` in `crates/core/bundle/src/sign.rs` (writer and reader) and `slot_message` in `crates/testing/sim-node/src/bundle.rs` (the simulated board, a second implementation on purpose). The message is `lemnos-slot-bundle\0` | u32 LE version 1 | u32 LE length | the exact `slot.json` bytes. A signature over the bare bytes does not verify.
- Member order and set, enforced by the inspector (`inspect_bundle`) and by the simulated board's `verify`: `slot.json` first; `slot.sig` second when present (an unsigned bundle is legal); then `rootfs.erofs` and `boot/...` in any order, each at most once and each listed in `slot.json`. An unlisted name is refused at its header, before its bytes are read, naming the member. An out-of-order member is refused naming the rule. A listed member that is missing is refused by name. Every member is checked against its manifest digest.
- The cmdline rule: `boot/cmdline.txt` is the complete line and must carry `<boot_args>.src.<source>=<kernel path>` for each `boot_sources` entry. The inspector takes a `BootRule` (new `crates/core/bundle/src/boot.rs`); the simulated board has the same rule as the field `boot` on `Slots`, defaulting to one source `rpi-boot-mode-1=/soc@107c000000/mmc@fff000` with `boot_args` `os`.
- Policy in the simulated board, identical to first flash: security off, an unsigned bundle is `not_checked_unsigned`, a bundle signed by a key the board holds that verifies is `verified`, a bundle signed by a key the board does not hold is `not_checked_unknown_key`, and a record whose key the board holds but whose signature does not verify is REFUSED. Security on takes only a held-and-verifying bundle. A malformed record (length, magic, algorithm, padding) is always `INVALID_ARGUMENT`.
- The boot-file rules in the simulated board (`crates/testing/sim-node/src/slots/boot.rs`), as Lemnos's code does them (next e3e8a963: slot-init `boot.rs`, lemnosd `boot_guard.rs`, slot-store `bootcheck.rs`; INPUTS "Boot-file mismatch, AS LEMNOS'S CODE DOES IT"). Every boot checks every file the manifest lists, before the image is opened:
  - a TRIAL that does not match fails by the normal failure path (failed, failure count, fail reason, fall back);
  - a DEFAULT that does not match, with a previous that matches: the previous becomes the default at once, with no count; the old default is marked failed with `its boot files don't match its manifest: <why>`; the log says `slot <slot>'s boot files don't match its manifest: <why>; <previous> is the default now, restarting into it`;
  - neither the default nor its previous matches: `no slot can start: <why>; its previous <p> doesn't match its manifest either: <why2>`; no pointer moves; the board restarts again after the rescue wait (30 s, on the simulated clock);
  - a default with no valid previous: fails by the normal failure path (approximation: no text in INPUTS for this case; the sim reports `no slot can start: <why>`).
  - Before a restart, and every 600 s of the simulated clock, the default is checked: it repoints when its previous matches (the restart proceeds, with the note `<default> doesn't match its manifest (<why>): <previous> is the default now`), and is refused, board up, when neither matches (`<default> doesn't match its manifest (<why>), and its previous "<previous>" doesn't either: the restart is refused`). The periodic check only reports. Interpretation: the periodic check also repoints when a previous matches, as the same check does before a restart.
  - `<why>` is the per-file list joined with `"; "`, each `<name> is missing from the boot partition` or `<name> doesn't match its manifest hash` (bootcheck's texts), in the manifest's order (sorted by path here; Lemnos uses the manifest list order).
  - The reboot action's reply is the note when the default was repointed; a refusal is `-1: <text>` and Atlas receives the board's text unchanged.
- `atlas-board-slots` names the four outcomes in Atlas's own words (`crates/drivers/board-slots/src/signature.rs`, `SignatureWords`): `verified`, `not_checked_unsigned`, `not_checked_unknown_key`, `refused`. A refusal carries the board's reason unchanged. The shared outcome type is `atlas_image::BoardOutcome`, read through `atlas_slots`; nothing in core changed.
- The `slot.key` member is gone: the key id lives in the record. A bundle that still carries `slot.key` is refused by name.

## Why

Lemnos I5g2 and I5i4 (INPUTS). Lemnos refuses the bare 64-byte `slot.sig` form, and the old installer's command line. The simulated board and the bundle inspector have to refuse the same things Lemnos refuses, so that a bundle Atlas writes is one the board accepts.

## Vector proof

The fixed vector from INPUTS: Ed25519 seed `0x07` x32, manifest `{"id":"vector"}`.

- message (19-byte tag, 4 + 4 length and version, then the 15 bytes of JSON): `6c656d6e6f732d736c6f742d62756e646c6500010000000f0000007b226964223a22766563746f72227d`
- record: `4c4d4e535349473101000000` | key id `fe812c12f3ab4ce6ac5db69ac352f906cb1b11ef43fb33e252ef7ff552263889` | signature `56be579db020703eb207da579036a89f0321fcd8ce2bac78a9aa939a58217e265ad27a7e1fc05829b0db05d37ac4d8dba35263c7a1b2644322b45ada4a101402`

Checked by two tests, each producing these bytes from the seed and the manifest, with no adjustment:

- `crates/core/bundle/tests/bundle.rs::the_fixed_vector_is_reproduced_byte_for_byte` (atlas-bundle: the message, the 108-byte record, and `verify_slot` accepting it under the seed's public key). It compares byte by byte and names the first difference if one occurs.
- `crates/testing/sim-node/src/bundle.rs` test `this_board_reproduces_the_fixed_vector` (the simulated board's own message and record).

## Refusals and where they are tested

Each by name. `bundle` is `crates/core/bundle/tests/bundle.rs`; `sim` is `crates/testing/sim-node/tests/bundle_rules.rs` unless stated.

Order and set (INPUTS item 4):
- slot.json not first: `bundle::slot_sig_must_be_second_and_slot_json_first`, `bundle::a_payload_member_before_slot_json_is_refused_by_the_order_rule`, `sim::the_members_are_checked_in_order_and_the_order_is_named`.
- slot.sig out of order: `bundle::slot_sig_must_be_second_and_slot_json_first`, `sim::the_members_are_checked_in_order_and_the_order_is_named`.
- unlisted member (`slot.key`, `boot/extra.txt`): `bundle::a_member_outside_the_set_is_refused_by_name`, `bundle::a_bundle_with_an_extra_member_is_refused`, `sim::a_member_outside_the_set_is_refused_at_its_header_by_name`, `sim::a_boot_file_slot_json_does_not_list_is_refused_by_name`, sim `trial.rs::the_old_slot_key_member_is_refused_by_name_and_leaves_nothing`.
- listed member missing: `bundle::a_listed_member_that_is_missing_is_refused_by_name`, `sim::a_listed_member_that_is_missing_is_refused_by_name`.
- digest of a member: `bundle::a_member_that_does_not_match_its_manifest_digest_is_refused`.
- unsigned bundle legal: `bundle::an_unsigned_bundle_is_legal_and_says_so`, `sim::the_members_are_checked_in_order_and_the_order_is_named`.

Signature (INPUTS item 5):
- bare-bytes signature by a held key refused: `bundle::a_signature_over_the_bare_manifest_bytes_by_a_held_key_is_refused`, `sim::a_signature_over_the_bare_manifest_bytes_by_a_held_key_is_refused`.
- held key that does not verify refused: `sim trial.rs::a_bad_signature_from_a_known_key_and_a_malformed_record_are_refused_and_leave_nothing`, `bundle::changing_one_byte_of_slot_json_breaks_the_signature`.
- unknown key not refused: `bundle::a_key_the_reader_does_not_hold_is_reported_not_refused`, `sim::an_unknown_key_is_not_checked_and_not_refused`; sim `trial.rs::signature_outcomes_are_verified_unsigned_and_unknown_key_and_installed` (the four outcomes through the install).
- malformed record is `INVALID_ARGUMENT` whatever the keys: `bundle::a_malformed_signature_record_is_refused_whatever_the_key_list`, `sim::malformed_records_are_invalid_argument_whatever_the_key_list`.
- security on: `sim_node.rs::security_on_refuses_unsigned_and_untrusted_bundles_and_keeps_nothing` and `trial.rs::security_on_refuses_to_make_an_unverified_slot_the_default` (both unchanged in meaning, moved to the new form).

Boot-file rules (INPUTS "Boot-file mismatch, AS LEMNOS'S CODE DOES IT"), one test per case, in `crates/testing/sim-node/tests/bundle_rules.rs` unless stated:
- a trial whose files differ fails by the failure path: `a_trial_whose_boot_files_do_not_match_fails_by_the_failure_path`.
- a default whose files differ is swapped with its valid previous at boot, no count: `a_default_that_does_not_match_is_swapped_with_its_previous_at_boot_with_no_count`.
- neither the default nor its previous matches: `no_slot_can_start_when_the_default_and_its_previous_both_do_not_match` (also: the restart comes again after the rescue wait, on the simulated clock).
- a default with no valid previous: `a_default_with_no_valid_previous_fails_by_the_failure_path_and_nothing_starts` (approximation, see above).
- restart repoints and proceeds: `a_restart_repoints_when_the_default_does_not_match_and_its_previous_does`.
- restart refused, board up: `a_restart_is_refused_when_neither_the_default_nor_its_previous_matches`.
- periodic check only reports: `the_periodic_check_only_reports_when_neither_matches`.
- per-file list text: `the_mismatch_list_names_each_file_missing_or_changed_joined_by_semicolons`.
- Atlas receives the board's text unchanged (node, `sim/trial.rs`): `the_board_restart_refusal_reaches_atlas_with_the_boards_text_unchanged`, `the_restart_repoint_note_reaches_atlas_unchanged`.

Command line (INPUTS item 6):
- a bundle whose cmdline lacks a source word is refused, naming the word: `bundle::the_cmdline_must_carry_the_word_for_each_boot_source`, `sim::the_cmdline_must_carry_the_word_for_each_boot_source`.

Outcome words (item 7): `crates/drivers/board-slots/src/signature.rs` tests `the_four_outcomes_have_their_board_words_and_plain_sentences`, `a_refusal_carries_the_boards_reason_unchanged`, `an_outcome_this_atlas_does_not_know_is_not_one_of_the_four`.

## Lock items (`// LOCK:` comments, what moves at the L2 lock)

- `crates/core/bundle/src/sign.rs`: the record, the message and the order are Lemnos's I5g2. Today Lemnos reads a bare 64-byte `slot.sig` and a `slot.key` member. At the move: nothing here changes except the vector if Lemnos changes it.
- `crates/core/bundle/src/boot.rs`: the cmdline rule. Today the board file has no `boot_args`/`boot_sources`. At the move the board's own values replace the default.
- `crates/testing/sim-node/src/bundle.rs` (three notes): the record and message (I5g2); the signature rule (I5g2); the `BootRule` shape (I5i4, I5i5). At the move these are Lemnos's, and the vector test checks them.
- `crates/testing/sim-node/src/slots.rs`: the `board_security` field is the sim's stand-in for `system.security/1` (S2, #255): `/etc/board/security`, precheck, `require_signed` removed. The gate on set_default and trial (S2). The `boot` field (L2, the boot rule). The `signature` word in a slot's list (L2, with `info`'s `key_ids`, not built here).
- `crates/testing/sim-node/src/slots/boot.rs`: the boot-file rules and their texts (block marked `// LOCK: Lemnos L2 (boot.rs, boot_guard.rs at e3e8a963)`). Today the sim models what Lemnos does at e3e8a963; the rescue wait (30 s) and the 600 s periodic period are the sim clock's values of Lemnos's `rescue_wait` and the boot guard period.
- `crates/testing/sim-node/src/node.rs`: the restart check on the reboot action, and the boot rule passed to `verify` (L2).
- `crates/drivers/board-slots/src/install.rs`: the signature is the bundle's `slot.sig` record, no `slot.key` (L2). `crates/drivers/board-slots/src/signature.rs`: the per-slot `signature` word is read once the L2 rev writes it (L2).
- Already present and unchanged, listed for completeness: `crates/testing/sim-node/src/slots/trial.rs` (L1 and L2 notes on the trial record and record_error), `crates/testing/sim-node/src/node.rs` (L1, M8a), `crates/drivers/board-slots/src/install.rs` (L1 `.incoming` sink), `crates/core/slots/src/list.rs` (L2 `signature` word), `crates/drivers/board-slots/src/lib.rs` (L1 forms).

## Not changed on purpose

- No Lemnos, Orion or Gaia pin is changed; the Lemnos lock is still `d6b9361d`. No file under `devices/` is touched.
- `info` (method 14) and `key_ids` reading are not built (later, per the brief).
- `system.security/1` is not built: the simulated board has a field, not the contract.
- The writer does not check the cmdline's word: Atlas does not hold a board's `boot_sources` at write time. The inspector and the simulated board refuse a bundle that lacks it. Atlas writes whatever cmdline the image carries, as before.
- An early check in the writer would read the board file the cmdline was generated from (`tree --cmdline-slot`): the bundle does not carry `boot_sources` (INPUTS, "(c)"). Not built here; the board-side check stays the authority.
- `crates/flash/disk` is unchanged. The first-flash record code is in `crates/flash/image` (`atlas_image::Record`), which `atlas-bundle` already depends on, so the bundle reuses it and nothing was moved.

## Checked

- `cargo test --locked -p atlas-bundle`: all pass (lib 4; tests/bundle 32+ including the new ones; tests/image 3; veritysetup 1; vectors 1).
- `cargo test --locked -p atlas-sim-node`: all pass (lib 12, identity_facts 14, bundle_rules 7, sim_node 34, trial 17).
- `cargo test --locked -p atlas-board-slots`: all pass (unit 3, slots 17, trial 10).
- `cargo fmt --all --check`: passes. `cargo clippy --locked -p atlas-bundle -p atlas-sim-node -p atlas-board-slots --all-targets -- -D warnings`: passes.
- The full gate (`GAIA_BIN=/mnt/ssd-samsung/atlas-next-scratch-gaia/bin-7177d72/gaia ./scripts/gate.sh`): see the PR for its result.
- No test depends on real time. The simulated clock is used for trials. No timing-sensitive test was added or changed.

## Not proven

- Nothing here runs against a real Lemnos. The vector agrees between two implementations in this repository, both written from INPUTS. It has not been checked by Lemnos's own code.
- Nothing is written to a board, flashed or installed. Untested on board.
- The boot and restart refusals are the sim's model. Lemnos has neither yet.
