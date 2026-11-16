# PATCH-LISTS: the effective patch list, and the A7S flip under Gaia's append rule

Stacks on #140 (pr/A7RAILS). A draft: it must not merge before Gaia's append rule is pinned (see "The pin").

## The rule

Gaia a6299c62 REPLACED a board's patch list with the family's (`crates/core/describe/src/board.rs`, the merge
`patches: b.patches.or(f.patches)`, lines 643, 657, 662). Gaia's append rule (`gaia/wt/gaia-pr-patchappend`, commit 8f5eb89b,
`join_patches` at board.rs:632-647 and `check_patch_lists` at board.rs:675-721) APPENDS:

- the effective list is the family's entries, then the board's;
- `inherit_patches = false` in the board's `[kernel]`, `[bootloader]` or `[bootloader.trusted_firmware]` section takes the board's
  list alone;
- a board entry that repeats a family entry while inheriting is REFUSED by Gaia.

Atlas follows the append rule. Nothing here adds a check that a board must restate its family's entries (the opposite of the rule).

## What changed on this branch

- `devices/families/sun60i/family.toml` `[kernel]`: `patches = [{ source = "a733-kernel-series", path = "patches" }]`, the same entry
  pr/C1 declares (C1 family.toml:112).
- `devices/boards/radxa-cubie-a7s/board.toml`: `patches = [{ file = "kernel/a7s-rails.patch" }]` only. The effective list is the
  series, then our correction.
- `scripts/check-kernel-fragments.sh`: the kernel patches applied after the series are the effective list's `file` entries (family
  then board, or the board's alone with `inherit_patches = false`).
- `scripts/check-devices.sh`: `effective_has_file`. A pads.toml that names a correction must find it in the effective list
  (family, then board), compared by resolved path. The self-test covers a correction listed on the family, and a board that opts
  out with `inherit_patches = false` (refused).

## The pin (not moved here)

`devices/gaia.lock` stays at a6299c62. The flip is not safe under that binary. Proof (below): the pinned binary builds the A7S kernel
from `a7s-rails.patch` alone, so the series is dropped. The flip goes in with the Gaia pin move, once the merged rev is sent.

## Plan evidence

Probe: a scratch product (not committed; `cubie-a7s-bench` to take C1's A7S lock) with the A7S board and this branch's devices.

- Pinned a6299c62 (replace): the kernel is `patched` from `["a7s-rails.patch"]` only. The series is NOT in the build.
- Append 8f5eb89b: the kernel is `patched` from `["patches", "a7s-rails.patch"]`, the series first, then the correction (the
  `radxa-cubie-a7s.a733-kernel-series` source is in C1's lock).

Both binaries validate the flipped board and family (`gaia validate`).

## The audit: every board that declares a patch list

The boards that DECLARE a list (a board with no list inherits the family's, unchanged under both rules, and is not in the table).
Scanned with `git archive` on each ref; nothing on any branch was changed. `Refused` means Gaia's append rule refuses it as it stands.
`Changes` means it keeps its meaning only if the family has no list.

| Tree | Board | Kind | Family list | Board list | Under append |
|---|---|---|---|---|---|
| origin/pr/A7AZ | radxa-cubie-a7a | kernel | series | series, `patches/linux` | **Refused**: restates the series. Fix on A7AZ: board keeps `patches/linux` only |
| origin/pr/A7AZ | radxa-cubie-a7z | kernel | series | series, `patches/linux` | **Refused**: restates the series. Same fix |
| origin/pr/A7AZ | radxa-cubie-a7z | bootloader | a733-bootchain `patches/u-boot` | the same, `patches/u-boot` | **Refused**: restates the family's U-Boot patch. Same fix |
| origin/pr/A7RAILS (this PR, before the flip) | radxa-cubie-a7s | kernel | none | series, `kernel/a7s-rails.patch` | Unchanged (family has none). The flip (this PR) moves the series to the family: board `kernel/a7s-rails.patch` only |
| origin/pr/R1 | radxa-rock-5c | bootloader | none (rk35xx family has no bootloader list here) | `assets/patches/u-boot/0001-rock-5c-hold-usb-otg-vbus-low.patch` | Unchanged (family has none) |
| origin/next | none | | | | none declares a list |
| origin/pr/Z2 | none | | | | none declares a list |
| origin/pr/AU1 | none (waveshare-luckfox-aura declares none; family rv11xx lists kernel and bootloader) | | | | Unchanged: inherits the family's lists |
| origin/pr/C1 | none (a7a, a7s, a7z declare none; family lists kernel, bootloader, TF-A) | | | | Unchanged: inherits. Note C1's family lists are the same entries this PR moves to the family (kernel) |
| origin/pr/K1 (branch and worktree, pending files: none) | none | | | | none declares a list. K1 has no K230 board in this repo |
| Gaia b8 `gaia/wt/b8-k230d` (read-only) | k230d-ai-cam | bootloader | k230 `patches/u-boot-v2022.10-k230d.patch` | the family's patch restated, then `k230d-ai-cam-console-uart0.patch` | **Refused** under append: restates the family's U-Boot patch. Declare: the family patch stays on the family (already there); the board lists ONLY the uart0 edit |

Under append, the K230D board's correct declaration is `[bootloader] patches = [{ file = "assets/patches/u-boot-v2022.10-k230d-ai-cam-console-uart0.patch" }]`
(the family's `u-boot-v2022.10-k230d.patch` applies from the family, first). The k230d-ai-cam board is Gaia's b8 branch, not an Atlas
branch: this table is the record; the change is Gaia's to make on b8.

The A7AZ rows are on another PR (#134 / pr/A7AZ). Not fixed here: the coordinator said not to fix other branches. They become refused
the moment Gaia's append rule is pinned, so pr/A7AZ must be corrected before the pin, not after.

## The built-image proof (after the flip)

The effective list is proven from a BUILT image, not from the files. The next K230D and A7S builds with Gaia's built-artefact checks
must show:

- **A7S (radxa-cubie-a7s)**:
  - the kernel source record (the `linux.patched` build input) lists, in order, the series' 379 patches (`patches`) and then
    `kernel/a7s-rails.patch`; a build without the series would show `a7s-rails.patch` alone and must be refused;
  - the built `sun60i-a733-cubie-a7s.dtb` has `regulator-min-microvolt` not equal to `regulator-max-microvolt` for `aldo1`, `bldo1`,
    `dcdc1` and `dcdc4` (the `unprogrammed_rails` check, `["aldo1", "bldo1", "dcdc1", "dcdc4"]`, to be added when Gaia's key is pinned);
  - the built dtb's `&pio` has no `vcc-pd-supply` and no `vcc-pj-supply`, and no pin references bank PI (the facts file's
    `pio-supply PD none`, `pio-supply PJ none`, `pio-ref PI 0`);
- **K230D (k230d-ai-cam)**:
  - the U-Boot source record lists the family's `u-boot-v2022.10-k230d.patch` first, then the board's uart0 edit (once the board is
    declared that way);
  - the built U-Boot dtb's `stdout-path` is `uart0:115200n8`.

## Not in this PR

- Gaia's rule change and its pin (the coordinator's merged rev).
- Other branches (A7AZ, b8's K230D board): the table is the record.
- The bootloader and TF-A patch lists on the sun60i family (C1 declares them; this PR moves only the kernel series).
