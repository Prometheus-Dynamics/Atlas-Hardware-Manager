# A7AZ: the Cubie A7A and A7Z bench images (A733, sun60i), slot form

**Group:** C (bench images). **Base:** C1 (`pr/C1`, 38a1f624). **Status:** DRAFT. Candidate, UNTESTED, NEVER BOOTED. Bench images only: the closed DRAM blob (`ddr.bin`, family LICENCE) is in the bootloader; nothing is published.

**Runs:** nowhere in this PR. No image is built (Gaia builds). No board is reached.

## What is here
- `devices/products/cubie-a7a-bench/`, `devices/products/cubie-a7z-bench/`: C1's product form (slot-disk, the Lemnos set at Lemnos pr/BB1b e8472620, bench-network, bench-pc-ssh-key, lemnos0-addressing, bench-hostname). Copies of C1's product with the names changed; `gaia.lock` written by `gaia lock` at Gaia c0d084f.
- `devices/boards/radxa-cubie-a7a/`: `board.toml` (kernel dts `allwinner/sun60i-a733-cubie-a7a`, U-Boot defconfig `radxa-cubie-a7a_defconfig` from radxa/u-boot 9da4b3e8), `lemnos-board.toml`, `patches/linux/0001-...-a7a.patch` (kernel dts + Makefile line).
- `devices/boards/radxa-cubie-a7z/`: `board.toml` (kernel dts `allwinner/sun60i-a733-cubie-a7z`, U-Boot defconfig `radxa-cubie-a7z_defconfig` from this repo's patch), `lemnos-board.toml`, `patches/linux/0001-...-a7z.patch`, `patches/u-boot/0001-...-a7z.patch` (A7Z defconfig and dts; dry-run applies cleanly to radxa/u-boot 9da4b3e8).

## Per item: proven / unverified
| Item | Status | Source |
|---|---|---|
| Product form, `gaia validate` (4 files) | PROVEN at Gaia c0d084f: `ok` for both products and both boards | this run |
| `gaia lock` (both products) | PROVEN: lock written | this run |
| `gaia plan` (both products) | PROVEN at c0d084fa (bin-c0d084f): exit 0 with `--lock`, `--recipes` (c0d084f src/recipes), `--tool python=`, `--input pc-key=<comment-stripped scratch copy>`, and the `slot_settings_args` settings. The earlier "no recipe slot-disk" was the call without `--recipes` | this run |
| A733 kernel dts, A7A | UNVERIFIED: derived from the DockSeed A7S dts (series fe0a8ed, `a733-dts`, `a733-ramoops-a7s`) and Radxa's A7A U-Boot dts (9da4b3e8) | `devices/boards/radxa-cubie-a7a/patches/linux` |
| A733 kernel dts, A7Z | UNVERIFIED: no public A7Z description; it is the A7A dts with the names changed | same |
| A7A U-Boot defconfig | UNVERIFIED: `radxa-cubie-a7a_defconfig` in radxa/u-boot 9da4b3e8 (exists) | research/radxa-u-boot |
| A7Z U-Boot defconfig and dts | UNVERIFIED, DERIVED FROM THE A7A (copy with the names changed); Radxa publishes none at 9da4b3e8 (both raw URLs 404; the A7A raw file loads). `patch --dry-run` clean | `devices/boards/radxa-cubie-a7z/patches/u-boot` |
| Kernel Makefile hunk (dtb line) | UNVERIFIED: the context lines come from the series' A7S patch; no kernel tree is on disk, so the hunk is not applied | `patches/linux` |
| Ports (A7A) | per Radxa (see VBUS) | `hardware-use/usb-type-c` |
| Ports (A7Z) | Radxa's type-c page: "USB-C 2.0 OTG & Power", 5 V input, device by default (overlay for host); user ruling: port 1 carries power+OTG+FEL | https://docs.radxa.com/en/cubie/a7z/hardware-use/type-c |
| Pins uart0 (A7A) | UNVERIFIED: PB9/PB10 are the A7S's pins; the A7A U-Boot dts names `uart0_pb_pins` | series a733-dts / radxa U-Boot |
| LEDs | MISSING (both): no source gives the GPIO | - |
| USB-C J4-type typec (A7S only) | NOT on A7A/A7Z: not in the A7A U-Boot dts, not known for the A7Z | - |
| eMMC (A7A, mmc2) | UNVERIFIED: enabled in the A7A U-Boot dts (8-bit, HS200); its vmmc/vqmmc supplies are NOT written (PMIC not in this dts) | radxa U-Boot a7a dts |
| A7Z eMMC | UNKNOWN | - |
| Ramoops | UNVERIFIED: the A7S series' values (reg 0x60000000 0x100000, console 0x40000, record 0x20000, ecc 16) | `a733-ramoops-a7s` |
| Watchdog | UNVERIFIED: `r_wdt` (C1 sets it in the product; lemnosd owns it) | C1 |
| PSTORE set to y | in the family fragment (C1) | families/sun60i/assets/linux.fragment |
| Lemnos gadget | UNVERIFIED on board: `udc:4100000.usb`, the same MUSB port form as the A7S | C1 / Lemnos pr/BB1b |

## VBUS (the gadget port)
The coordinator's ruling (2026-11-14): if Radxa's own page labels the gadget port as the 5 V power input, `vbus_undriven = true` with its label cited, the A7S schematic V1.10 sheet 13 as the family precedent, bench only. Otherwise stop and report.

- **A7A: `vbus_undriven = true`.** Radxa's page https://docs.radxa.com/en/cubie/a7a/hardware-use/usb-type-c labels the one USB-C port: "Supports 5V power input" and "Supports USB OTG (USB0 port)". Radxa's A7A U-Boot dts (radxa/u-boot 9da4b3e8, `arch/arm/dts/sun60i-a733-radxa-a7a.dts`) gives USB0 VBUS the fixed 5 V rail (`usb0_vbus-supply = <&reg_vcc5v>`), not a GPIO-switched one; the switched PL2 rail is on USB1. The A7A is not host-capable with a switched VBUS on that port, so no stop.
- **A7Z: `vbus_undriven = true` (coordinator ruling A7Z (a), 2026-11-14).** Port 1 is Radxa's "USB-C 2.0 OTG & Power", "Supports 5V power input", device mode by default (https://docs.radxa.com/en/cubie/a7z/hardware-use/type-c). Family pattern, Radxa Cubie A7S schematic V1.10: sheet 13 (J16 VBUS = VCC5V0, only into the input of U12), sheet 12 (USB0-DRVVBUS enables U2, SGM2576, for J4 = USBC2 VBUS), sheet 7 (PL2 = USB0-DRVVBUS). The switched port is USBC2 (Armbian armbian/build 00c27198, patch/kernel/archive/sun60iw2-opi-vendor/0005-cubie-a7z-add-device-tree.patch:56-62, usbc2-vbus on PL2). **A7Z schematic not read.** Bench only, powered from port 1.
  Earlier this PR stopped on that Armbian line and then the coordinator ruled (a). The URLs tried for a Radxa A7Z dts are in the table above.

## PL2: the host port's VBUS (per board, corrected ruling)
- **No PL2 hog on either board** (coordinator, corrected: a permanent output-low hog would kill the host port that USB cameras and other devices need). The earlier hog commit is reverted in this branch.
- **PL2 is the USB0-DRVVBUS pin, the VBUS switch of the usb1 host port.** The A7S schematic V1.10 names it: sheet 7 (PL2 = USB0-DRVVBUS), sheet 12 (USB0-DRVVBUS enables U2, SGM2576, for J4 = the host port's VBUS). The same dts wiring is in Radxa's A7A and A7S U-Boot dts and the A7S series.
- **A7A** (Radxa's file, unmodified in source): `sun60i-a733-radxa-a7a.dts` (radxa/u-boot 9da4b3e8) line 31-38 `reg_usb_vbus` (fixed, PL2, `gpio = <&r_pio 0 2 GPIO_ACTIVE_HIGH>`), line 71 `usb1_vbus-supply = <&reg_usb_vbus>`. No `regulator-always-on` and no `regulator-boot-on` in that node. Consumer: the usb1 USBPHY supply (only the host controller asks for it).
- **A7S** (for the family pattern): Radxa's `sun60i-a733-cubie-a7s.dts` at 9da4b3e8, line 37 (same gpio) and line 85 (`usb1_vbus-supply`); DockSeed's series names the regulator `reg_usbc_vbus` (A7S `a733-a7s-usbc`, PL2, consumer the HUSB311 Type-C controller, no always-on, no boot-on). Both A7S sources are C1's; this PR does not change the A7S.
- **A7Z**: the regulator is Armbian `reg_usbc2_vbus` (`usbc2-vbus`, PL2, `armbian/build` 00c27198 `0005-cubie-a7z-add-device-tree.patch:57-64`: no always-on, no boot-on, no vin supply), consumer `usb1_vbus-supply` (in the U-Boot dts this PR adds). Enabled only by the host controller.
- Flags check: none of the three PL2 regulators (A7A, A7S series, A7Z) has `regulator-always-on` or `regulator-boot-on`. Nothing was removed.
- Per board, who holds PL2 at each step: BootROM/FEL, nothing of ours; SPL, UNVERIFIED (the SPL uses the same board dt, and no PL2 regulator is always-on or boot-on, so nothing turns PL2 on from the SPL that the dts asks for); U-Boot, PL2 driven only when the usb1 host controller enables its supply; kernel, no PL2 user in either kernel dts (the bench kernels do not enable ehci1/ohci1 nor usb1 vbus); lemnosd, nothing.
- **Host port (usb1) VBUS:** the PL2 regulator (A7A `reg_usb_vbus`, A7Z `reg_usbc2_vbus`) is off at boot, with no regulator-always-on and no regulator-boot-on. It is enabled when the usb1 host controller asks for its VBUS supply, so the host port's VBUS works through that regulator. This PR does not enable the host controllers in the kernel dts (`&ehci1`/`&ohci1` are not set on either board), so a host port needs that kernel step before it is used; that is an open item, not a claim that the host port is proven.
- **Port 1 (the gadget port)**: `vbus_undriven = true` in both boards' lemnos-board.toml, undriven (no VBUS regulator on USB0). No host overlay for port 1 is shipped.

Radxa schematic URLs (NOT downloaded, for the lead to approve): A7A V1.10 `https://dl.radxa.com/cubie/a7a/docs/hw/radxa_cubie_a7a_v1.10_schematic.pdf`; A7Z V1.10 `https://dl.radxa.com/cubie/a7z/docs/hw/radxa_Cubie_A7Z_v1100__schematic.pdf`, V1.11 `https://dl.radxa.com/cubie/a7z/docs/hw/radxa_cubie_a7z_schematic_v1.11.pdf`, V2.00 `https://dl.radxa.com/cubie/a7z/docs/hw/radxa_cubie_a7z_schematic_v2.00.pdf`.

## Ethernet, per board (mainline gap)
- **A7A:** present in Radxa's U-Boot (gmac0, RGMII, PHY reset PH16). In the kernel: the GMAC210 driver (`dwmac-sun55i`, its gmac210 match) is NOT in mainline 7.3.4 (family.toml, B19); it comes from the DockSeed series (`a733-gmac210`, `a733-gmac-a7s`). **Missing in mainline; present in the series.**
- **A7Z:** UNKNOWN (no Radxa description). Same SoC gmac path assumed, unverified. **Missing in mainline (assumed); present in the series only if the A7Z uses gmac0.**
- Per the user's correction, Ethernet stays in the bench image (driver, DHCP, mDNS) plus the USB gadget. Ethernet untested on the bench.

## The missing list
- **Both:** a VBUS line for the A7Z; the A7Z U-Boot and kernel dts from a schematic (the lead's approval for the A7Z and A7A schematic URLs above); LED GPIOs; PMIC (AXP318) supplies in the kernel dts; eMMC supplies; UFS (A7A U-Boot enables it; not in the kernel dts); the A7Z device-port VBUS statement; `boot` and `data` partition sizes (placeholders, as C1); the Lemnos board file validation (`lemnos-ctl validate`, not run here).
- **Gaia:** `gaia plan` for both products is blocked by `no recipe "slot-disk"` (shared with C1). The kernel dts Makefile hunk is not applied (no kernel tree on disk).
- **Pins:** Lemnos pr/BB1b e8472620 and Gaia c0d084f are kept (not moved). Merge waits for the merged pins.

## Not done (and why)
- No schematic downloaded (the lead decides).
- No image built; no board touched.
- `devices/gaia.lock` and the root `Cargo.toml` not changed.
