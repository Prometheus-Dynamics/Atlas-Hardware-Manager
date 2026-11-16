# A7RAILS: the Cubie A7 rails are left as the maker leaves them

Stacks on #138 (PADS, pr/PADS 035caba6): this branch is based on pr/PADS. Not a draft. Do not merge.

## What this changes

- **A correcting kernel patch for the A7S**, applied after DockSeed's series (fe0a8ed): `devices/boards/radxa-cubie-a7s/kernel/a7s-rails.patch`.
  The series pins PMIC rails to fixed voltages that Radxa's kernel dts does not program. This patch makes ALDO1, BLDO1, DCDC1 and
  DCDC4 ranges (the maker's ranges) and removes the driver's bank-mode writes for PD, PE, PG, PJ and PK. CLDO5 (1.8 V, eMMC VCCQ
  and bank PC) stays fixed, because the schematic proves it. Each change is recorded with its source line in
  `kernel/edits.toml`.
- **Bank statements** for the A7S (`devices/boards/radxa-cubie-a7s/pads.toml`): every bank of the A733 is named. Three new
  forms (the bootloader sets it; the PMIC powers it up; no pads) are accepted only where the patch leaves the rail a range. PD,
  PI and PJ stay UNSTATED and refused. PA has no pins in the series' pin table.
- **The checker** (`scripts/check-devices.sh`, `pads_rule`): the two rail forms read the patch's post-image (`pads_rail_state`), and
  a rail the patch pins to a fixed voltage refuses the statement. Self-tests added. The value check no longer accepts any text
  that contains " to " (it was a loophole).
- **First-boot record**: `board.toml` comment and `docs/research/a7s-rail-voltages.md` section 11.
- **Research doc**: `docs/research/a7s-rail-voltages.md` (sources, rails, pinctrl reading, bank table, naming discrepancies, A7A/A7Z
  table).

## For Gaia's unprogrammed_rails check (not added to board.toml)

To be added when Gaia's rev with the key is pinned:

```
unprogrammed_rails = ["aldo1", "bldo1", "dcdc1", "dcdc4"]
```

Node names as they appear under the PMIC in the built A7S dtb. A7A/A7Z: the same four if the same correction is applied to their
dts (not in this PR).

## Facts this PR rests on (see the research doc for lines)

- Radxa's U-Boot (9da4b3e8, our bootloader) programs BLDO1, BLDO2 and CLDO5 (when its eMMC consumer probes) and does NOT program
  ALDO1 or CLDO1 (no always-on, no consumer). The regulator uclass applies a voltage only when min equals max.
- The regulator core moves a rail only when it is outside its [min, max] range.
- The pinctrl 2-bit variant reads the bank supply's voltage at pin request and writes 11 (1.8 V) or 10 (3.3 V). It guesses nothing.
  The driver writes only PC and PH after this patch.
- SPL's `gpio_init` copies the detected bank value into the mode register for A733 (`board.c:204-207`).

## Not verified

No board was touched and nothing was measured. The PMIC identity (AXP318W in the schematic and the series, axp8191 in Radxa's
files) is not settled. PD, PI and PJ are unresolved and refused. The PG, PL and PM rail (ALDO1 or CLDO1) and its power-up value
are unknown. The GPU rail (E4) has no operating point in this dts. The mainline 7.3.4 pinctrl core is not on disk.

## Stacking

The base is pr/PADS (#138). If #138 has merged into next when this is reviewed, this PR stacks on next.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

## Coordinator rulings applied (second commit)

- **PD and PJ** are "left as the PMIC powers it up", the supply cited as BLDO2 or SWOUT1 with both sources. The patch removes
  their supply lines from `&pio` (edit E8), so neither rail is named as the bank's supply in our dts. Mechanical check: the
  built dts names no supply and the driver writes no mode for the bank (`kernel/post-patch-facts.txt`).
- **PI** is "left as the PMIC powers it up" on the mechanical condition that the built dts names no PI pin (`pio-ref PI 0`).
  **PA** is "no pads" on the mechanical condition of zero pins in the SoC table (`bank-pins PA 0`). Both are checked by the
  rule, and a fact that disagrees refuses the image.
- **dcdc4 (GPU)**: case 1. The consumer and an OPP at 960 mV exist in our dts and dtsi, so the range is the maker's form and the
  OPP sets the voltage when the GPU runs (section 13.2 of docs/research/a7s-rail-voltages.md).
- **The driver reading** is from the image's kernel base: linux 7.3.3 (tarball sha verified) with the 379-patch series and the
  correction applied, and the 7.3.4 sunxi pinctrl files of the gate's cache give identical results. The four points are cited
  in section 13.1 of the research doc; nothing differs from the 6.1 copy.
- **A7A and A7Z**: edits files and facts from their own Radxa files (no edits needed, no bank supplies, no fixed rail). Their
  pads.toml state every bank except PI, which is refused (no pin groups on disk). Both still refuse, on PI.
- **#138 merged**: origin/next is merged into this branch.

## Build wiring and verification (third commit)

- **The correction is a build input.** The A7S board declares `patches = [{ source = "a733-kernel-series", path = "patches" }, { file = "kernel/a7s-rails.patch" }]` (Gaia takes a board's list in place of the family's, so the series is named on the board). `gaia plan` (a6299c62, a scratch probe with C1's A7S lock, not committed) gives the kernel as `patches0` (the series) and then `patch1` (`boards/radxa-cubie-a7s/kernel/a7s-rails.patch`). See docs/research/a7s-rail-voltages.md section 13.7.
- **The gap is refused.** A pads.toml that names a correction must find it in the board's or family's `[kernel] patches`; the self-test covers the case.
- **The facts are verified, not recorded.** `check-kernel-fragments.sh` applies the series and then the board's patches (`--fuzz=0`, a reject fails) to the pinned 7.3.4 tree, and runs `scripts/check-board-facts.py` on it: 53 recorded facts match (io_bias_banks, supply lines, PI references, pin table, the four unprogrammed rails as ranges). A negative (a PD supply) is refused. The step runs only for boards with kernel patches.
- **PH and PC** (docs/research/a7s-rail-voltages.md 13.6): PH's SWOUT2 has no voltage ops, so its read-back is its parent DCDC1's (core.c:4822-4823), never a 1.8 V mode on a 3.3 V output. PC is a fixed 1.8 V applied at registration, read back as 1.8 V.
- **A7A and A7Z** stay refused on PI only (13.8 says what lifts it).
- **dcdc4** stays on the unprogrammed list (a range); GPU scaling is the OPP's 960 mV when the GPU runs.
