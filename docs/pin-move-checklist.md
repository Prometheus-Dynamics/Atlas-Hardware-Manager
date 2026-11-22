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
11. **Images.** Build each product the move touches with the pinned Gaia, signed. Record the version, slot name
    and sizes (`.flash`, `.slot`, `.bundle`), before and after.
12. **Board profile.** Run the move's bar in Gaia's board profile (`scripts/qemu-rpi-slots.sh --profile board`)
    at the pinned Gaia. Reach Orion the way the board is reached (the USB gadget through `--usb-host` and
    `--usb-host-forward`). Report each line of the bar with the log lines that show it.
13. **Board.** Send the run sheet to the coordinator and wait for its approval before any board contact. Flash
    through atlas-stage and the RAM bed. Hold the board lock (`flock -w /run/lock/raze-gpu.lock`).

## After

14. Report the landed head, the revs, and `ATLAS_SCRATCH_ROOT` to the coordinator.
