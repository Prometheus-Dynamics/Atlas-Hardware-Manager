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

## As built (2026-11-14)

**Pins.** Gaia moved on this branch to `b8585a049326ed29ef6eb37a7afde56a3acfda8c` (coordinator's change; its recipes equal
`b3fa40f`'s). The Lemnos pin (`d6b9361d`) and the Orion pin are unchanged. The bench-key public half is the only key
material committed (`devices/components/raze-lemnos-board/files/slot-keys`).

**Lemnos build.** `lemnos-ctl` built `--locked -p lemnos-ctl` (profile dev, `CARGO_TARGET_DIR` in scratch) at
Lemnos `7d7fabab` (`origin/pr/I5j`, "I5j: lemnos-ctl writes and verifies a slot bundle"). `origin/next` does not have
I5j yet (its tip was `092c32ed` when fetched; `7d7fabab` is the held I5j head, parent `bfbe88f4`). `lemnos-ctl bundle` exists at 7d7fabab.

**Gaia, gating binary b3fa40f, then b8585a0 (the pin).** Products validated and planned with the committed locks;
`gaia build --config-only` (warm store `store-build`) passed for the Raze product with b3fa40f (exit 0, 20 s).

Raze product plan (b8585a0, `gaia plan ... --json`): 111 steps; `raze-base::erofs` constant `compression = lzma`;
`raze-base::boot` size 82837504; `raze-base::bundle` runs as written (`key_args` `--key-file @key@ --expect-key-id @key-id@`,
`verify_key_args` `--pubkey @pubkey@`); `raze-base::cmdline` base `dhash_entries=16384 ihash_entries=8192 thash_entries=4096 uhash_entries=1024`.

`gaia validate` of product, board and component: `ok` (rc 0). `check-devices.sh` with b8585a0: rc 0, eight products `ok`.
`check-config-only.sh` with b8585a0: all eight products `ok`, `passed`.

**What the locked (c6a3d10) binary and the first pin did not accept, found on the way.** At b8585a0 (not c6a3d10):
- every product making a slot (`sd-image` and `slot-boot`) needs `BR2_PACKAGE_HOST_EROFS_UTILS=y` and `BR2_PACKAGE_HOST_CRYPTSETUP=y`
  in its OS fragment: refused by name without. Added to raze.fragment, rock-4d-base, rock-5c-base, zero-3e-base and
  cubie-a7-bringup (the bringup products inherit or carry these), and to the good-sd-image fixture.
- the recipe default for `compression` is now `lzma`, so the sd-image products plan with lzma; their families and the
  bcm2712 family state `CONFIG_EROFS_FS_ZIP_LZMA=y` and `CONFIG_EROFS_FS_ZIP_LZMA_DEFAULT_MAX_STREAMS=4` beside the zstd lines.
- `scripts/check-devices.sh`: `SLOT_RECIPES` gained `slot-boot`; the erofs self-test now swaps the explicit
  `compression` line (lz4 case) and removes it (recipe-default case, refused unless the kernel has LZMA).

**Lemnos bundle, flag by flag** (`lemnos-ctl bundle --help` at 7d7fabab vs `[generate.bundle]`): every flag in
`args` (`--layout-of --socs --slot --payload --rootfs --verity-table --product --version --payload-out --out`),
`key_args` (`--key-file --expect-key-id`) and `verify` / `verify_key_args` (`--verify --pubkey`) is in the usage line.
Nothing missing, nothing extra. The bundle itself was NOT run (it needs a signing seed; the seed was not opened).

**`tree` (the component's args, I5j 7d7fabab, no `--boot-dir`):** rc 0; outputs overlay.dts (2958 bytes),
modprobe.conf, cmdline.words (86 bytes), cmdline-slot.words (`os.slot=raze-base-2026.1.0-dev.1-l2d os.roothash=@roothash@
os.src.rpi-boot-mode-1=/soc@107c000000/mmc@fff000 panic=5`), manifest-boot.json (`{"overlay":{"board":"raze","sha256":...}}`).
`dtc -@` compiles the overlay to 1,823 bytes; `fdtoverlay` applies it to the CM5 base dtb (79,751 bytes, sha256 f773b476...,
the dev-image copy) with rc 0. The base dtb used is the dev image's copy, not a dtb from our kernel build.

**Board validate:** `lemnos-ctl validate --socs socs --no-live devices/boards/raze/lemnos-board.toml`: rc 0, "board raze, 21 devices".

**Boot partition (I5j tool; payload of the real sizes, the kernel a stand-in of 6,773,267 bytes, the bundle's files
not built):** `boot-partition --layout-of ... --min-size --payload DIR` gives `kernel 6773267 10485760`,
`initramfs 495328 1048576`, `device-tree 79751 131072`, `cmdline 249 4096`, `board-overlay 1823 16384`; total minimum
**35,245,568** bytes (was 38,242,816 with the earlier 11 MiB kernel and 90 KiB dtb bays). `boot_size` 82,837,504 still fits, so
it is kept. Writing at `--size 82837504` gave an image of that size and `--verify` printed `ok` for every check (rc 0).
`--size 38000000` is refused (not a whole number of sectors; the minimum is still the one printed by `--min-size`).

**Bay sizes (board file `[boot]`, flash-time sizes, set 2026-11-14 by the coordinator's direction):** a bay cannot grow after
the image is built, so each has room past today's file. Lemnos's written rule (`docs/slots.md`, `docs/boards.md`) is the
largest file a future slot may bring plus a margin (the board file's comment there says 10 %), rounded up. These are above
that 10 % floor for every role; where Lemnos's rule and the coordinator's words differ, the larger value meets both.
| key | bytes | today's file | source of today's file | room |
|---|---|---|---|---|
| `bay_kernel_bytes` | 10,485,760 (10 MiB) | 6,773,267 | gzip -9n of the Image, 15,096,320 raw, built by Gaia c6a3d10 from `next` on 2026-11-14 (the trim is in the plan); coordinator's measurement | 55 %; Lemnos's 10 % floor is 7,450,594 |
| `bay_initramfs_bytes` | 1,048,576 (1 MiB) | 495,328 | slot-init newc archive (proven, one file `init`) | about 2x |
| `bay_device_tree_bytes` | 131,072 (128 KiB) | 79,751 | bcm2712-rpi-cm5-cm5io.dtb, sha256 f773b476..., the dev-image copy (same bytes as Raze #2) | 1.6x: the base tree changes with a kernel bump |
| `bay_overlay_bytes` | 16,384 (16 KiB) | 1,823 | compiled board overlay, `dtc -@` here | 9x: camera stream tables and later overlays grow it |
| `bay_cmdline_bytes` | 4,096 (4 KiB) | 249 | stored cmdline, the tool's figure | 16x |

Dated measurements: the kernel figure 6,773,267 is the 2026-11-14 build (coordinator's `gzip -9n`); the older
10,168,581 gzip figure (L2-partition-requirements 8.2, 2026-11-13) was measured before B29's trim landed and is STALE.

**Differences between INPUTS and the tools (found, not fixed by changing a tool):**
1. Kernel figures. INPUTS / the brief give 6,779,764 bytes gzip "today". The L2-partition note (8.2, 2026-11-13) gives
   10,168,581 for next's gzip -9 Image: that was measured BEFORE B29's trim landed on next and is STALE. The figure next
   really builds is 6,773,267 bytes (gzip -9n of the 15,096,320-byte Image, Gaia c6a3d10 build from next, 2026-11-14,
   measured by the coordinator). The brief's 6,779,764 is close to it and is not the value in use. The kernel bay is 10 MiB.
2. INPUTS said the slot compression should be set in the image; the b8585a0 recipe's default is lzma and the product also
   states it. No conflict, noted only.
3. `lemnos-ctl boot-partition --layout-of BOARD --min-size --payload DIR` (the Gaia `fit` form) is accepted; the usage line
   shows `--min-size` without `--layout-of`. Both forms exit 0.
4. The gaia b8585a0 checks now require the host erofs and cryptsetup lines and `EROFS_FS_ZIP_LZMA*` in the families. INPUTS
   described the first as the coming Gaia rule; it is in force at the pin.
5. `tree --bridge-env` exists at I5j (R1j). The hand-written `lemnos-orion-links.conf` drop-in is kept until the move.
6. The slot id and version are literal (placeholders: count 1, commit "l2d"). No check holds `slot` = `version` with `+`
   read as `-`. INPUTS' example `raze-base-2026.1.0-dev.993-<commit8>` is the shape the move uses.

**Gate.** See the PR text; the gate is run as `GAIA_BIN=...b8585a0... CARGO_TARGET_DIR=.../atlas-next-target-2 ./scripts/gate.sh`.

**Not proven.** No image was built (only config-only). No bundle was written or verified (no seed read). The boot partition
was not booted. The overlay was applied to a dev-image dtb, not to a dtb from our kernel build. The `dtc` here is not
Buildroot's host dtc. The plan's `mkfs_erofs` is the host's `/usr/bin/mkfs.erofs`; the build uses the image's own
(Buildroot's 1.8.10 pin). `fdtoverlay` from `BR2_PACKAGE_HOST_DTC` is assumed (Gaia's example fragment does the same). The
Raze's `slot-keys` line is the bench key, a test credential, milestone 2 only.

## The move (Lemnos only; Gaia and Orion are already at their final pins for this branch)

Gaia is at b8585a0 on this branch. Orion is unchanged. The Lemnos edits when the L2 rev is sent:
1. Lemnos lock: every `[sources.lemnos] ref` (components `lemnos`, `lemnosd`, `lemnos-ctl`, `lemnos-orion`, `lemnos-slot-init`,
   `raze-lemnos-board`, and any other Lemnos ref) from `d6b9361d...` to the L2 rev. The L2 rev must contain I5j (`bundle`),
   the bay keys (I5i8), `boot-partition`, and the I5i7 / I5i9 refusal texts. Then `check-devices.sh --write-locks`.
2. Lemnos board files: run `lemnos-ctl validate` from that rev over EVERY Lemnos board file we hold (Raze, zero-3e,
   pending files stay out). Re-check the bay sizes with the first Raze build's `role stored_bytes` line (`gaia plan` and
   `boot-report`); change the five keys only if a stored file moved.
3. `lemnos-orion-links.conf` (the hand-written `LEMNOS_ORION_LINKS` drop-in) goes when `tree --bridge-env` lands in the
   generated outputs (R1j), and `--bridge-env` is added to the `[generate]` args.
4. Version and slot: replace the placeholder count and commit with the real ones of the Atlas tree the image is built from;
   the slot id is the version with `+` read as `-`, lowercase. Gaia's `slot` setting and the product's `version` change together.
5. Compression: measure `lzma`, `lz4hc` and zstd on the first Raze image and set `compression` (and, for zstd, the kernel's
   `EROFS_FS_ZIP_ZSTD` is already on for the Raze).
6. The `[kernel] image = "Image.gz"` and `cmdline` on the Raze are already in; the B29 trim changes the kernel bay only.
7. Re-run `gaia build --config-only`, then the full gate with `GAIA_BIN` set.
