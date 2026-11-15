# AU1: Luckfox Aura (RV1126B) bench slot image

Status: DRAFT. Untested on board. Nothing built, nothing flashed. The U-Boot port is not started (the coordinator's order, after the SDK lands). The Aura image is bench-only and hand-flashed: its DDR init is a closed rkbin blob.

## What this PR adds

- `devices/families/rv11xx/family.toml`: the RV1126B family in the current form (pins: Linux v7.3.4 `7595a62d`, U-Boot v2026.10, TF-A v2.15.0, rkbin `3e288fe` on the board). Kernel patches named as `[{ file = "patches/linux" }]`.
- `devices/families/rv11xx/patches/linux/`: the kernel series, applied by Gaia on v7.3.4 in order.
  - `0001-arm64-dts-rockchip-add-the-RV1126B-SoC-dtsi.patch`: `rv1126b.dtsi`, the boot set (below).
  - `0002-arm64-dts-rockchip-add-the-Luckfox-Aura-board-dts.patch`: `rv1126b-luckfox-aura.dts` and its Makefile line. Ramoops and the watchdog are enabled.
  - Both patches were tested: they apply with `patch -p1 --fuzz=0` on the v7.3.4 `Makefile`, and the Aura dts compiles with `cpp` and `dtc` against the v7.3.4 include tree with no warnings.
- `devices/boards/waveshare-luckfox-aura/board.toml` and `assets/linux.fragment`: the Aura board in the Z2 slot-disk form (SD boot; `u-boot`, `uboot-env` 16 MiB / 128 KiB, `boot` 128 MiB, `data` 1 GiB). Kernel options: pstore (crash log) and USB gadget.
- `devices/products/waveshare-luckfox-aura/product.toml`: a DRAFT product with a placeholder `firmware` recipe. It is NOT a flashable bench image. No `gaia.lock` is committed.
- Validation: `gaia validate` (Gaia `c0d084f`) passes for the family, the board and the product. `scripts/check-devices.sh` stops at the known stale-binary refusal (`c0d084f` built, `devices/gaia.lock` pins `d8441c9`), so the gate is not passed here.

## Per-block status

Triage classes come from `/mnt/ssd-samsung/atlas-next-scratch-root/research/rv1126b-triage.md`. "Port source" is the Rockchip BSP at `/mnt/ssd-samsung/gaia/bsp/rockchip-bsp/kernel-develop-6.12` (read-only; Gaia's B7 worker fetched it, per the coordinator).

| Block | In this PR | Mainline v7.3.4 | Remaining work |
|---|---|---|---|
| CPU, PSCI, timer, GIC | `rv1126b.dtsi` (4x A53, `psci` smc, armv8 timer PPIs, `arm,gic-400`) | exists | none in the driver |
| CRU, reset | CRU node; `#reset-cells` | `rockchip,rv1126b-cru` exists | Reset names use mainline's spelling (`SRST_A_USB3OTG`, `SRST_A_GMAC`, `SRST_P_PIPEPHY`, `SRST_REF_PIPEPHY`, `SRST_P_OTPC_NS`). Vendor numeric IDs not compared (UNVERIFIED). |
| CRU PLL/clock setup | not carried | - | The vendor `assigned-clocks` (PLL and fractional rates for UART, CM, audio) are not in the dtsi. Boot uses the reset clock rates until they are added. |
| GRF, IOC GRF | `grf`, `ioc_grf` syscon nodes | `syscon` works | UNVERIFIED per consumer: the register offsets the PHYs and GMAC use. |
| PMU | node only, no power domains | binding and power domain driver exist | power-controller children (vendor `rv1126b-power.h` IDs; mainline's header is `rockchip,rv1126b-power-controller.h`) not carried. |
| Pinctrl and GPIO | pinctrl node + 8 GPIO banks (`rockchip,gpio-bank`) | `pinctrl-rockchip.c` has rv1126 data only | **small port**: RV1126B bank/mux/route/pull data in `pinctrl-rockchip.c`. Pin groups exist in the BSP `rv1126b-pinctrl.dtsi` and `rv1126b-pinconf.dtsi`, not in the dtsi. |
| UART2 (console) | `uart2` node, `rockchip,rv1126b-uart` + `snps,dw-apb-uart` | 8250_dw | pinctrl-0 not set (see pinctrl). Aura console pins UNVERIFIED (the EVB uses uart2m0). |
| Watchdog | `wdt`, enabled on the Aura | `snps,dw-wdt` exists | none |
| SD (microSD) | `sdmmc0` (`rk3288-dw-mshc` fallback), enabled | `dw_mmc-rockchip` binds via fallback | pinctrl-0 not set. Card-detect and clock phase/delay UNVERIFIED. vmmc is a fixed stand-in (see PMIC). |
| eMMC | `emmc` node, disabled | same as SD | not used for the bench image (SD first). |
| USB 2 PHY | `usb2phy`, `usb2phy_otg` (interrupts per vendor) | no `rv1126b`/`rv1126` match | **small port**: `phy-rockchip-inno-usb2.c` cfg for RV1126B. Closest reference is rk3576. Register layout UNVERIFIED. |
| USB 3 combo PHY | `usb3phy` node, disabled | no match | **small port**: `phy-rockchip-naneng-combphy.c` cfg. Not in the first image. |
| DWC3 (USB OTG Type-C) | `usb_drd_dwc3`, `rockchip,rv1126b-dwc3` + `rockchip,rk3576-dwc3` fallback; the Aura runs `dr_mode = "peripheral"` on `usb2phy_otg` only | glue binds rk3576 fallback UNVERIFIED | none expected; the first image is USB 2 gadget only |
| GMAC (Ethernet) | `gmac` node, `rockchip,rv1126b-gmac` + `snps,dwmac-4.20a`, enabled on the Aura with RGMII | `dwmac-rk.c` has `rockchip,rv1126-gmac` only | **small port**: `dwmac-rk` match + ops for RV1126B, or the rv1126 fallback if its GRF offsets match (UNVERIFIED, register comparison owed). Ethernet stays in the image (user ruling). |
| I2C0 | `i2c0` with `rockchip,rv1126b-i2c` + `rockchip,rv1126-i2c` fallback, disabled | `i2c-rk3x.c` has rv1126 soc_data | UNVERIFIED: register comparison against the vendor tree (coordinator ruling 1). Any difference becomes its own patch. |
| OTP (serial source) | `otp` node, disabled, with the vendor's clocks and resets | no match | **small port** (`rockchip-otp.c`). `hwlocks` not carried (no hwspinlock node). Serial offsets UNVERIFIED. |
| SARADC, TSADC (thermal), PWM, SPI, other UARTs and I2Cs, GPU/VPU/ISP/NPU | not carried | - | later patches; not needed for the boot-to-shell set |
| OPP tables, idle states | not carried | - | cpufreq and idle are not needed for boot |
| ramoops | Aura dts reserved-memory node | `ramoops` generic | placement UNVERIFIED: `0x110000` is a placeholder (not checked against TF-A, U-Boot or kernel use) |
| Watchdog owner | Lemnos owns the runtime watchdog (lemnosd); the kernel `wdt` node is enabled | - | none |

## Aura board facts

| Fact | Status |
|---|---|
| Storage: microSD boot | Ruled (coordinator, SD first). eMMC layout unconfirmed. |
| USB: USB 3 OTG Type-C as a USB 2 gadget, peripheral mode | Port per the user's ruling. SuperSpeed not in this image. |
| Power: 5 V 3 A; a PC port may not supply it | UNVERIFIED. Whether a separate power port exists is still to confirm (user). |
| Console UART | UNVERIFIED: the dts uses the EVB's uart2 (1.5 Mbaud, `stdout-path`). Aura pins from the schematic still owed. |
| Ethernet: RGMII PHY, probably RTL8211F-family | UNVERIFIED chip (part marking not legible, triage). PHY address (`reg = <0>`) and RGMII delays from strap names only. |
| PMIC | UNIDENTIFIED. Per ruling 5: two fixed supplies (`vcc3v3_sys`, always on) stand in, marked in the dts. Identify on the bench. |
| DDR | Closed rkbin blob `rv1126b_ddr_1332MHz_v1.10.bin` (ruling 4). Bench-only, never published. |

## U-Boot (not started, per the coordinator)

- SoC port: not written. Seed: the BSP `u-boot-next-dev` (`arch/arm/mach-rockchip/rv1126b` absent in mainline v2026.10; present in the BSP as `configs/rv1126b_defconfig`, `include/dt-bindings/clock/rockchip,rv1126b-cru.h`, `drivers/misc/rv1126b-secure-otp.S`, and others). Clean pinned series on v2026.10, Atlas's. Lemnos reviews env and boot-count parts; Gaia builds. **Owner: Atlas. Waiting: the SDK source path from Gaia (B7).**
- DDR (TPL): the rkbin closed blob. Vendor `sdram_rv1126b.c` in the BSP `next-dev` is a stub. **Gap: closed blob, not ours to write.**
- `tools/rkcommon.c`: no `rv1126b` header entry (`mkimage -T rksd` fails). Magic and version UNVERIFIED (vendor `rkcommon.c` not read). **Small port, Atlas.**
- BL31: upstream TF-A `plat/rockchip/rv1126b` first (ruling 3). **UNVERIFIED: not built.** The rkbin `rv1126b_bl31_v1.13.elf` is the fallback, used only if upstream fails, and would then be named as a closed blob.
- Maskrom flash path: the Lemnos worker's (not in this PR).

## Missing list

Grouped by owner. "Gaia" is the build owner; "Lemnos" owns non-trivial driver work and the boot-flow review (per the coordinator).

1. **Atlas, kernel series:** pinctrl RV1126B data; USB 2 PHY; USB 3 combo PHY; OTP; GMAC glue (or the verified rv1126 fallback); I2C fallback check; SARADC; thermal; dt-bindings for the new compatibles (`rockchip,rv1126b-grf`, `-pmu`, `-otp`, `-usb2phy`, `-usb3-phy`, `-gmac`, `-i2c`, `-uart`, `-pinctrl`, `-ioc-grf`, `-cru` is already in the tree).
2. **Atlas, dts:** the pin groups (copy from the BSP `rv1126b-pinctrl.dtsi`, `rv1126b-pinconf.dtsi`), then `pinctrl-0` on uart2, sdmmc0, gmac and i2c0 once the pinctrl driver can decode them; CRU `assigned-clocks`; PMU power-domain children; `aliases`/`chosen` checked against the Aura schematic.
3. **Atlas, U-Boot port:** as above (SoC port, rkcommon entry, env and boot-flow options). Waiting on the SDK path.
4. **Gaia:** apply the `[kernel] patches` (file form validates; the apply on the real tree is Gaia's test); build the image; `gaia lock` the rkbin and U-Boot pins when given revs. Gaia's `d8441c9` vs `c0d084f` stale-binary refusal is Gaia's pin to move.
5. **Lemnos:** an RV1126B SoC map and a serve-only board file (lemnos-board.toml is not written here); USB gadget identity (the USB id `2207:110b` is a guess from U-Boot's RV1126 default, UNVERIFIED: check `lsusb -d 2207:` in maskrom); U-Boot boot-flow review (env, boot count).
6. **Bench (user):** power port question; identify the PMIC; read the Aura console pins and microSD card-detect from the schematic or a board; confirm the PHY address and RGMII delays; confirm the ramoops region against `/proc/iomem`.
7. **Closed blob (user decision):** the DDR init `rv1126b_ddr_1332MHz_v1.10.bin` is a closed rkbin blob. The image stays bench-only and unpublished.

## Blockers (who owes what)

- SDK: the RV1126B SDK is not fetched. Gaia's B7 worker is fetching it to `/mnt/ssd-samsung/gaia/bsp/`. The BSP trees that are on disk: `rkbin` (RV1126B blobs present at `3e288fe`), `rockchip-bsp/kernel-develop-6.12` and `u-boot-next-dev`. `luckfox-pico` is the RV1106 SDK, not this chip. Used read-only. **Owner: Gaia (B7), coordinator relays the rev.**
- U-Boot: waiting on the SDK path (coordinator). **Owner: Atlas.**
- Pinctrl driver: **Atlas (series), Lemnos review.**
- Lemnos SoC map and board file: **Lemnos.**
- PMIC, power port, console and card-detect pins, PHY address: **user, on the bench.**

## Not done in this PR (honest)

- `gaia plan`/`gaia lock` for the product: not run (no lock; Gaia pins not given).
- Kernel build of the series: not run. Only the dtc compile of the Aura dts and the patch apply were tested.
- No Gaia image, no board, no flash, no USB. No downloads.
- `scripts/check-devices.sh` not passed (stale-binary refusal, see above).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
