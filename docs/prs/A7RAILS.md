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
