# Route A rehearsal and board scripts (D1, Raze, Lemnos b78e5317)

A save of the scratch tools behind route-A-steps.md. Not product code, not wired into any gate.

- `rehearse-route-a.sh` + `con.py`: the emulated rehearsal of route A (steps 0-4 and the endings) on Gaia's QEMU
  harness (`git show 6efa33ed:scripts/qemu-rpi-slots.sh` in Gaia; not copied here). Needs a scratch env with the
  board serial, the replica path and the build outputs.
- `board/board-route-a.sh`: steps 1-3 and step 3's undo on a real board over 993's SSH (`board/r2`), the same
  commands as the rehearsal. `board/ttycon.py`: one command on the board's gadget console (ttyGS0 on the PC).
- `board/cp7.sh`, `board/trial-checks.sh`: checkpoint 7's loop (with the 70 °C / line-45 stop) and the read-only
  step-4 checks, staged on /data for the trial slot.
- `board/raze.env.example`: the scratch env's keys. The filled env (serial, link address, paths) stays out of git.

Never committed with these: the replica or anything built from a board's p1 dump, p1 images, keys or seeds.

The first board run (Raze #2, 2026-11-20) halted in the Pi bootloader on bay0 (route-A-steps.md, "Finding"); step 4 is
blocked by the standing precondition at the top of the step list.
