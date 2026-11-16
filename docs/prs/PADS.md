# PADS: the pad-table provenance rule

Split out of K1 (coordinator ruling, 2026-11-16): the rule is general, so it lands on its own. K1 keeps the K230 parts and
rebases on this PR afterwards. Nothing here changes a board's build, boot or image; it refuses a product whose pad table
has no provenance.

## Why

A board's bootloader can write the SoC's iomux table, including the bank-voltage mode. A wrong bank voltage can damage the
chip (the K230 BPI table says so at its dts line 65). A table copied from another board, or one with no source, or one with
the bank voltages left unstated, must not build.

## The rule

1. A family states, in `devices/families/<f>/pads.toml`, whether its bootloader applies a pad table:
   - `applies_pad_table = true`: every board of the family states its own `pads.toml`.
   - `applies_pad_table = false` with `pad_table_reason = "..."`: no table applies; the reason is required.
   - Neither (no file, or no key): the family is refused. An omitted statement is the mistake the rule stops.
2. A board of a flagged family must have `devices/boards/<b>/pads.toml` (a product-local board such as
   `devices/products/<p>/board/` takes its own `pads.toml` beside its `board.toml`). Its keys:
   - `source`: where the table came from, with the path and the sheet or lines.
   - `derived_from_board`: empty means the board's own table. A board id means it was copied from that board: refused.
   - `bank_voltages`: every bank the table drives, stated. Empty is refused.
3. The rule runs over every product's board. A board with no product is not built, so it is not refused here.

Refusal messages name the board and the reason, and the self-test proves each one plus the passes:
the pad rule passes a sourced, stated table and a family that says `false` with a reason, and refuses a derived, unsourced,
unstated or missing table, and a family that is unstated or says `false` with no reason.

**Deviation from the brief, and why:** the family and board statements are sidecar files (`pads.toml`), not keys in
`family.toml` or `board.toml`. Gaia reads those description files with `deny_unknown_fields`: the probe of
`applies_pad_table` under `[bootloader]` returned `families/rk35xx/family.toml:38: bootloader.applies_pad_table: unknown key`
(gaia 2.0.6-dev at d8441c9). A `[pads]` table in a board file would be refused the same way. `pads.toml` is not a
description, so Gaia never reads it, and the rule reads it directly.

## Families on next

| Family | Statement | Basis |
|---|---|---|
| rk35xx | `applies_pad_table = true` | U-Boot v2026.10 applies the io-domain table at boot (`CONFIG_ROCKCHIP_IODOMAIN` defaults on for RK3568, `research/u-boot/drivers/misc/Kconfig:114-121`); not in SPL (`CONFIG_SPL_ROCKCHIP_IODOMAIN` is off, `Kconfig:127`). |
| sun60i | `applies_pad_table = true` | The A733 boot chain reads the board's PIO setup from its device tree. Each board states its table. |
| bcm2712 | `applies_pad_table = false`, reason "pads are set by the Pi firmware; not applicable" | No Atlas bootloader; the Pi firmware and the kernel overlays set the pins. |

k230 is not on next, so its family statement stays in K1 with its pending draft.

## Boards on next, by source

**rk35xx (the io-domain table; U-Boot v2026.10 at `research/u-boot`, dts from `dts/upstream/src/arm64/rockchip/`):**
- **radxa-zero-3e: stated.** `rk3566-radxa-zero-3.dtsi:456-467` (`&pmu_io_domains`, status okay). Supplies, each voltage from
  its regulator node in the same file: pmuio1 `vcc3v3_pmu` 3.3 V (:349), pmuio2 `vcca1v8_pmu` 1.8 V (:287), vccio1
  `vccio_acodec` 3.3 V (:325), vccio2 `vcc_1v8` 1.8 V (:38), vccio3 `vccio_sd` 1.8 V to 3.3 V (:337), vccio4 `vcc_1v8` 1.8 V,
  vccio5 to vccio7 `vcc_3v3` 3.3 V (:68).
  - Driver: the ZERO 3E defconfig selects `CONFIG_ROCKCHIP_RK3568=y` (`configs/radxa-zero-3-rk3566_defconfig:7`), so the io-domain
    driver is on in U-Boot proper. It is not in SPL: the defconfig has no `CONFIG_SPL_ROCKCHIP_IODOMAIN`.
  - **Open:** vccio3 is a range, not a fixed bank voltage. It is stated as the range, and that is a judgment for the coordinator.
- **radxa-rock-5c: not established.** No `io-domains` node in `rk3588-base` or in `rk3588s-rock-5c.dts`; the io-domain driver
  is off for RK3588 (not in the `Kconfig:114-121` default list, and not in `rock-5c-rk3588s_defconfig`, which sets
  `CONFIG_ROCKCHIP_RK3588=y` at line 7). So no table is applied and no bank voltage is stated on disk. No ROCK 5C schematic is on disk.
- **radxa-rock-4d: not established.** Same reasoning for RK3576 (`rk3576-rock-4d.dts`, no io-domains node, RK3576 not in the
  default list).

**sun60i (Radxa Cubie A7S schematic V1.10, `dl-a7s-schematic/radxa_cubie_a7s_schematic_v1.10.pdf`, rendered at 300 dpi):**
- **radxa-cubie-a7s: not established.** Sheet 4 (04 POWER AXP318) gives the PIO supply nets: BLDO1 feeds VCC-PE, VCC-PK and
  VCC-MCSI; BLDO2 feeds VCC-PD, VCC-LVDS0, VCC-PJ and VCC18-LCD; BLDO4 feeds VCC18-CODEC. The BLDO outputs carry no voltage
  annotation on that sheet, and the text layer does not give one either. VCC-PH, VCC-PC, VCC-IO, VCC-PF, VCC-PL and VCC-PM are
  not mapped to a rail on sheet 4. The sheet-7 (A733 GPIO) bank labels were not read in this session. The board is left
  refused. It has no product, so the rule does not block a build; any product on it would be refused.
- **radxa-cubie-a7a, radxa-cubie-a7z: derived from the A7S.** `derived_from_board = "radxa-cubie-a7s"`, so any product on them
  is refused. Their real provenance comes with PR #134. They have no products, so nothing is refused today.

## Products on next: what the rule now says

Run: `pads_rule` over the tree (the function run; the full device check is in the gate log).

| Product | Result | Why |
|---|---|---|
| raze-base | passes | bcm2712: `applies_pad_table = false` with the reason. |
| zero-3e-base | passes | ZERO 3E table stated (vccio3 as a range: see Open). |
| zero-3e-bringup | passes | Same board, same table. |
| rock-5c-base | **refused** | radxa-rock-5c: bank voltages not stated (no io-domain table applied; no schematic on disk). |
| rock-5c-bringup | **refused** | Same board. |
| rock-4d-base | **refused** | radxa-rock-4d: bank voltages not stated (RK3576, no io-domain table; no schematic on disk). |
| rock-4d-bringup | **refused** | Same board. |
| cubie-a7-bringup | **refused** | Its board is a copy of the A7S: derived from `radxa-cubie-a7s`, and bank voltages not stated. |

The refusals are where the provenance really is missing. For the ROCK 5C and ROCK 4D, the table is applied by nothing on
disk and the bank voltages cannot be stated without a schematic. For the bring-up Cubie, the copied table is derived.

## Open (for the coordinator's ruling before the push)

1. **ZERO 3E vccio3:** stated as a range (1.8 V to 3.3 V, set at run time by the SD voltage switch). If that is not
   acceptable as a bank voltage, the ZERO 3E products (zero-3e-base, zero-3e-bringup) are refused too.
2. **A7S bank voltages:** need the sheet-7 bank labels, or the BLDO voltage values from another sheet or from the AXP318
   defaults. Until then the A7S, and any Cubie product on it, is refused.
3. **ROCK 5C and ROCK 4D:** need a schematic or the vendor board data for the bank voltages. Lemnos may find a public one.
4. **Sidecar placement:** confirm `pads.toml` beside the description file, as the way to keep the statement away from Gaia's
   parser.
5. **K1 rebase:** K1's pending family and board drafts state the pad table in the old form (`[bootloader]` key, `[pads]`
   table). They move to `pads.toml` when K1 rebases on this PR.

## Test fixtures

The four good fixtures (`devices/tests/fixtures/good`, `good-f8`, `good-rust`, `good-sd-image`) have one family each
(`example-family`). Under the rule each states `applies_pad_table = false` with a fixture reason, so the fixture checks
keep passing. Without that statement the rule refused them ("states no pad-table fact"), which the first self-test run showed.

## Not in this PR

- No family or board is changed except the new `pads.toml` files and the `pads` rule in `scripts/check-devices.sh`.
- No K230 part (K1 keeps it). No board is built, flashed or booted.
