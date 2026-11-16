# PADS: the pad-table provenance rule

Lands the rule that stops a product from building on a pad table that has no provenance. Nothing here changes a board's
build, boot or image. The rule refuses a product whose board's pad table is copied, unsourced, unstated, or missing.

## The rule

Sidecar `pads.toml` beside each family and board is the permanent shape. Gaia reads the description files with
`deny_unknown_fields`: an unknown key in `family.toml` or `board.toml` is refused (probe: `families/rk35xx/family.toml:38:
bootloader.applies_pad_table: unknown key`, gaia 2.0.6-dev at d8441c9). `pads.toml` is not a description, so Gaia never
reads it, and the rule reads it directly.

1. **A family states whether its bootloader applies a pad table**, in `devices/families/<f>/pads.toml`:
   - `applies_pad_table = true`: every board of the family states its own `pads.toml`.
   - `applies_pad_table = false` with `pad_table_reason = "..."`: no table applies, and the reason is required.
   - Neither: the family is refused ("states no pad-table fact").
2. **A board of a flagged family states one of two things**, in `devices/boards/<b>/pads.toml`:
   - `applies_pad_table = false` with a `pad_table_reason`, which carries the evidence for both the bootloader and the
     kernel. Accepted the way the bcm2712 family statement is accepted. If either side applies a table, the statement is
     false and the voltages are required.
   - Or the table itself: `source` (where it came from, with the path and lines), `derived_from_board` (empty means the
     board's own table; a board id means it was copied and is refused), and `bank_voltages` (every bank the table drives,
     stated, not inherited).
3. **A range is accepted only for a switchable bank**, and only with `switched_by` in `pads.toml` naming the part that
   switches it and both endpoints (1.8V and 3.3V).
4. A product-local board (`devices/products/<p>/board/`) takes its own `pads.toml` beside its `board.toml`.
5. The rule runs over each product's board. A board with no product is not built, so it is not refused here.

The refusal messages name the board and the reason, and point at `pads.toml`. `pads_self_test` proves: a sourced, stated table
passes; a board stating no table applies, with a reason, passes; a family that says `false` with a reason passes; and it
refuses each of: a derived table, no provenance, bank voltages not stated, no table at a flagged family, a board claiming
`false` without a reason, a family stating `false` without a reason, a family stating no fact, a range without `switched_by`,
and a range without both endpoints.

## Families (devices/families/<f>/pads.toml)

| Family | Statement | Source |
|---|---|---|
| rk35xx | `applies_pad_table = true` | U-Boot v2026.10 applies the io-domain table at boot where the io-domain driver is on: `CONFIG_ROCKCHIP_IODOMAIN` defaults on for RK3568 (`research/u-boot/drivers/misc/Kconfig:114-121`), not in SPL (`CONFIG_SPL_ROCKCHIP_IODOMAIN` off, `Kconfig:127`). Boards state their own table or their own `false`. |
| sun60i | `applies_pad_table = true` | The A733 boot chain reads the board's PIO setup from its device tree. Each board states its table. |
| bcm2712 | `applies_pad_table = false`, reason "pads are set by the Pi firmware; not applicable" | No Atlas bootloader applies a pad table on this family. |

## Boards (devices/boards/<b>/pads.toml)

**radxa-cubie-a7s: stated (source: the board's own regulator nodes, cross-referenced to the schematic).**
- Whose: the Radxa U-Boot device tree at commit 9da4b3e8, `research/radxa-u-boot/arch/arm/dts/sun60i-a733-cubie-a7s.dts`.
  The DockSeed series dts (fe0a8ed) is not on disk and is not cited.
- The regulator nodes give the voltages: `reg_bldo1` "vcc-pe" 1.8 V (lines 178-183, name at 182); `reg_bldo2` "vcc-pd" 1.8 V
  (185-190, name at 189); `cldo1` "vcc-pm" 1.8 V (218-222, name at 221); `cldo5` "vcc-pc" 1.8 V (240-244, name at 243).
- Cross-reference to schematic V1.10 sheet 4 (04 POWER AXP318, `dl-a7s-schematic/radxa_cubie_a7s_schematic_v1.10.pdf`):
  BLDO1 feeds VCC-PE, VCC-PK and VCC-MCSI; BLDO2 feeds VCC-PD, VCC-LVDS0, VCC-PJ and VCC18-LCD. The sheet gives no voltage on
  the BLDO outputs, so the voltages come from the dts.
- **Partial, and flagged:** VCC-PH, VCC18-PF, VCC-PL, VCC-IO and VCC-PG have no regulator node and are not mapped on sheet 4.
  They are not in `bank_voltages`. The board is un-refused on the banks it states; the rest are open (see Open).

**radxa-zero-3e: stated (the io-domain table; U-Boot v2026.10 and the kernel-side dts).**
- Source: `rk3566-radxa-zero-3.dtsi:456-467`, `&pmu_io_domains` (status okay), supplies from the regulator nodes in the same file:
  pmuio1 `vcc3v3_pmu` 3.3 V (:349), pmuio2 `vcca1v8_pmu` 1.8 V (:287), vccio1 `vccio_acodec` 3.3 V (:325), vccio2 `vcc_1v8` 1.8 V
  (:38), vccio3 `vccio_sd` 1.8 V to 3.3 V (:337), vccio4 `vcc_1v8` 1.8 V, vccio5 to vccio7 `vcc_3v3` 3.3 V (:68).
- The driver is on in U-Boot proper (`configs/radxa-zero-3-rk3566_defconfig:7`, `CONFIG_ROCKCHIP_RK3568=y`), and not in SPL.
- `vccio3` is a range. `switched_by` names the SD card voltage regulator (`vccio_sd`, dts min 1800000 max 3300000), which
  switches the SD bank between 1.8V and 3.3V at run time.

**radxa-rock-5c: not applied (`applies_pad_table = false`), both sides.**
- U-Boot (v2026.10): `configs/rock-5c-rk3588s_defconfig` sets `CONFIG_ROCKCHIP_RK3588=y` (line 7) and neither
  `CONFIG_ROCKCHIP_IODOMAIN` nor `CONFIG_SPL_ROCKCHIP_IODOMAIN`. `drivers/misc/Kconfig:114-121` does not default the io-domain
  driver on for RK3588. No `io-domains` node in `dts/upstream/src/arm64/rockchip/rk3588-base.dtsi`, `rk3588s.dtsi` or
  `rk3588s-rock-5c.dts`.
- Kernel: the kernel-side dts copy (`research/scratch-uboot-mainline/dts/upstream/src/arm64/rockchip/`) has no `io-domains`
  node in those three files. The kernel config is not on disk. The io-domain driver applies only from an io-domains node, so
  with no node no table is applied.
- **UNVERIFIED against mainline 7.3.4:** the kernel-side copy is not the 7.3.4 tag. kernel.org was unreachable from this
  session (raw 404, git 403). The statement says so.

**radxa-rock-4d: not applied (`applies_pad_table = false`), both sides.**
- U-Boot: `configs/rock-4d-rk3576_defconfig` sets `CONFIG_ROCKCHIP_RK3576=y` (line 9), with no io-domain config. Not in the
  `Kconfig:114-121` default list. No `io-domains` node in `rk3576.dtsi` or `rk3576-rock-4d.dts`.
- Kernel: the same three checks on the kernel-side copy give no io-domains node. Config not on disk. The same 7.3.4 caveat.

**radxa-cubie-a7a and radxa-cubie-a7z: derived from the A7S (`derived_from_board = "radxa-cubie-a7s"`, bank voltages empty).**
They have no products, so nothing is refused today. Any product on them is refused until its own provenance comes with PR #134.

**The bring-up copy (`devices/products/cubie-a7-bringup/board/`) was deleted with its product (see below).**

## Products on next: what the rule says

Tree run (pr/PADS, after the changes): zero refusals on the products that exist. The rule passes:
raze-base (bcm2712 reason), zero-3e-base and zero-3e-bringup (io-domain table, `vccio3` with `switched_by`), rock-5c-base and
rock-5c-bringup (the not-applied statement), rock-4d-base and rock-4d-bringup (the not-applied statement).

## Deleted in this PR

**cubie-a7-bringup** (`devices/products/cubie-a7-bringup/`): a copy of the A7S board data (its `board.toml` repeats the A7S's own
lines, and its board's `pads.toml` marked it derived from the A7S). The A7S board's own statement now stands on its own
provenance, so the copy is superseded. It was the only product on the sun60i family, and the A7S board has no product of its
own yet. Its components were used only by it and are removed with it (`cubie-a7s-extlinux`, `usb-gadget-family`). Live
references fixed: `devices/boards/radxa-cubie-a7s/board.toml` (the LED comment) and `devices/README.md` (the bench-key list).
Nothing in CI (`.github`) or `scripts/` named it.

## Kept, with the reason (the coordinator's test, applied)

The test was: is a bring-up product superseded by a base or bench product on the same board? The base product of each
board is the managed image without the bench key. A base product does not replace a bench product, because it has no SSH
access for the bench. Deleting these three would remove the only bench access to each board, and the bring-up rule's own
self-test copies them as templates (`scripts/check-devices.sh`, the bring-up rule self-test, which mutates
`zero-3e-bringup`, `rock-5c-bringup`). So they are kept. **This is the one decision for the coordinator:** delete them, and the
bring-up rule's self-test needs a fixture in their place.

- zero-3e-bringup: kept (the only bench product of ZERO 3E).
- rock-5c-bringup: kept (the only bench product of ROCK 5C).
- rock-4d-bringup: kept (the only bench product of ROCK 4D).

## What stays refused

- **radxa-cubie-a7a and radxa-cubie-a7z** (boards with no products): stated as derived from the A7S. Any product on them is
  refused until their own provenance comes with PR #134.
- **radxa-cubie-a7s**: un-refused on the banks it states. The five PIO banks VCC-PH, VCC18-PF, VCC-PL, VCC-IO and VCC-PG are
  not stated. A product on the A7S that needs one of them is refused on the day the rule is taught per-bank completeness.

## Open

1. **A7S partial coverage:** VCC-PH, VCC18-PF, VCC-PL, VCC-IO and VCC-PG. Needs the sheet-7 (A733 GPIO) bank labels or another
   rail source before they can be stated.
2. **The bring-up products** (see Kept): delete or keep, and if deleted, the bring-up rule's self-test needs a new fixture.
3. **The RK kernel statements** are checked against the kernel-side dts copy, not the 7.3.4 tag. Re-check against 7.3.4 when
   it is reachable.
4. **ZERO 3E `vccio3`:** accepted as a switched range with `switched_by`, per the ruling. Recorded here so it is visible.

## Test fixtures

The four good fixtures (`devices/tests/fixtures/good`, `good-f8`, `good-rust`, `good-sd-image`) have one family each
(`example-family`). Each states `applies_pad_table = false` with a fixture reason, so the fixture checks keep passing.

## Not in this PR

- No K230 part (K1 keeps it). No board is built, flashed or booted.
- The product and component deletion is the only removal (cubie-a7-bringup).
