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

**radxa-cubie-a7s: REFUSED (the bank list is not complete; see the update below).**
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
raze-base (bcm2712 reason), zero-3e-base and zero-3e-bringup (io-domain table, `vccio3` with `switched_by`), rock-5c-base (deleted in R1, #131) and
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


## Update: the bank-list rule and the kernel-side check (2026-11-16 rulings)

**Every bank is named.** A family's `pads.toml` lists the SoC's full bank list (`[banks.<soc>] names = "..."`), and a board names
its `soc`. The board's `bank_voltages` must name every bank of that list exactly. A missing bank and an unknown bank are both
refusals. Each bank's value is either a voltage (`1.8V`, `3.3V`, or a range with `switched_by`) or
`fixed by hardware, per sheet N`. Any other value is refused. The refusal names the board and the bank.
- sun60i (A733): `PA PB PC PD PE PF PG PH PI PJ PK PL PM`. The main pin controller has 11 banks from A, and the R-domain
  controller has 2 from L (research/radxa-u-boot/drivers/pinctrl/sunxi/pinctrl-sunxi.c, `sun60i_a733_pinctrl_desc` and
  `sun60i_a733_r_pinctrl_desc`).
- rk35xx (RK3566): `pmuio1 pmuio2 vccio1 ... vccio7` (the io domains of `rk3566-radxa-zero-3.dtsi:456-467`). RK3588 and RK3576
  boards state `applies_pad_table = false`, so they need no list.

**radxa-cubie-a7s: REFUSED.** The board names PC, PD, PE, PJ, PK and PM, from the dts regulators and the sheet-4 nets:
PE and PK on BLDO1 at 1.8V, PD and PJ on BLDO2 at 1.8V, PC on CLDO5 at 1.8V, PM on CLDO1 at 1.8V. The rest are not
established: **PA, PB, PI** (no rail label found on sheets 4, 7 or 8 in this session), and **PF, PG, PH, PL** (their rail
labels are on sheets 4 and 7: VCC18-PF, VCC-PG, VCC-PH, VCC-PL, but no regulator node gives their voltage, and sheet 4 puts
VCC-PL on ALDO1, which the dts names `vcc-mipi-dsi` at 1.8V, a conflict this session did not resolve). A probe product on the
A7S refuses on exactly PA, PB, PF, PG, PH, PI and PL. The A7S has no product in this PR, so nothing builds. No A7S image may
build until those seven banks are established.

**The kernel side is checked against fragments.** For a board stating `applies_pad_table = false`, the family's `pads.toml`
names a `kernel_option`, and the rule refuses the statement if any kernel fragment of the family or the board sets
`CONFIG_<kernel_option>=y`. rk35xx names `ROCKCHIP_IODOMAIN`. The resolved `.config` exists only after Gaia's build, so
**at check time the rule reads fragments, not the resolved config**. The U-Boot side has no resolved config at check time
either (the U-Boot defconfig is not in the repository); it is checked only through the built configs on disk, not the rule.

**RK3588 evidence, corrected.** `research/rock5c.config:1443` and `research/rock5c-fam.config:1452` both say
`# CONFIG_ROCKCHIP_IODOMAIN is not set`. Their headers say `U-Boot 2026.10 Configuration` and they carry 277 `CONFIG_SPL`
entries, so they are **U-Boot** configs, not the kernel 7.3.3 config the ruling assumed. They verify the U-Boot side of the
statement for that build. The kernel side stays unverified: no kernel config for ROCK 5C is on disk. The rock-5c `pads.toml`
cites the U-Boot config and says so.

**RK3576 (ROCK 4D):** no built config exists in `research/` (only a7s, rock5c and zero3), so both sides stay unverified.

**Tracked lines, so the bring-up products are not forgotten:**
- zero-3e-bringup: deleted in Z2 (#125), with its fixture moved there.
- rock-5c-bringup: deleted in R1 (#131).
- rock-4d-bringup: stays until a ROCK 4D bench product exists.
The same lines go into the Z2 and R1 PRs.

**Rulings not yet made (with the coordinator):** the remaining bring-up products (above), the A7S's seven unstated banks (the
A7S stays refused), and the K230 bootargs override (the K1 recorded-edits list is one entry).

## Update 2026-11-16 (second round): the A7S banks from the DockSeed series, and the kernel side by node

Supersedes the A7S bank mapping above where they differ. Earlier I had PD on BLDO2 and PM on CLDO1; the series corrects both.

**The sources.** S = the DockSeed A7S series at `/mnt/ssd-samsung/gaia/bsp/a733/a7s-linux-mainline/patches` (rev fe0a8ed), which
supplies the per-bank `vcc-p?-supply` properties and the regulator nodes. R = the Radxa U-Boot dts at 9da4b3e8 (the regulator
voltages). Sch = schematic V1.10 sheet 4 net names. Every bank is cited in `devices/boards/radxa-cubie-a7s/pads.toml` with
file:line. **Any disagreement leaves the bank refused until ruled**, and no source is picked silently.

| Bank | S (series, file:line) | R (Radxa dts) | Result |
|---|---|---|---|
| PA | none | none | **refused**: no source names a supply |
| PB | `vcc-pb-supply = reg_swout2` (bank-supplies patch :36); swout2 "vcc-io", no voltage in S (axp318w patch :68-71) | swout2 3.3V (dts :165-169) | **3.3V** |
| PC | `vcc-pc-supply = reg_cldo5` (bank-supplies :37); cldo5 1.8V (bank-supplies-2 :80-82) | cldo5 "vcc-pc" 1.8V (dts :240-244) | **1.8V**, agree |
| PD | `vcc-pd-supply = reg_swout1` (bank-supplies :38); swout1 "vcc33-lcd", no voltage in S (axp318w :59-62) | bldo2 named "vcc-pd" 1.8V (dts :185-190, name :189) | **refused: DISAGREEMENT**, S puts PD on SWOUT1 (3.3V), R names BLDO2 at 1.8V. S's own text: "the VCC-PD ball is connected to VCC33-LCD through R251, not to BLDO2, whose output also carries a VCC-PD label" (bank-supplies patch, the commit message) |
| PE | `vcc-pe-supply = reg_bldo1` (bank-supplies-2 :46); bldo1 1.8V (bank-supplies-2 :102-105) | bldo1 "vcc-pe" 1.8V (dts :178-183) | **1.8V**, agree |
| PF | `vcc-pf-supply = reg_swout2`, "or VCC18-PF, see power-source" (bank-supplies-2 :47) | none | **1.8V to 3.3V, switched by the pin controller's power-source select** (reset: 3.3V VCC-IO) |
| PG | `vcc-pg-supply = reg_aldo1` (describe-aldo1-as :44); aldo1 "vcc-pl" **3.3V** (describe-aldo1-as :61-64) | aldo1 "vcc-mipi-dsi" **1.8V** (dts :172-175) | **refused: DISAGREEMENT** on the ALDO1 voltage |
| PH | `vcc-ph-supply = reg_swout2` (bank-supplies :39) | swout2 3.3V (dts :165-169) | **3.3V** |
| PI | none | none | **refused**: no source names a supply |
| PJ | `vcc-pj-supply = reg_swout1` (bank-supplies-2 :50), "Bank PJ runs from VCC33-LCD (SWOUT1) with no series part" | sheet 4 puts VCC-PJ on BLDO2 (1.8V) | **refused: DISAGREEMENT**, SWOUT1 3.3V (S) against BLDO2 1.8V (Sch) |
| PK | `vcc-pk-supply = reg_bldo1` (bank-supplies-2 :51) | sheet 4 BLDO1 feeds VCC-PK; bldo1 1.8V (dts :178-183) | **1.8V**, agree |
| PL | none on r_pio (bank-supplies-2 :117, "Bank PL has no vcc-pl-supply on r_pio"); its rail is ALDO1 "vcc-pl" 3.3V (describe-aldo1-as :61-64) | aldo1 "vcc-mipi-dsi" 1.8V (dts :172-175) | **refused: DISAGREEMENT** on the ALDO1 voltage |
| PM | r_pio `vcc-pm-supply = reg_aldo1` (bank-supplies-2 :118); S: "the net named VCC-PM on CLDO1 does not reach the SoC" | cldo1 "vcc-pm" 1.8V (dts :218-222, name :221) | **refused: DISAGREEMENT**, ALDO1 3.3V (S) against CLDO1 1.8V (R) |

**The VCC-PL / ALDO1 / vcc-mipi-dsi question (the first candidate).** It is one node, the AXP318's ALDO1. The Radxa dts calls it
`vcc-mipi-dsi`, 1.8V (dts :172-175). The series calls it `vcc-pl`, **3.3V** (describe-aldo1-as :61-64). The series' voltage is
the one with the series' reasoning behind it (the PMIC bus runs on bank PL), but it is a disagreement, and the ruling decides the
voltage. Until then PG, PL and PM stay refused. **Finding for the coordinator, with both lines:** ALDO1 is 1.8V in R (dts :172-175)
and 3.3V in S (describe-aldo1-as :63-64).

**The A7S probe refuses exactly on PA, PD, PG, PI, PJ, PL and PM.** No A7S product exists in this PR, so nothing builds.

**The kernel side is checked by node.** A board stating `applies_pad_table = false` is refused if a dts the repository ships for
the board or the family (their `assets/`) enables an io-domains node (a node definition not marked `status = "disabled"` within
eight lines). The rock boards ship no such node, so they pass. The in-tree dts the image builds from is not in the repository,
so **the built dtb must be checked by Gaia's built-image check** (to be asked of Gaia). The fragment check (the family's
`kernel_option` unset in the fragments) stays as a secondary line only. It is not the verification.

**U-Boot side:** a resolved U-Boot config is not available at check time (the U-Boot defconfig is not in the repository), so the
U-Boot side is checked by the built-image check too. The built U-Boot configs on disk (`research/rock5c.config`) show the option is
unset for that build; that is evidence for that build, not a check of the image.
