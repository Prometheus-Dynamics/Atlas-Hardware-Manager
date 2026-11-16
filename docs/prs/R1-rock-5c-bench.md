# R1: the ROCK 5C bench image as a U-Boot slot image

Status: DRAFT. Code and descriptions only. Nothing built as an image, nothing flashed, no board access. Shipping waits on
the rk35xx family proof (Z2, `zero-3e-bench`) and on the Lemnos and Gaia merges listed below.

Built the way Z2 builds `zero-3e-bench`: `products/rock-5c-bench` (recipe `slot-disk`, `initramfs = ["lemnos-slot-init"]`),
the board's `lemnos-board.toml` in slot form, the shared bench components, and a board component `rock-5c-slot-board`.

## Pins (the next pins; PINS-2 comes later)

- Gaia: `9efa797f` (next's pin; `devices/gaia.lock`). Built and used: `/mnt/ssd-samsung/gaia/bin/next-9efa797f/gaia`.
- Lemnos: `31e0f736` (next's `lemnos-slot-store` pin, root `Cargo.toml`, BB1b slot declarations). Every Lemnos component of
  `rock-5c-bench` and the slot board is locked to it, so the one-rev rule passes. It replaces the earlier `7748a241` (Z2's pin).
- What R1 depends on in these pins (PINS-2 must need no rework here, or this is the list that changes):
  - `lemnos-board.toml` keys: `[[devices]] system-slots` with `record = "uboot"`, `uboot_env_size`, `uboot_env_mmc_index`,
    `boot_sources`; `[[hardware]] vbus = "gpio:gpio0:28"` with `vbus_off` and `vbus_reason`; `options = ["usb3"]`;
    `usb-gadget` config; `[security] orion = false`; `[descriptions] lemnos`.
  - `socs/rk3588.toml` names: controller `usb-otg0` (path `/usb@fc000000`, sysfs `fc000000.usb`) and watchdog
    `/watchdog@feaf0000`.
  - The slot-board component's `lemnos-ctl tree` / `boot-partition` / `bundle` / `data-partition` arguments and flags.
  - The packaging paths in the Lemnos components (`packaging/systemd/...`).
  - If PINS-2 renames or removes any of these, R1 changes in those files only (`lemnos-board.toml` and the slot-board component).
- Earlier pin record (for history): this branch used `7748a241` before the merge of next; that is no longer used.

- Lemnos: the product's components are locked at `7748a241` (pr/BB1b tip, the same pin as Z2's `zero-3e-bench`).
  NOTE: the brief named `e8472620` as "pr/BB1b, the same as Z2". At the Lemnos checkout on disk, `e8472620` is the commit
  "BB1: the A733 USB OTG controller is a device port", not the BB1b tip. Z2 uses `7748a241`, so R1 does too. Coordinator to confirm.
- Gaia: `c0d084fa` (the binary the brief names; `gaia --version` reports it). `devices/gaia.lock` is unchanged (G12 moves it).
- Root `Cargo.toml` Lemnos rev is still `f4a295c6` (Z2 STOP 1): the Lemnos one-rev rule refuses until the Cargo pin moves.

## Spec, item by item

| Item | What R1 does | Status |
|---|---|---|
| Board files from Gaia's next | `devices/boards/radxa-rock-5c` and `devices/families/rk35xx` are used as they are on next, with R1's additions (below) in the board's own files. No parallel copy. | Proven: `gaia validate` ok (board, family, product). The additions are Atlas edits to Gaia-owned files: Gaia to confirm ownership. |
| Partitions (u-boot 32K, uboot-env 16M/128K, boot, data) | Restated in `board.toml` as Z2 does: `u-boot` (32K, outside table), `uboot-env` (16M, 128K, type 3de21764...), `boot` (128 MiB, FAT), `data` (1 GiB, last). | Proven: validate and plan ok. Sizes UNVERIFIED on board; Gaia `boot-report` gives the real fit. |
| `lemnos-board.toml` in slot form | `devices/boards/radxa-rock-5c/lemnos-board.toml`: identity, watchdog, reboot, `system-slots` with `record = "uboot"`, `uboot_env_size = 0x10000`, `uboot_env_mmc_index = 1`, `boot_sources = ["/mmc@fe2c0000=/mmc@fe2c0000"]`, gadget, security off. The old `lemnos-board.pending.toml` is removed: its entry is now active. | Written; validated by plan (`rock-5c-slot-board` placement). Not run through `lemnos-ctl tree` (the generator runs in the image build). |
| Watchdog | `/watchdog@feaf0000` (RK3588 Synopsys DW, `rockchip,rk3588-wdt`). Lemnos `socs/rk3588.toml` at `7748a241` `[controllers.watchdog]`, read from Lemnos's v6.12 base. | DERIVED from the Lemnos map. Not seen on a board. |
| Gadget on DRD0 (upper USB 3.0 Type-A) | `udc:fc000000.usb`, `controller = "usb-otg0"`, `role = "peripheral"`, `options = ["usb3"]`. Lemnos `socs/rk3588.toml` `[controllers.usb-otg0]` (path `/usb@fc000000`, sysfs `fc000000.usb`). Kernel dts: `&usb_host0_xhci { dr_mode = "peripheral"; }` (the in-tree value is `"host"`, `rk3588s-rock-5c.dts` line 937). | `dr_mode` change: DERIVED from the in-tree dts. Gadget never enumerated on a board. |
| Gadget VBUS (the hold) | `vbus = "gpio:gpio0:28"`, `vbus_off = "low"`, `vbus_reason` cites the schematic. See "VBUS: the hold" below. | Citation: recorded reading (B21, 9174ecd6), PDF not re-read. Hold: UNVERIFIED (see below). |
| PSTORE, PSTORE_RAM, PSTORE_CONSOLE =y | Inherited from the rk35xx family fragment (`devices/families/rk35xx/assets/linux.fragment`, Z2). | Config lines present. `check-kernel-fragments` not run here. |
| ramoops reserved memory | `assets/dts/rockchip/rk3588s-rock-5c-ramoops.dtsi`: reg 0x110000 / 0xe0000, console 0x80000, pmsg 0x30000, ftrace 0, ecc 0, record 0x14000, from Radxa's RK3588 kernel (`gaia/bsp/radxa-kernel-6.1` `arch/arm64/boot/dts/rockchip/rk3588-linux.dtsi` lines 92-102, "0x110000 to 0x1f0000 is for ramoops"). Firefly's tree (4f6ee8c16, rk3588-linux.dtsi:85-87) uses 0xf0000; 0xe0000 fits both. boot-log-size/count left out (Rockchip vendor extensions; mainline ramoops has no such property). The next node, minidump-smem@1f0000, starts at the region's end, so the size does not grow. | Address and sizes cited from the vendor tree (the coordinator's ruling, 2026-11-14). Confirm with /proc/iomem at first boot. Not run on a board. |
| mDNS | Inherited from the zero copy: `systemd-resolved` on, `20-bench-mdns.network` on lemnos0. | Config present. Not run on a board. |
| Ethernet (DHCP) kept | `10-bench-ethernet.network` (DHCP, mDNS), per the coordinator's ruling. | Config present. The mainline Ethernet driver for the RK3588S GMAC: UNVERIFIED here (not checked in this PR). |
| Shared pieces | `bench-hostname` (shared, unchanged); prefix `rock5c` (`bench-hostname-prefix`); `lemnos0-addressing` (shared); the PC key component is `bench-pc-key` (the brief's "bench-pc-ssh-key" is this component, under its real name). | Proven: files present, `gaia plan` ok. |
| lemnosd owns the watchdog | `hw-watchdog` keepalive in the board file; the bench unit has no watchdog key (copied from Z2). | Config present. |
| RK env hard rule | Family `u-boot.fragment` sets `CONFIG_ENV_MMC_USE_DT=y`, `CONFIG_ENV_MMC_DEVICE_INDEX=1`, `CONFIG_ENV_SIZE=0x10000`. The rule (`scripts/check-devices.sh` `rk_env_rule`) refuses a slot product on an rk35xx board without them. | The check is run by the gate (see the gate result below). |
| SD index for U-Boot | SD = U-Boot `mmc 1`. Basis: the Radxa U-Boot copy on disk (`research/radxa-u-boot`): `rk3588s-rock-5c.dts` lines 20-21 `mmc0 = &sdhci` (eMMC), `mmc1 = &sdmmc` (microSD, `/mmc@fe2c0000`, `rk3588-base.dtsi:2200`). | UNVERIFIED for U-Boot v2026.10 (the copy is Radxa's tree). Matches the family's `ENV_MMC_DEVICE_INDEX=1`. |
| Bench key and PC key | `rock-5c-slot-board/files/slot-keys` is the bench's PUBLIC line, the same file Z2 carries. The PC key is a named input (`pc-key`); the plan uses a throwaway key stripped by `pc-key-strip.sh`. No private key or seed touched. | Proven: plan ok. |

## VBUS: the hold (coordinator ruling 2026-11-14)

Facts (the schematic reading recorded in B21, `docs/prs/B21.md` on origin/pr/A16 at 9174ecd6; the PDF was NOT re-read here):
- The upper USB 3.0 port's 5 V (`VCC5V0_USB_OTG0`, J4 VBUS pin 10) is the output of load switch U9 (SY6280AAC). Its EN (pin 4)
  comes from `USB_OTG_PWREN_H` = GPIO0_D4 (ball AL39, sheet 5) through R82 (1K), sheet 14.
- R83 (the EN pull-down) is NC: EN has no pull-down, so it floats until something drives PD4.
- Mainline/Radxa `rk3588s-rock-5c.dts`: `vcc5v0_usb_otg0` is `regulator-fixed`, `enable-active-high`, `gpio = <&gpio0 RK_PD4>`, not always-on,
  and `&u2phy0_otg { phy-supply = <&vcc5v0_usb_otg0>; }`.
- Sheet 23: the upper USB-C is a sink only, not on the OTG net.
- Line: PD4 = bank D (3) pin 4 = 3*8+4 = 28 on gpio0. `vbus_off = "low"` (enable active high).

What R1 puts in place:
1. U-Boot: `assets/patches/u-boot/0001-rock-5c-hold-usb-otg-vbus-low.patch` adds a `gpio-hog` (`output-low`, line 28) under `&gpio0`
   in `arch/arm/dts/rk3588s-rock-5c-u-boot.dtsi`, applied through `[bootloader] patches`. Owner from U-Boot until the kernel: the hog.
2. Kernel: the regulator `vcc5v0_usb_otg0` owns PD4 in Linux (no gpio-hog in the kernel dts: a hog on the same line would make the
   regulator's gpio request fail). The loader dts DELETES `phy-supply` on `&u2phy0_otg`, so the PHY never drives the 5 V in gadget
   mode. Whether the regulator core then drives the line low at late init (no consumer): UNVERIFIED on board.
3. Lemnos: the `vbus` entry names the line for Lemnos's own boot hold (Q2g, not merged). Q2g skips PD4 because the regulator
   `vcc5v0_usb_otg0` owns it (coordinator ruling 2026-11-14; Lemnos told): the U-Boot hog holds EN low first, then the regulator
   holds it low from its probe. Q2g must not add a second hog on that line.

Hold timeline (c): who holds EN at each step.
| Step | Who holds EN (PD4) | Status |
|---|---|---|
| 1. Power-on, BootROM (maskrom or boot) | Nothing of ours. R82 (1K) to the EN pin; R83 NC, so EN floats. The pad's reset state of GPIO0_D4 is not read (TRM not on disk). | Nothing of ours can hold it. UNVERIFIED pad reset state. |
| 2. SPL (idbloader, sector 64) | Nothing set by R1: the hog is in the main U-Boot dtsi, which SPL uses only if its dtb carries it (`u-boot,dm-spl`). | UNVERIFIED whether SPL applies the hog. Likely not. |
| 3. U-Boot proper | The hog: output-low on line 28, applied when the gpio0 driver binds. | UNVERIFIED that U-Boot applies gpio-hog children of a gpio controller at bind on v2026.10. |
| 4. Kernel probe to late init | The regulator (`regulator-fixed`): line left as found (ASIS), then the regulator core's unused-regulator cleanup at late init. The PHY has no `phy-supply`, so it does not enable VBUS. | Core behaviour UNVERIFIED on board. |
| 5. lemnosd running | Nothing new. Lemnos's Q2g boot hold: not merged. | Blocked on Lemnos Q2g. |

(a) During maskrom and the BootROM before SPL, nothing of ours holds EN. Exactly that. Our code starts at SPL at the earliest.

(b) Does the SY6280AAC block reverse current (OUT to IN) when disabled? UNVERIFIED. The PDF could not be read here: WebFetch returns the
raw PDF bytes (`pf.unikeyic.com` / `unikeyic.com` DS_SY6280.pdf) and the alldatasheet page returns HTTP 403. Secondary text from
search results says the SY6280 has reverse blocking with no body diode, and the EN falling threshold is 0.8 V; the Silergy copies in
search results say "Target design specification" in one case. That is not a datasheet read. So the BootROM window is harmless only if
reverse blocking holds with EN floating: UNVERIFIED. Read the Silergy datasheet (section on reverse current) before a board run.

## Gate and checks (on next 9efa797f; merged with origin/next d1adf855)

- `gaia validate` (board, family, product): ok, with `/mnt/ssd-samsung/gaia/bin/next-9efa797f/gaia`.
- `gaia lock --check` for `rock-5c-bench` and `rock-5c-base`: both match the descriptions. `rock-5c-bench/gaia.lock` is locked to
  Gaia `9efa797f` and Lemnos `31e0f736`.
- `gaia plan` for `rock-5c-bench` (throwaway pc-key, test version): ok. Not built. The plan's patch list (the plan's graph, nodes
  `radxa-rock-5c.u-boot.patched` and `rock-5c-bench.os.radxa-rock-5c.u-boot.patch0`): ONE entry, `devices/boards/radxa-rock-5c/assets/patches/u-boot/0001-rock-5c-hold-usb-otg-vbus-low.patch`.
  The family (rk35xx) has no patches, so the board's own list stays as is: PATCH-LISTS says a board's list follows its family's, and
  the family has none. The kernel and TF-A have no patches in the plan.
- `scripts/check-devices.sh` with the same binary: passes (3 families, 7 boards, 19 components, 7 products), including the Lemnos pin,
  the RK env rule, the PADS rule (ROCK 5C states `applies_pad_table = false` with its reason), and the bring-up self-test.
- `scripts/gate.sh`: see the gate result at the bottom of this section and in the PR comment.

## Bring-up product deleted (R1 owns this)

- `devices/products/rock-5c-bringup` is deleted. Its files moved to `devices/tests/fixtures/bringup-rock-5c/`: the bring-up rule's
  self-test (`bringup_rule_self_test` in `scripts/check-devices.sh`) copies that fixture in as `products/rock-5c-bringup` and
  runs its refusal cases on it, so the coverage stays.
- References updated: `devices/README.md` (bring-up list), `devices/components/dev-bringup-hostkey/component.toml`,
  `scripts/build-product.sh` (example name now `rock-5c-bench`). PADS.md's line "rock-5c-bringup: deleted in R1 (#131)" is the record.
- Nothing else in the tree names the product (grep).

## Dependencies on Z2 and C1 pieces not on next yet

R1 was cut from `pr/Z2`. The shared pieces R1's product needs are Z2's, and are not on next yet: the rk35xx `u-boot.fragment`
(env rule lines, Lemnos boot-path lines) and `linux.fragment` (PSTORE lines), `devices/components/lemnos0-addressing`,
`devices/components/bench-hostname`, `scripts/pc-key-strip.sh`, and the RK env rule and pc-key input in `scripts/check-devices.sh`.
The Z2 product (`zero-3e-bench`) and the C1 product (`cubie-a7s-bench`) are removed from R1: they are Z2's and C1's, not R1's.
So R1 lands after Z2 (or with its shared pieces).

## Missing (named, by owner)
- Lemnos: Q2g boot hold (merged + locked) and its PD4 claim: Q2g skips PD4 because the regulator `vcc5v0_usb_otg0` owns it (coordinator
  ruling 2026-11-14; Lemnos told). The PINS-2 merged rev of BB1b/BB2/BB3 (the list in "Pins" above).
- Gaia: ownership of the R1 additions to `board.toml`, `rk3588s-rock-5c-loader.dts` and the U-Boot patch (Z2 edited Gaia's files the same
  way); the U-Boot patch against v2026.10's file text (UNVERIFIED: the file on disk is Radxa's tree); ROCK 5C `boot-report` fit.
- Mainline: RK3588S ramoops confirmed with `/proc/iomem` at first boot (vendor values cited above); Ethernet GMAC driver present in the
  family kernel (UNVERIFIED).
- Hardware: SY6280AAC reverse-current datasheet read (b); pad reset state of GPIO0_D4; U-Boot late-init regulator behaviour; a meter check
  of VBUS on the port (B21's list applies).
- Z2 (shared pieces, see "Dependencies"): R1 lands with or after Z2. Shipping waits on the rk35xx family proof on the ZERO 3E.
- Nothing booted. Nothing flashed. No board access. No image built.
