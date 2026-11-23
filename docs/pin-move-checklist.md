# Pin move checklist

A pin move changes the Lemnos, Orion or Gaia rev that Atlas builds on. Go through every step, for every move.
The move's PR names each step's result.

## Before the move

1. **Use only revs the coordinator has sent or confirmed.** A rev seen in a peer's branch, log or message is not
   a pin until the coordinator names it for this move.
2. **One rev per repo.** Every Lemnos component in a product is on the same rev. `scripts/check-devices.sh`
   checks this. The one named exception is `flasher-lemnos-way-back` in `products/flasher-bcm2712` (see the
   component's comment); a new exception needs a ruling and a self-test in the script.

## The move

3. **Component refs.** For each `devices/components/*/component.toml` that names the repo, change the `rev` and
   rewrite its comment so it says the new rev, what it adds, who sent it (coordinator and date, pin move N) and
   the rev it replaces (`before:`).
4. **Crates.** Change the git revs in the workspace `Cargo.toml` and in any crate's `Cargo.toml`. Update
   `Cargo.lock` for those sources only (`cargo update -p <crate> --precise <rev>`), with no unrelated bumps.
5. **Gaia.** Change `devices/gaia.lock` (`commit =`, with the reason comment), then rewrite the products' locks
   with `scripts/check-devices.sh --write-locks`.
6. **JOB_WAIT.** Read Lemnos's bound on a board-side flash commit at the new rev:
   `git -C <lemnos> show <rev>:service/orion/src/stream.rs | grep -n JOB_WAIT`.
   It must equal `BOARD_COMMIT_CEILING` in `crates/board/files/src/bounds.rs` (900 s at Lemnos a6251ecf,
   "a backstop, not a pace"). If it moved, the constant, its line citation and `STEP_MARGIN` move in the same
   PR. If the line moved, update the citation.
7. **Other bounds.** Read the bounds table in Lemnos's `docs/daemon.md` at the new rev (the flash commit ceiling,
   and the install and install_file stall bounds). A bound that changed is named in the PR, with what Atlas does
   about it.
8. **No workaround.** A fault in Lemnos, Orion or Gaia goes to its owner with a repro. Atlas does not work around
   it without a ruling.

## Checks

9. `scripts/check-devices.sh --self-test` and `scripts/check-devices.sh`.
10. The scoped check (`scripts/check-scoped.sh`), then the full gate (`scripts/gate.sh`) with the UI gate.
    - The gate runs in a target directory of its own (`CARGO_TARGET_DIR`): `gate.sh` refuses one another worktree built
      in, because cargo would take that worktree's newer artefacts as fresh for this tree's sources.
    - Nothing else of yours runs beside the gate: no second gate, build, `check-devices.sh` or Gaia plan. On
      2026-11-23 a gate failed on two UI tests that timed out while `check-devices.sh` ran beside it, and passed
      alone. A red gate is explained by its cause before it is run again; the host's load average at its start and
      end goes in the report.
11. **Images.** Build each product the move touches with the pinned Gaia, signed. Record the version, slot name
    and the sizes of the image (`.flash`) and of the slot inside it (`.slot`), before and after. The sizes of every
    output are in the build's printed listing and in `<out>/<product>.outputs.txt`; `--out` copies the one OS image only.
12. **Board profile.** Run the move's bar in Gaia's board profile (`scripts/qemu-rpi-slots.sh --profile board`)
    at the pinned Gaia. Reach Orion the way the board is reached (the USB gadget through `--usb-host` and
    `--usb-host-forward`). Report each line of the bar with the log lines that show it.
13. **Board.** Send the run sheet to the coordinator and wait for its approval before any board contact. Flash
    through atlas-stage and the RAM bed. Hold the board lock (`flock -w /run/lock/raze-gpu.lock`).

## After

14. Report the landed head, the revs, and `ATLAS_SCRATCH_ROOT` to the coordinator.
15. **Every marker.** At each pin move, before step 14 is sent:
    - List every `LOCK`, `PROVISIONAL` and "to be checked" marker in the tree (`grep -rn` over the tracked files, not
      `docs/prs/`, which is PR history and is not checked).
    - Re-verify each marker that names Lemnos or Orion against the new revs, at the board's file:line. Then update the
      marker to the truth at the new rev, or remove it. A marker that names a rev names the new one.
    - Every board-facing string constant either imports the board's exported item, or carries a citation on one of the
      3 lines above it. The citation form is a comment that names the repo, the pinned rev, and the board's file:line:

          // board: Lemnos e199fcb9 service/orion/src/files.rs:38 (DESTINATION_PREFIX)
          // board: Orion b01a7fb2 crates/client/files/src/push.rs:411

      The rev is a prefix of the pinned rev (at least seven hex digits). A citation may not name a rev the pin does not
      carry, and neither may a `LOCK` block.
    - Run `scripts/check-board-names.sh` and `scripts/check-board-names.sh --self-test`. The script fails when:
      (a) a `LOCK` line that names Lemnos or Orion has no rev in its comment block, or names one the pin does not carry;
      (b) a `board:` citation names no rev, or a rev the pin does not carry;
      (c) a board-facing string constant has no citation within 3 lines above it (its value starts with `lemnos.`,
          `lemnos/`, `system.`, `/data/`, `/etc/lemnos`, `/run/orion` or `/etc/board`, or is a contract tag `x.y/N`);
      (d) `ATLAS_BENCH_NO_RENEW` appears outside `crates/tools/next/src/hold.rs` and the docs.
      The pinned revs are read from the workspace `Cargo.toml` and from `devices/components/*/component.toml`, so a
      component on another Lemnos rev (a flasher's) is pinned too.
    - The script must flag zero before the move is reported.
