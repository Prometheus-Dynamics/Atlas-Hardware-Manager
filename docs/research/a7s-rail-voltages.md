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
