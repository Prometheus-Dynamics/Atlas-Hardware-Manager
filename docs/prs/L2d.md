# L2d: raze-base on the `slot-boot` recipe with `[generate.bundle]` (DRAFT, held for the L2 revs)

The one thing Gaia waits on for the first Raze image. Written NOW against Gaia's gating branch and Lemnos's tip so
that, when the coordinator sends the two revs (Lemnos L2, Gaia paired), ONLY the pins move. This branch sits on
top of the held Raze board-file draft (PR 92, `l2/raze-board-file`): read docs/prs/L2b.md first, it is this
branch's base and its "What changes at the move" and "Changes still to come" sections are part of this work.

Every fact comes from /mnt/ssd-samsung/atlas-next-scratch-root/INPUTS.md (read ALL of it; later sections
override earlier ones) and from the tools' own code and output. Where INPUTS and a tool disagree, the tool wins:
follow it and write the difference down here.

## The tools to write against (drafting only: NO pin or lock on this branch moves to an unsent rev)

- Gaia: the binary at `/mnt/ssd-samsung/atlas-next-scratch-gaia/bin-<rev>/gaia` built from
  `origin/pr/slot-boot-bundle` (the orchestrator names the exact directory), recipes and docs in
  `/mnt/ssd-samsung/atlas-next-scratch-gaia/src` (checked out at that rev: read `recipes/slot-boot.toml`,
  docs, and examples there). Use it with `GAIA_BIN=...` and `--recipes .../src/recipes` for `validate` and `plan`.
- Lemnos: its tip that has `lemnos-ctl bundle` (I5j). Lines: /mnt/nvme-wd/lemnos-gates/I5j-lines.md. Also
  /mnt/nvme-wd/lemnos-gates/L2-partition-requirements.md, I5i7-requirements.md, I5i8-manifest-answer.md.
  Build `lemnos-ctl --locked` from a `git worktree` of /run/media/sozo/bd1d96d9-fa81-4fac-b25e-193cfcac2dcb/Github/lemnos
  placed under /mnt/ssd-samsung/atlas-next-scratch-L2d (its own target there too; delete both before the final
  report), as L2b did. If I5j is not on `origin/next` yet, find its branch (`git branch -r`) and say which rev.

## What the branch must hold

1. `devices/products/raze-base/product.toml`: the image on recipe `slot-boot` with settings `slot`, `device`,
   `boot_size`, `product`, `version`, `sign_key_env`, `sign_key_id`, `sign_pubkey_env`. The settings name two
   environment variables and a key id; NO key material and no path of a key in any file. Variable names:
   `ATLAS_BENCH_SEED_FILE` and `ATLAS_BENCH_PUBKEY_FILE` unless Gaia's recipe fixes other names. Key id: the bench
   key's (INPUTS, "THE BENCH KEY"); say in a comment that it is the bench key and what must change for any
   image that leaves the bench. `version`: `2026.1.0-dev` form per INPUTS ("Lemnos (next bfbe88f4): version,
   product, slot id") and the slot id derived from it by the one rule there (no `+`, lowercase, <= 64 bytes); if
   the product file cannot compute them, state them literally and say where the one rule will live.
   `compression = "lzma"` per INPUTS; `initramfs = ["lemnos-slot-init"]`.
2. The Lemnos component (`devices/components/lemnos` and/or `raze-lemnos-board`, wherever `[generate]` lives
   today): `[generate]` (tree without `--boot-dir`), `[generate.provides]` roles (board-overlay, manifest-boot,
   cmdline, cmdline-slot), `[generate.image]` (the `boot-partition` tool: fit/args/verify), and
   `[generate.bundle]` EXACTLY as INPUTS gives it ("The bundle wiring, FINAL SHAPE"), unless I5j-lines.md or the
   gating recipe differ: then theirs, with the difference written down.
3. The board data: `[kernel] image = "Image.gz"` and `[kernel] cmdline` (base words) on the Raze; the Raze's
   Lemnos board file: the five `bay_*_bytes` keys (`bay_kernel_bytes`, `bay_initramfs_bytes`,
   `bay_device_tree_bytes`, `bay_cmdline_bytes`, `bay_overlay_bytes`) sized from real figures with stated headroom
   (kernel: 6,779,764 bytes gzip today; initramfs: 495,328-byte archive; take the others from a `tree` run and the
   built dtb), `boot_sources`, and what L2b.md lists as still to come.
4. The OS fragment: host `dtc`/`fdtoverlay`, `BR2_PACKAGE_HOST_EROFS_UTILS`, `BR2_PACKAGE_HOST_CRYPTSETUP` as
   INPUTS names them; `lemnos-slot-init` moves from `components` to `initramfs` (its `to = "/init"` stays).
5. The key list on the image: `[security] keys` names a file on the board; that file is installed with exactly
   the bench key's line (`<public key> bench key, not for any released image`): the PUBLIC key only.
6. `devices/gaia.lock`, the product's `gaia.lock` and the Lemnos pin: UNCHANGED on this branch. Instead, a
   section here, "The move", listing the exact edits to make when the revs arrive (files, lines, commands such as
   `check-devices.sh --write-locks`), so the move is mechanical.
7. `scripts/check-devices.sh` rules that the new shape breaks or needs (the slot-root plan rule reads
   `<image>::erofs`; SLOT_RECIPES names `sd-image|slot`: `slot-boot` joins it; the LZMA stream rule now applies):
   adjust, each with its self-test.

## Proof (as far as it can go before the revs)

- `gaia validate` and `gaia plan` of raze-base with the gating binary and recipes: paste the plan's steps and
  outputs. `gaia build --config-only` passes.
- `lemnos-ctl` at the tip: `validate` of the Raze board file; `tree` with the component's real arguments; and
  `bundle --help` (or its usage) compared flag by flag with `[generate.bundle]`.
- The repository's gate at the LOCKED Gaia will fail on this branch where the locked Gaia refuses the new forms:
  expected. Run `GAIA_BIN=<gating binary> ./scripts/check-devices.sh` and say exactly what passes and what fails
  with each binary, pasting the failures. Do not loosen a check to get green.
- NOT a full build (the orchestrator starts builds, one at a time). Nothing is booted.

## Not in this branch

Any pin move; the simulated board; the app; rk35xx and sun60i products (parked); merging PR 89 or PR 92.
