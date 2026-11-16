# Radxa Cubie A7S: the PMIC rails behind the GPIO banks (A7RAILS)

Facts and citations only. Nothing here was measured; no board was touched. Where a source is silent, the text says so.
"Most likely" statements are marked and are not relied on by any pad statement.

## 1. Sources

| Id | Source | Revision and location |
|---|---|---|
| S | DockSeed A7S mainline series, the kernel patches | fe0a8ed, `/mnt/ssd-samsung/gaia/bsp/a733/a7s-linux-mainline/patches`, in `patches/series` order |
| S-BC | DockSeed U-Boot patches (our bootloader's extra patches) | a733-bootchain b6f6872b, `patches/u-boot/series` (18 files) |
| RU | Radxa U-Boot dts, the tree our bootloader builds from | radxa/u-boot 9da4b3e8, `research/radxa-u-boot/arch/arm/dts/sun60i-a733-cubie-a7s.dts` (278 lines) |
| RK6 | Radxa vendor kernel board dts, Linux 6.6 | radxa/allwinner-device 7b63c9ec, `configs/cubie_a7s/linux-6.6/board.dts` |
| RK5 | same repo, Linux 5.15 | `configs/cubie_a7s/linux-5.15/board.dts` |
| UBV | same repo, the vendor U-Boot board dts | `configs/cubie_a7s/uboot-board.dts` (a different file from RU) |
| SCH | Radxa Cubie A7S schematic V1.10 | `dl-a7s-schematic/radxa_cubie_a7s_schematic_v1.10.pdf` (text layer `a7s.txt`); sheets 4, 7, 9 |
| DRV-V | Vendor (Radxa 6.1 tree) pinctrl core, the nearest on-disk copy of the sunxi pinctrl core | `/mnt/ssd-samsung/gaia/bsp/radxa-kernel-6.1/drivers/pinctrl/sunxi/pinctrl-sunxi.c` |
| UCL | U-Boot regulator uclass and AXP regulator driver | `research/radxa-u-boot/drivers/power/regulator/regulator-uclass.c`, `axp_regulator.c` |
| CORE | Regulator core (vendor 6.1) | `/mnt/ssd-samsung/gaia/bsp/radxa-kernel-6.1/drivers/regulator/core.c`, `of_regulator.c` |

The mainline 7.3.4 sunxi pinctrl core that the series patches is NOT on disk (kernel.org was not reachable; no downloads were made).
DRV-V is an older vendor copy. Where this doc cites DRV-V it says so.

The correcting patch is applied after S. Its base is S rebuilt from the series: the 40 patches that touch
`sun60i-a733-cubie-a7s.dts` applied in series order (the post-series dts, 680 lines), and the 6 patches that touch
`pinctrl-sun60i-a733.c` (57 lines). The patch applies with `patch -p1 --fuzz=0` to both (kernel/a7s-rails.patch).

## 2. Which bootloader our A7S image runs, and what it programs

The family (`devices/families/sun60i/family.toml`) builds U-Boot from RU 9da4b3e8 with the 18 S-BC patches. The 18
patches touch regulators only in `pcie1v8-supply = <&reg_bldo1>; pcie3v3-supply = <&reg_dcdc1>;` (the same as RU).
The bootchain's PCIe patch says PD20 enables an adapter's regulator (its commit text, "measured on the board" is DockSeed's
claim, not ours).

What the U-Boot regulator code does (UCL):

- A regulator is probed at bind only when it has `regulator-always-on` or `regulator-boot-on` (`regulator-uclass.c:456-460`).
- At probe, `regulator_post_probe` calls `regulator_autoset`. With no always-on or boot-on it returns `-EMEDIUMTYPE` and
  nothing is written (`regulator-uclass.c:322-324`).
- A consumer that probes a regulator (a `vmmc-supply` or similar) also runs `regulator_autoset` for it.
- The voltage is written only when min equals max (`REGULATOR_FLAG_AUTOSET_UV`, `regulator-uclass.c:503-506`). A range
  is only enabled, never set.
- The AXP driver's `set_value` writes the PMIC register (`axp_regulator.c`; the AXP318 table is at lines 198-225).

So, for the RU A7S dts:

| Rail (RU node) | RU value | RU flags | Programmed by RU? |
|---|---|---|---|
| bldo2 "vcc-pd" (dts:185-190) | 1.8 V fixed | always-on | YES, at probe, to 1.8 V |
| bldo1 "vcc-pe" (dts:178-183) | 1.8 V fixed | always-on | YES, at probe, to 1.8 V |
| swout1 "vdd-swout1" (dts:158-163) | 3.3 V fixed | always-on | YES, asked for 3.3 V (the AXP318 switch's own ops are not checked here) |
| swout2 "vdd-swout2" (dts:165-170) | 3.3 V fixed | always-on | YES, asked for 3.3 V (same caveat) |
| cldo5 "vcc-pc" (dts:240-244) | 1.8 V fixed | none | YES, when its consumer probes: the eMMC `vmmc-supply` (dts:73) |
| aldo1 "vcc-mipi-dsi" (dts:172-176) | 1.8 V fixed | none | NO: no flag and no consumer in RU. The 1.8 V is declared, not applied |
| cldo1 "vcc-pm" (dts:218-222) | 1.8 V fixed | none | NO: no flag and no consumer |
| dcdc1 "vdd-3v3" (dts:151-156) | range 1.0-3.4 V | always-on | NO: a range is only enabled |

The answer to the brief's question: RU's U-Boot programs BLDO2 (and BLDO1, CLDO5 through its consumer). It does NOT program
ALDO1. The 1.8 V on ALDO1 in RU is a declaration, not an action.

Bank mode (`GPIO_POW_MOD_SEL`): `arch/arm/mach-sunxi/board.c:81` `gpio_init()` runs in SPL (`board.c:485`). For
`CONFIG_SUNXI_GEN_NCAT2` (selected by `MACH_SUN60I_A733`, Kconfig:567-571) it copies `POW_MOD_VAL` into `POW_MOD_SEL`
(`board.c:204-207`). So the SoC's own detected value is what the bootloader leaves in every bank. The register offsets
used are the H6 macros (`SUN50I_H6_GPIO_POW_MOD_*`); whether they are the A733's offsets is UNVERIFIED (no A733 manual on
disk). The series' commit text says the reset value is 0x022aaaa0 (3.3 V for every bank) when a boot chain does not touch
the register.

## 3. The kernel rails: the series, Radxa's kernel, Radxa's U-Boot

Series (S, post-series dts) against RK6 and RU. "Fixed" means min equals max.

| Rail | S (post-series) | RK6 (7b63c9ec, kernel) | RU (9da4b3e8, U-Boot) | Schematic (SCH) | Bank(s) it feeds |
|---|---|---|---|---|---|
| aldo1 | "vcc-pl" 3.3 V fixed, always-on (`describe-aldo1-as.patch:61-66`) | range 0.5-3.4 V, AOBO (1369-1375) | 1.8 V fixed, no flags (172-176) | ALDO1 to VCC-PL (sheet 4); PL ball 1M23; PG via RA3; PM via RA13 (sheet 7) | PG, PL, PM |
| dcdc1 | "vcc-3v3-sys" 3.3 V fixed, always-on (`add-the-axp318w-pmic.patch:47-50`) | range 1.0-3.8 V, AOBO (1280-1288) | range 1.0-3.4 V, always-on (151-156) | DCDC1 feeds SW1/SW2 to SWOUT1/2 (sheet 4); no voltage annotated | (input rail) |
| bldo1 | "vcc-pe-pk-mcsi" 1.8 V fixed, always-on (`remaining-pin-bank-supplies.patch:102-105`) | range 0.5-3.4 V, no flags (1407-1412) | 1.8 V fixed, always-on (178-183) | BLDO1 to VCC-PE, VCC-PK, VCC-MCSI (sheet 4); no voltage annotated | PE, PK |
| bldo2 | no node in S | range 0.5-3.4 V, no flags (1413-1416) | 1.8 V fixed, always-on (185-190) | BLDO2 to VCC-PD, VCC-LVDS0, VCC-PJ, VCC18-LCD (sheet 4) | none in S (S moves PD to SWOUT1) |
| cldo1 | no node in S | range 0.5-3.5 V, AOBO (1437-1443) | 1.8 V fixed, no flags (218-222) | net label VCC-PM (sheet 4), 1.8 V annotation | none in S (r_pio vcc-pm is aldo1) |
| cldo5 | "vcc-pc" 1.8 V fixed, always-on (`supply-the-emmc-from-the-pmic.patch:61-62`) | range 0.5-3.4 V, AOBO (1465-1471) | 1.8 V fixed, no flags (240-244) | CLDO5 to VCC-PC and VCC18-PF (sheet 4, 1.8 V annotation); eMMC VCCQ1-5 on VCC-PC (sheet 9) | PC; PF (VCC18-PF) |
| swout1 | no min/max; always-on | dc1sw1 switch, no value | 3.3 V fixed, always-on (158-163) | VCC33-LCD (sheet 4); R251 to VCC-PD (sheet 7); PHY AVCC (sheet 15) | PD (per S), PJ (per S) |
| swout2 | no min/max; always-on | dc1sw2 switch, no value | 3.3 V fixed, always-on (165-170) | VCC-IO, VCC-PH, VCC-CARD, VCC-eMMC (sheets 4, 9, 13) | PB, PH, PF (VCC-IO) |
| dcdc4 (GPU) | "vdd-gpu" 0.96 V fixed, always-on (`enable-the-gpu.patch:68-`) | range 0.5-1.54 V, AOBO (1307-1313); gpu-supply = dcdc4 (1957) | no node | not a bank rail | none |
| dcdc3 | range 0.8-1.05 V, always-on | range 0.5-1.54 V, AOBO (1298-1306) | range 0.9-1.0 V, always-on (201) | CPU | none |
| dcdc5 | range 0.8-1.05 V, always-on | range 0.5-1.54 V, AOBO (1316-1324) | range 0.8-1.0 V, always-on (211) | CPU | none |

Fixed-regulator nodes in S that are not PMIC rails are not changed: `vcc3v3-pcie` (3.3 V), `usb1-vbus`, `usbc-vbus`
(5 V), `vcc5v0-sys` (5 V), `ldo2v5-input` (2.4 V), `wifi-3v3`. They are fixed by a fixed-regulator driver, not by the PMIC.

### What the correcting patch changes (kernel/a7s-rails.patch; recorded in kernel/edits.toml)

| Edit | Rail | Change | Reason |
|---|---|---|---|
| E1 | aldo1 | 3.3 V fixed to range 0.5-3.4 V (the PMIC's table; RK6 1369), boot-on added, always-on kept | No maker file fixes it. RU declares 1.8 V but does not apply it (section 2) |
| E2 | dcdc1 | 3.3 V fixed to range 1.0-3.4 V (the intersection of RU 1.0-3.4 and RK6 1.0-3.8), boot-on added | No maker file fixes it |
| E3 | bldo1 | 1.8 V fixed to range 0.5-3.4 V, always-on kept | The schematic gives no voltage. RU applies 1.8 V at boot; the kernel must not re-apply it |
| E4 | dcdc4 (GPU) | 0.96 V fixed to range 0.5-1.54 V (RK6), boot-on added | No maker file fixes it. CONSEQUENCE, UNVERIFIED: the GPU runs at the PMIC's value until a GPU operating point sets one |
| E5 | pinctrl io_bias_banks | from PC, PD, PE, PG, PH, PJ, PK to PC and PH | Section 4 |
| E6 | cldo5 | kept fixed 1.8 V | The schematic proves it: sheet 4 annotates CLDO5 1.8 V for VCC-PC and sheet 9 ties eMMC VCCQ to VCC-PC; RU and RK6 agree on 1.8 V |
| E7 | swout1, swout2 | not changed (no value in S) | S gives none; RU asks for 3.3 V |

A range is not a no-op for the regulator core. `of_regulator.c:110-111` sets `apply_uV` whenever min and max are both
nonzero, and `core.c:1259-1290` moves the rail only when its current voltage is outside [min, max] (to min if below, to max if
above). So each E1-E4 range contains the AXP's full register table, and the core never writes a value it reads back. The
rail's value stays what the PMIC powers up to (or what U-Boot wrote, for bldo1).

## 4. The pinctrl driver: does it read the bank supply, and what does it write?

Answer, with the lines:

1. Yes. It reads the supply's voltage with `regulator_get_voltage()`. The vendor 6.1 core does this in
   `sunxi_pinctrl_set_io_bias_cfg` at `pinctrl-sunxi.c:691` (`uV = regulator_get_voltage(supply)`). The series' 2-bit
   variant (`pinctrl-sunxi-add-a-two-bit-pow-mod-sel-withstand-voltage-variant.patch`, the `BIAS_VOLTAGE_PIO_POW_MODE_SEL_2BIT`
   case) writes `val = uV <= 1800000 ? 3 : 2` into the bank's 2-bit field (11 = 1.8 V, 10 = 3.3 V) for banks in
   `io_bias_banks` only (its `if (!(pctl->desc->io_bias_banks & BIT(bank))) return 0;`).
2. When: at pin request, not at probe. The vendor 6.1 `sunxi_pmx_request` (`pinctrl-sunxi.c:838-871`) does
   `regulator_get("vcc-p%c")` (855), `regulator_enable` (861), then `sunxi_pinctrl_set_io_bias_cfg` (868). Its return value is
   ignored there.
3. What if the read fails or is zero: `uV < 0` returns the error and writes nothing (`pinctrl-sunxi.c:692-693`); `uV == 0`
   ("dummy regulator") returns 0 and writes nothing (`:695-697`).
4. Guess? No. The driver has no default voltage. It has no write from a constant. Its only input is the read-back.
5. The damaging direction (1.8 V mode written while the pad's rail is 3.3 V) is possible only if the read-back says
   1.8 V (or less) for a rail that is really 3.3 V. The read-back is the AXP register, which the regulator core reads
   from hardware. So the driver is only as right as the rail's declared and programmed value.

Per bank, after E5, for the banks the 2-bit variant writes:

| Bank | In io_bias_banks (after E5)? | Supply (S) | Read-back source | Mode the driver can write | Damaging case? |
|---|---|---|---|---|---|
| PC | yes (BIT 2) | cldo5 | fixed 1.8 V, programmed by the kernel at registration (E6) | 11 (1.8 V) | No: the kernel itself sets 1.8 V before any pin is requested. Schematic annotation and eMMC VCCQ agree. The series tested 11 on the A7S |
| PH | yes (BIT 7) | swout2 | switch, no fixed value; the AXP318W driver's `get_voltage` for SWOUT2 is UNVERIFIED | 10 (3.3 V) or no write | No: the only write is 3.3 V mode |
| PD | no (E5) | swout1 per S | n/a | none; mode stays as SPL sets it | Not written. PD stays UNSTATED (section 5) |
| PE | no (E5) | bldo1 | RU-set 1.8 V (E3) | none | Not written |
| PG | no (E5) | aldo1 | PMIC power-up value (E1) | none | Not written |
| PJ | no (E5) | swout1 per S | n/a | none | Not written. PJ stays UNSTATED |
| PK | no (E5) | bldo1 | RU-set 1.8 V (E3) | none | Not written |
| PB | no (never in the set) | swout2 | n/a | none | Not written |
| PL, PM | no (R domain; the A733 r-pinctrl has its own description with no withstand variant (series `pinctrl-sunxi-a733-r-leave-the-i-o-withstand-voltage-to-the-hardware.patch`)) | aldo1 | n/a | none | Not written |
| PA, PI | no | none | n/a | none | Not written |
| PF | separate path (`pf_power_source`) | vcc-pf: swout2 or VCC18-PF (cldo5) | the power-source bit | the series' PF patch writes 3.3 V mode at init and 1.8 V mode only when the bank is switched to a supply that reads at or below 1.8 V (`pinctrl-sunxi-a733-set-the-withstand-mode-of-bank-pf-with-its-supply.patch`, `sunxi_pf_withstand_1v8`) | Only when the SD driver has switched PF to CLDO5 (1.8 V, fixed by E6). Unchanged by this PR |

A bank out of the set keeps the mode the SoC (SPL) set: the detected value (section 2). Its statement is therefore "the
PMIC's rail, not programmed by the bootloader or kernel" (where true), not "the kernel sets it".

## 5. Bank statements (the pads.toml, devices/boards/radxa-cubie-a7s/pads.toml)

Forms: a voltage (fixed, with its source), a range with `switched_by`, "fixed by hardware, per sheet N", "set by the bootloader
... not re-programmed by the kernel" (the rail is one the bootloader programs and the kernel no longer writes), "left as the PMIC
powers it up; not programmed by our bootloader or kernel; source: <file> line N" (the rail is one nothing programs), and
"no pads: ... 0 pins" (the SoC's own table gives the bank no pins). The two rail forms are accepted only where the correcting
patch makes the named rail a range (the checker reads the patch's post-image: `pads_rail_state` in scripts/check-devices.sh).

| Bank | Statement | Basis |
|---|---|---|
| PA | no pads: 0 pins | S `pinctrl-sun60i-a733.c` `a733_nr_bank_pins` first entry = 0 |
| PB | 3.3 V | SWOUT2 (SCH sheet 4 VCC-IO); RU swout2 3.3 V fixed, always-on |
| PC | 1.8 V | CLDO5 (SCH sheet 4 annotation; sheet 9 eMMC VCCQ); RU cldo5 1.8 V fixed; kernel keeps it fixed (E6); the driver writes 11 |
| PD | UNSTATED, refused | Conflict: S says VCC-PD is SWOUT1 (3.3 V, R251); RU and SCH sheet 4 put BLDO2 (1.8 V) on a net labelled VCC-PD. Not settled |
| PE | set by the bootloader to 1.8 V (RU bldo1, dts:178-183), not re-programmed by the kernel | RU programs it at boot (section 2); S agrees on 1.8 V; SCH silent on the value |
| PF | a range, 1.8 V to 3.3 V, switched by the PF power-source bit | S `sunxi-pinctrl-power-source` patch; SCH sheet 7 has both balls (1U5 VCC-IO, 1U6 VCC18-PF) |
| PG | left as the PMIC powers it up (RK6 aldo1 line 1369) | Rail ALDO1 (SCH sheet 7 net VCC-PL via RA3). RU does not program ALDO1; E1 makes it a range |
| PH | 3.3 V | SWOUT2 (SCH sheet 4 VCC-PH; sheet 7 ball); RU swout2 3.3 V |
| PI | UNSTATED, refused | No supply in any source. S gives PI 17 pins and says the PI field is reserved; no PI pad name is in the schematic text layer |
| PJ | UNSTATED, refused | Conflict: S says SWOUT1 (3.3 V); SCH sheet 4 puts VCC-PJ on BLDO2 (1.8 V); sheet 7 ball is VCC33-LCD |
| PK | set by the bootloader to 1.8 V (RU bldo1), not re-programmed by the kernel | As PE |
| PL | left as the PMIC powers it up (RK6 aldo1 line 1369) | Rail ALDO1 (SCH sheet 7 ball 1M23 VCC-PL). S has no vcc-pl-supply on r_pio |
| PM | left as the PMIC powers it up (RK6 aldo1 line 1369) | ALDO1 via RA13 (SCH sheet 7) or CLDO1 (SCH sheet 4 label). RU does not program CLDO1 either (section 2) |

The checker, run over these files as a mirror product (the real A7S has no product, so `pads_rule` skips it on the real tree):
it refuses PD, PI and PJ and passes every other bank. Making ALDO1 a fixed value in the patch makes it refuse PG, PL and PM
("still pins it to a fixed voltage").

## 6. Naming and file discrepancies (facts)

- **PMIC name.** SCH sheet 4: "AXP318-WBBGA". RK6, RK5, RU and UBV: `x-powers,axp8191` at I2C 0x36 (`axp8191-*` regulator
  names). S: `compatible = "x-powers,axp318w"` (`add-the-axp318w-pmic.patch`). Whether axp8191 and AXP318W are one register
  map is NOT verified from disk.
- **eMMC supply, RU.** RU dts:73-74: `vmmc-supply = <&reg_cldo5>; vqmmc-supply = <&reg_swout2>;`, with `mmc-hs200-1_8v` on the
  same node. Swapped against RK6 (`vmmc = dc1sw2`, `vqmmc18sw = cldo5`, lines 911-914) and S (`vmmc = swout2`,
  `vqmmc = cldo5`). The RU node is the one our bootloader builds from. Its effect: U-Boot probes CLDO5 (1.8 V) as the eMMC
  VCC and SWOUT2 (3.3 V) as VCCQ, the reverse of the schematic's VCC-eMMC from SWOUT2 (sheet 4, sheet 9).
- **PCIe supplies differ in three files:**

  | File | pcie1v8-supply | pcie3v3-supply | Lines |
  |---|---|---|---|
  | RU (9da4b3e8, our bootloader) | bldo1 | dcdc1 | 276-277 |
  | UBV (vendor U-Boot, 7b63c9ec) | bldo2 | dc1sw1 | 824-825 |
  | RK6 (vendor kernel 6.6, 7b63c9ec) | dc1sw1 | bldo2 | 1084-1085 |
  | RK5 (vendor kernel 5.15, 7b63c9ec) | dc1sw1 | bldo2 | 1097-1098 |

  The kernel files agree with each other; UBV is swapped against them, and RU is different again.
- **BLDO2 flags.** The earlier analysis said RK6 gives BLDO2 always-on and boot-on. The current read says no flags on
  RK6 bldo2 (lines 1413-1416). Corrected here.
- **Two U-Boot dts files.** RU (9da4b3e8) and UBV (7b63c9ec) are different files with different PCIe and regulator
  content. Our image uses RU.
- **Regulator ranges, RK6 vs RU.** RK6 gives ranges for most rails; RU gives fixed values for ALDO1, BLDO1, BLDO2, CLDO1
  and CLDO5 and ranges for DCDC1.

## 7. A7A and A7Z (for PR #134; not edited here)

The A7A and A7Z vendor files (7b63c9ec) have no bank supply properties (`vcc-p*-supply`). Their PMIC rails, from
`configs/cubie_a7a|a7z/linux-6.6/board.dts` (line numbers from the same parser):

| Rail | A7A RK6 | A7Z RK6 | RU A7A/A7Z (uboot-board.dts) |
|---|---|---|---|
| dcdc1 | range 1.0-3.8, AOBO (1257) | range 1.0-3.8, AOBO (1255) | range, no value (333) |
| dcdc2-4 | range 0.5-1.54, AOBO | range 0.5-1.54, AOBO | no value |
| aldo1 | range 0.5-3.4, AOBO (1346) | range 0.5-3.4, AOBO (1344) | no value (363) |
| bldo1 | range 0.5-3.4, no flags (1384) | range 0.5-3.4, no flags (1382) | no value |
| bldo2 | range 0.5-3.4, no flags (1390) | range 0.5-3.4, no flags (1388) | no value |
| cldo1 | range 0.5-3.5, AOBO (1414) | range 0.5-3.5, AOBO (1412) | no value |
| cldo5 | range 0.5-3.4, AOBO (1442) | range 0.5-3.4, AOBO (1440) | no value |

In their RK6 files: `vmmc = dc1sw2`, `vqmmc18sw = cldo5`, `vqmmc33sw = dc1sw2` (A7A 920-923; A7Z 953-956). PCIe: `pcie1v8 = bldo1`,
`pcie3v3 = dcdc1` (A7A 1042-1043; A7Z 1081-1082). That is the same as RU's A7S file and different from the A7S RK6 file.

What the A7A/A7Z PR needs: none of their kernel rails is fixed in the vendor files, so applying E1-E4 to their dts would
change only the series' values (if their series dts pins them; PR #134 is not read here). Their `pads.toml` stays "derived
from the A7S" and is refused, as before: a bank must be stated in the new form from their own Radxa files, and the
correcting patch for them is not in this PR.

## 8. The schematic table (most likely, NOT relied on)

From `research/a7s-bank-voltages.md` (the earlier analysis, same schematic). Carried here as the record of what each rail
most likely is. No statement in pads.toml depends on it alone.

| Bank | Most likely supply | Most likely voltage | Why it is not relied on |
|---|---|---|---|
| PA | none found | n/a | No pads (section 5) |
| PB | VCC-IO (SWOUT2) | 3.3 V | Net name only |
| PC | CLDO5 | 1.8 V | Annotation and eMMC wiring; not measured |
| PD | SWOUT1 (R251) and/or BLDO2 (label) | 3.3 V or 1.8 V | Two drivers under one name |
| PE | BLDO1 | 1.8 V (RU) | No annotation |
| PF | VCC-IO or VCC18-PF | 3.3 V reset, 1.8 V when switched | Software-selected |
| PG | ALDO1 via VCC-PL | unknown | RU does not program ALDO1 |
| PH | SWOUT2 | 3.3 V | Net name only |
| PI | none found | unknown | No PI pad in the text layer |
| PJ | SWOUT1 or BLDO2 | 3.3 V or 1.8 V | Conflict |
| PK | BLDO1 | 1.8 V (RU) | No annotation |
| PL | ALDO1 | unknown (RU declares 1.8 V, S 3.3 V) | RU does not program it |
| PM | ALDO1 (RA13) or CLDO1 (label) | unknown | Conflict |

## 9. For Gaia's built-image check (not added to board.toml)

Gaia will add `[checks] unprogrammed_rails = [...]` to the board description, to refuse a built image whose listed regulators
still have min equal to max. The pinned Gaia rejects an unknown key, so the key is NOT added here. The list for this patch:

```
unprogrammed_rails = ["aldo1", "bldo1", "dcdc1", "dcdc4"]
```

These are the regulator node names under the PMIC in the built A7S dtb (`axp318` node). cldo5 stays fixed and is not listed.
The same four nodes would apply to A7A/A7Z if the same correction is applied to their dts (not in this PR). The list also
lives in kernel/edits.toml, marked "to be added when Gaia's rev with the key is pinned".

## 10. Not established

- Every voltage on every rail. Nothing was measured.
- PD, PI, PJ (refused in pads.toml).
- The rail behind PG, PL and PM (ALDO1 or CLDO1, and its power-up value).
- Whether the AXP318W driver's SWOUT1/SWOUT2 `get_voltage` returns a value (affects the PH read-back only).
- The A733 `POW_MOD_SEL` / `POW_MOD_VAL` register offsets used by RU's SPL (H6 macros; UNVERIFIED for A733).
- The mainline 7.3.4 sunxi pinctrl core (not on disk; DRV-V is an older copy).
- The GPU's voltage after E4 (depends on an operating-point table, not in this dts).
- Whether the series' eMMC and PCIe supplies match the schematic (section 6 lists the disagreements).
- The eMMC part's VCC and VCCQ ranges (KLM8G1GETF-B041; datasheet not on disk).

## 11. Required first-boot step (the record)

The first boot of an A7x image (A7S, A7A, A7Z) reads, read-only, and records:

1. The regulator summary and every `/sys/class/regulator/*/microvolts` and `/sys/class/regulator/*/state`, to the boot log.
2. The pin bank modes: the `POW_MOD_SEL` register (the base of the r-pio and pio controllers, read with devmem or the
   `pinctrl-sunxi` debugfs), and the `vcc-p*` supply of each bank, with its read-back.

The pad statements are then updated from the record as "measured on board, our image <sha>, <date>". That update is the only
way a `left as the PMIC powers it up` statement becomes a voltage. No stock-image boot or meter is asked of the user, because
our kernel does not write a rail that the driver reads as unprogrammed. This is recorded in devices/boards/radxa-cubie-a7s/board.toml
(comment only) and in this section.

## 12. What the patch does not do

- It does not touch PF's switch or the SD driver.
- It does not change any bank's voltage in the bootloader.
- It does not add the `[checks]` key (section 9).
- It does not touch pr/A7AZ or the A7A/A7Z dts.

## 13. Coordinator rulings of 2026-11-16: the lift basis, GPU, PD/PI/PJ/PA, A7A and A7Z

### 13.1 The driver reading, from the kernel the gate builds

Two trees, both read from disk:

- **The tree this doc was verified on:** linux 7.3.3, pristine from `/mnt/ssd-samsung/gaia/store-next/buildroot-downloads/linux/linux-7.3.3.tar.xz`
  (sha256 `79c66dde66cd96877fcde8f7d36aec946b1f423d2b97b5a7b1751b75c764f3d1`, verified). The DockSeed series (fe0a8ed, the
  rev the family pins) applied with `patch -p1 --fuzz=0` in `patches/series` order: 379 of 379 applied. Then
  `kernel/a7s-rails.patch` applied with `--fuzz=0`.
- **The image's kernel base:** `devices/families/sun60i/family.toml` pins `linux` at `7595a62d` (v7.3.4), not 7.3.3. The gate's
  kernel-fragments step (`scripts/check-kernel-fragments.sh:290-322`) applies the series to that pinned tree. So the image
  builds from **v7.3.4 plus the series**. To check the difference, the pristine `drivers/pinctrl/sunxi/pinctrl-sunxi.c`
  and `pinctrl-sunxi.h` of 7.3.3 and of the gate's cached 7.3.4 (`~/.cache/atlas/kernel/7552930bd7aabcd5/src`) were compared:
  byte-identical. The sunxi pinctrl hunks of the series, applied in series order to the 7.3.4 files, give files identical to
  the 7.3.3 result, and so does the A733 driver with our correction. Nothing differs on the four points below.

The four points, from the 7.3.3 + series + correction tree (`drivers/pinctrl/sunxi/pinctrl-sunxi.c` unless marked):

| Point | Citation | What the line does |
|---|---|---|
| (a) reads the bank supply's voltage | `:865` `uV = regulator_get_voltage(supply);` | the supply's read-back |
| (b) writes 1.8 V only at or below 1.8 V | `:929-930` `/* 0b11: 1.8V, 0b10: 3.3V */` `val = uV <= 1800000 ? 3 : 2;` | 0b11 (1.8 V) only when the read-back is at or below 1.8 V; otherwise 0b10 (3.3 V) |
| (c) writes nothing on error or zero | `:866-867` `if (uV < 0) return uV;`, `:870-871` `if (uV == 0) return 0;` | both return before any write; the caller at `:1083` ignores the return value |
| (d) writes only for io_bias_banks | `:925-927` `case ..._2BIT:` `if (!(pctl->desc->io_bias_banks & BIT(bank))) return 0;`; the set is `pinctrl-sun60i-a733.c:36` `.io_bias_banks = BIT(2) | BIT(7),` | only PC and PH |

The 6.1 vendor copy (section 4) has the same structure for each point. Mainline differs on none of the four, so the lift
condition holds. The series' only other pinctrl writer for the A7S is the PF path (`pf_power_source`), unchanged by this PR.

Caveat, stated: the gate's kernel-fragments step applies the series to a copy of the tree, but the correcting patch is not
applied by any step (board kernel patches are not wired yet). So the image's build does not yet carry the correction.

### 13.2 The GPU (dcdc4): which case, and what was done

Case 1 applies. Radxa's kernel dts gives `reg_dcdc4` as a range (`configs/cubie_a7s/linux-6.6/board.dts:1307-1313`, 500000 to
1540000, boot-on and always-on) with the consumer `&gpu { gpu-supply = <&reg_dcdc4>; }` (lines 1956-1957). Radxa's SoC dtsi
with a GPU OPP table is not on disk, so Radxa's OPP table is UNVERIFIED.

Our kernel has the same consumer (the series dts, `&gpu`, `gpu-supply`, line 347, status okay), and the OPP table exists in
our SoC dtsi: `sun60i-a733.dtsi:379-385` `gpu_opp_table` with `opp-1008000000 { opp-microvolt = <960000 960000 960000>; }`,
and the GPU node has `operating-points-v2 = <&gpu_opp_table>` (line 437). So the GPU driver scales VDD-GPU to 960 mV through
the OPP when the GPU runs. That is the maker's form: a range, with the value set by the consumer's OPP.

Done: `dcdc4` is a range of 500000 to 1540000 (the Radxa range, edit E4), boot-on and always-on. It stays on the
`unprogrammed_rails` list (it must stay a range). GPU voltage scaling is enabled, through the OPP. The 960 mV is not a probe-time
value. Recorded in `kernel/edits.toml` E4.

### 13.3 PD, PJ, PI and PA: mechanical statements

- **PD and PJ** are stated as "left as the PMIC powers it up", the supply cited as BLDO2 or SWOUT1 with both sources (Radxa
  U-Boot dts:185-190; the series bank-supplies patch :38 and :50; schematic sheet 4 and sheet 7 R251). The correcting patch
  removes `vcc-pd-supply` and `vcc-pj-supply` from `&pio` (edit E8), so neither rail is named as the bank's supply in our dts.
  Neither rail is programmed by our kernel (no `bldo2` node; `swout1` has no min or max). The driver writes neither bank's mode
  (`io_bias_banks` excludes BIT(3) and BIT(9)), and with no supply the request path finds the dummy regulator, which reads 0 V
  and writes nothing (`drivers/regulator/core.c:2453-2458`, full constraints).
- **PI**: "left as the PMIC powers it up", on the mechanical condition that the built dts and the board's pin use name no pin
  of bank PI. In the built tree, `pio-ref PI 0`: no `<&pio 8 ...>` reference in `sun60i-a733-cubie-a7s.dts` or
  `sun60i-a733.dtsi`, and no `_pi_` pin group in either. The SoC pin table gives PI 17 pins, so a reference would be a real use.
- **PA**: "no pads", on the mechanical condition `bank-pins PA 0`: `pinctrl-sun60i-a733.c:18` `{ 0, 11, 17, 24, 16, 7, 15, 20, 17, 28, 26 }`, PA first (cited).

The checker reads these facts from `devices/boards/radxa-cubie-a7s/kernel/post-patch-facts.txt` (recorded from the tree
above, with the dts sha256). A statement whose fact disagrees is refused: a dts that muxes PI, or a supply on PD or PJ, or a
mode write, each refuses the image. Shown on a mirror: PI with `pio-ref PI 1` refuses, and PD with `pio-supply PD swout1`
refuses. The self-test covers each kind (`scripts/check-devices.sh`, `pads_self_test`).

### 13.4 A7A and A7Z (Radxa files only)

Both boards: `configs/cubie_a7a|a7z/linux-6.6/board.dts` and `uboot-board.dts`, from radxa/allwinner-device 7b63c9ec. Read facts:
no `vcc-p*-supply` property in either kernel dts; no regulator with min equal to max in either file (all 25 PMIC nodes are
ranges, and the bootloader file has no values). Their pin use is by pinctrl group name, and the Radxa SoC dtsi that names the
groups is not on disk.

- Edits: none (`devices/boards/radxa-cubie-a7a/kernel/edits.toml`, `...a7z...`). The series does not apply to these boards in
  this repo; the pr/A7AZ dts (#134) was not read, and it must be checked against the facts file.
- Statements (their pads.toml): PA "no pads" (zero-pins); PB..PE, PG, PH, PJ..PM "left as the PMIC powers it up" with the
  unsupplied check (no supply, no mode write); PF a switched range 1.8V to 3.3V with `switched_by` (the SD driver's IO-voltage
  switch, `vqmmc18sw = cldo5` and `vqmmc33sw = dc1sw2`, lines 920-923 / 953-956). PI is REFUSED: its pin groups are not checkable
  without the SoC dtsi. So both A7A and A7Z pads.toml still refuse, on PI only.

### 13.5 What is still open

- The PI and PD/PJ facts are recorded, not regenerated by the check: the check reads the recorded file and cannot re-derive it
  without the series tree. The facts file names the tree and the sha256 of the dts it was read from.
- PG, PL and PM (the rail behind ALDO1 or CLDO1) are unchanged from section 5.
- The 7.3.4 full-series build is not run: only the sunxi pinctrl hunks were applied to the 7.3.4 files (identical result).

### 13.6 The two written banks, read back: PH (SWOUT2) and PC (CLDO5)

The driver writes a mode only for PC and PH after this PR (section 13.1). Each one's `regulator_get_voltage()` result is read
from the code, not assumed. Sources: the series' AXP318W patch (`x-powers-axp318w/regulator-axp20x-add-support-for-the-axp318w.patch`),
the base `drivers/regulator/axp20x-regulator.c` and `drivers/regulator/core.c` of linux 7.3.3 (tarball sha256 above), and our
post-patch dts (line numbers from the post-series dts).

**PH: bank PH's supply is `vcc-ph-supply = <&reg_swout2>` (dts:438). SWOUT2 is a switch.**

- *Its node, our dts* (dts:514-517): `reg_swout2: swout2 { regulator-name = "vcc-io"; regulator-always-on; };`. No min,
  no max, no voltage property. Not a fixed value (case 3 does not apply).
- *Its descriptor, the series* (patch:294-295): `AXP_DESC_SW(AXP318, SWOUT2, "swout2", NULL, AXP318_DCDC_OUTPUT_CONTROL2, BIT(4))`.
  The `supply` argument is NULL. The macro (base :394-405) gives it `.ops = &axp20x_ops_sw` (base :593-597), which has only
  `enable`, `disable`, `is_enabled`. No `get_voltage`, no `get_voltage_sel`, no `list_voltage`, and `fixed_uV` is 0.
- *The probe* (base :1652-1662, extended by the series at patch:447-448): a switch descriptor gets
  `new_desc->supply_name = dcdc1_name`, the DCDC1 rail's `regulator-name`, which is `vcc-3v3-sys` (dts:486).
- *The core* (`core.c`, `regulator_get_voltage_rdev`): with no `get_voltage_sel` (:4811), no `get_voltage`, no `list_voltage`
  and no `fixed_uV`, it reaches `else if (rdev->supply)` (:4822) and returns `regulator_get_voltage_rdev(rdev->supply->rdev)`
  (:4823). That is the parent. The `uV_offset` of SWOUT2 is 0.

So the case is **(1), the parent rail's read-back**: the parent is DCDC1 (`vcc-3v3-sys`), and its value is read from its own
AXP register (DCDC1 has `axp20x_ops` with `get_voltage_sel`, descriptor patch:257). The parent is not fixed by us: our dts
gives DCDC1 a range of 1.0 V to 3.4 V (dts:486-489 after E2), and neither Radxa file programs it (section 2). So DCDC1's
read-back is the PMIC's power-up value, which is the value the switch passes on.

- If DCDC1 is not yet registered when the pin is requested, the core returns `-EPROBE_DEFER` (`core.c:4824-4826`): case (2), an
  error, and nothing is written (`pinctrl-sunxi.c:866-867`).
- The result is 10 (3.3 V mode) when DCDC1 reads above 1.8 V, and 11 (1.8 V mode) when it reads 1.8 V or less.

**Can PH be the 1.8 V mode on a 3.3 V rail?** Only if DCDC1's register and its output disagreed, because PH's output is DCDC1's
output (SWOUT2 is always on, with no enable of ours to change it). A 1.8 V read-back is a 1.8 V parent, so the bank mode and the
rail agree. The mode can therefore follow DCDC1's power-up value, not a 3.3 V assumption, but it cannot be 1.8 V mode on a 3.3 V
rail unless the PMIC contradicts its own register. That is the same residual risk every read-back in this PR carries, and the
first-boot record (section 11) measures DCDC1's register and output. **Result: no stop.** PH's mode is set by DCDC1's read-back,
which is the rail PH runs from.

**PC: `vcc-pc-supply = <&reg_cldo5>` (dts:433). CLDO5 is a fixed 1.8 V in our dts.**

- *Its node* (dts, post-series, `reg_cldo5`): `regulator-min-microvolt = <1800000>; regulator-max-microvolt = <1800000>;`,
  always-on. That is case (3), a fixed value from the node's properties, and the kernel keeps it fixed (edit E6).
- *Its descriptor* (patch, `AXP_DESC_DO(AXP318, CLDO5, "cldo5", ...)`): an LDO with `axp20x_ops` (base :583-591), so
  `get_voltage_sel` reads the register. The step is 100 mV from 0.5 V, so 1.8 V is an exact register value.
- *Applied at registration*: the core applies a constraint with min equal to max (`apply_uV`, `core.c:1233-1234`). With the
  current voltage outside it, the core moves the rail to the bound (`core.c:1346-1351`). So after registration CLDO5 reads
  1.8 V whatever the power-up value was.
- *The read-back* is therefore 1.8 V, and the mode written is 11 (1.8 V), which matches the rail. If the registration step
  failed, the regulator would not register and the request would return an error (case 2): nothing written.

**Result for both written banks:** PC reads its own fixed 1.8 V, written 11, which is the 1.8 V rail the schematic and the
eMMC wiring support. PH reads DCDC1's register through the switch, written 10 or 11 to match that parent, never 1.8 V mode on a
3.3 V output. No stop is needed.

### 13.7 The correction is wired into the build; the plan shows it after the series

Gaia at a6299c62 takes a board's `[kernel] patches` in place of the family's (`crates/core/describe/src/board.rs`: `patches: b.patches.or(f.patches)`),
and applies them in list order (`crates/core/graph/src/os.rs`, `locked_source`: a `Local` file or a directory, or an `InSource` path).
The family declares no series on this branch, so the series is wired on the board: `devices/boards/radxa-cubie-a7s/board.toml`:

    patches = [{ source = "a733-kernel-series", path = "patches" }, { file = "kernel/a7s-rails.patch" }]

Proof: a scratch probe product (not committed; named cubie-a7s-bench to take C1's lock at origin/pr/C1) with the A7S board and
C1's gaia.lock. `gaia validate` passes on the board and the product. `gaia plan` (a6299c62) gives the kernel as
`linux.patched` built from `patches0` (the series, `radxa-cubie-a7s.a733-kernel-series`) and then `patch1` (our file,
`boards/radxa-cubie-a7s/kernel/a7s-rails.patch`):

```
  cubie-a7s-bench.os.radxa-cubie-a7s.linux.patched  gaia.source.patch      products/a7s-probe/product.toml
  radxa-cubie-a7s.a733-kernel-series                gaia.source.git        boards/radxa-cubie-a7s/board.toml
  cubie-a7s-bench.os.radxa-cubie-a7s.linux.patch1   gaia.source.input      products/a7s-probe/product.toml
  radxa-cubie-a7s.a733-kernel-series.tree -> cubie-a7s-bench.os.radxa-cubie-a7s.linux.patched.patches0
  cubie-a7s-bench.os.radxa-cubie-a7s.linux.patch1.tree -> cubie-a7s-bench.os.radxa-cubie-a7s.linux.patched.patches1
```

and in the plan's JSON the patched node takes `["patches", "a7s-rails.patch"]` in that order.

**The gap is closed by three checks:**
- `scripts/check-devices.sh` (pads rule): a pads.toml that names a correction must find `{ file = "<it>" }` in its board's or
  family's `[kernel] patches`, and the file must exist. Otherwise the statement cites a patch the build does not include, and the
  image is refused. The self-test has a case for it (`railunwired`).
- `scripts/check-kernel-fragments.sh`: applies the series, then the board's own patches (`--fuzz=0`, a reject fails the gate) to
  the pinned 7.3.4 tree, then runs `scripts/check-board-facts.py` on the patched tree. Only boards with kernel patches take the
  facts step.
- `scripts/check-board-facts.py`: regenerates every fact from the patched tree (the io_bias_banks set, each bank's supply
  line, the PI references, the bank pin table, and that the unprogrammed regulators are ranges) and fails on any difference.
  Verified on the kept 7.3.4 patched tree: 53 recorded facts match. Shown negative: a PD supply added to the dts is refused
  (`pio-supply PD` and the dts sha256 both differ).

**The lift:** `check-kernel-fragments` ran clean at the head of this PR: 7 kernels, 0 skipped, the A7S row patched and its facts
matching. The correction is in the build's inputs and in Gaia's plan, after the series.

### 13.8 What would lift A7A and A7Z

Both stay refused on PI only. PI needs a mechanical check that the board's pin groups use no bank-PI pin. Their pin groups are named in
the Radxa SoC dtsi (`sun60i-a733.dtsi` of the Radxa tree), which is not on disk, so no `<&pio 8 ...>` reference can be read from their
dts. What would lift it: their dts on `pr/A7AZ` (#134), with the pinctrl groups resolved to their bank (from the Radxa or series dtsi, on
disk), and a `pio-ref PI 0` fact recorded and checked by `check-board-facts.py` against that tree. Until then, PI is refused.
