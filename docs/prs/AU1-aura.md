# AU1: Luckfox Aura (RV1126B) bench slot image

Status: DRAFT. Untested on board. Nothing built, nothing flashed. The U-Boot port is not started (the coordinator's order, after the SDK lands). The Aura image is bench-only and hand-flashed: its DDR init is a closed rkbin blob.

## What this PR adds

- `devices/families/rv11xx/family.toml`: the RV1126B family in the current form (pins: Linux v7.3.4 `7595a62d`, U-Boot v2026.10, TF-A v2.15.0, rkbin `3e288fe` on the board). Kernel patches named as `[{ file = "patches/linux" }]`.
- `devices/families/rv11xx/patches/linux/`: the kernel series, applied by Gaia on v7.3.4 in order (see the table below).
- `devices/boards/waveshare-luckfox-aura/board.toml` and `assets/linux.fragment`: the Aura in the Z2 slot-disk form (SD boot; `u-boot`, `uboot-env` 16 MiB / 128 KiB, `boot` 128 MiB, `data` 1 GiB). Kernel options: pstore and USB gadget.
- `devices/products/waveshare-luckfox-aura/product.toml`: a DRAFT product with a placeholder `firmware` recipe. It is NOT a flashable bench image. No `gaia.lock` is committed.
- Validation: `gaia validate` (Gaia `c0d084f`) passes for the family, the board and the product. `scripts/check-devices.sh` stops at the known stale-binary refusal (`c0d084f` built, `devices/gaia.lock` pins `d8441c9`), so the gate is not passed here.

## Kernel series (v7.3.4), in apply order

Each patch is a plain unified diff against pristine v7.3.4. All seven apply in order with `patch -p1 --fuzz=0` (checked: the result equals the edited trees). Every driver object built with the kernel's own flags (aarch64 cross-compiler, `CONFIG` from `defconfig`) with zero warnings: `pinctrl-rockchip.o`, `phy-rockchip-inno-usb2.o`, `phy-rockchip-naneng-combphy.o`, `i2c-rk3x.o`. The Aura dts compiles with `cpp` and `dtc` with no warnings, from the applied tree. Nothing is booted.

| Patch | What it does | Status |
|---|---|---|
| 0001 pinctrl | RV1126B type, the 8 banks' iomux, and the pull, drive and schmitt calculators, with the drive and pull dispatch. Binding entry. | builds, 0 warnings; UNVERIFIED on hardware |
| 0002 pin groups | `rv1126b-pinctrl.dtsi`: the pcfg nodes plus the UART2 (`uart2m0`), SD (`sdmmc0`) and I2C0 (`i2c0m0`) groups, from the BSP | dts only |
| 0003 USB2 PHY | `rockchip,rv1126b-usb2phy`: tuning and the OTG and host port table (fields mainline has) | builds, 0 warnings |
| 0004 USB3 combo PHY | `rockchip,rv1126b-usb3-phy`: USB3 config (not used by the first image); the binding gets an RV1126B `if` block (two clocks, two resets); `combphy_reg.offset` widened to u32 | builds, 0 warnings |
| 0005 I2C | `rockchip,rv1126b-i2c` soc data (GRF -1); the rv1126 fallback is NOT equivalent, so it gets its own entry | builds, 0 warnings |
| 0006 SoC dtsi | `rv1126b.dtsi`, the boot set (see the table below); `php-grf` spelled as the mainline driver reads it | dtc clean |
| 0007 dwmac glue | `rockchip,rv1126b-gmac`: RGMII set-to-rgmii (delays in the IOC GRF), speed (VI GRF divider), binding (compatible, php-grf required). RMII and the integrated MAC-PHY not ported. | builds, 0 warnings (kernel flags); binding not checked with dt-schema |
| 0008 OTP | `rockchip,rv1126b-otp`: the BSP's byte-addressed read path (`rv1126b_otp_read`), data size 0x90, clocks usr, sbpi, apb, phy. Word size 0 so the common path passes byte offsets. | builds, 0 warnings |
| 0009 thermal | `rockchip,rv1126b-tsadc`: the BSP chip (v14 initialize, v5 get-temp, v4 alarm and tshut, the amplitude limiter, GRF bias and offset trim). Shared-code change: an optional `phy_init` hook, a `phy_config` field, and a probe call that is skipped when the hook is NULL. | builds, 0 warnings |
| 0010 SARADC | `rockchip,rv1126b-saradc`: eight 13-bit channels, 24 MHz, the v2 start and read with DAS 0x14 (the BSP's v3 non-DMA path). Shared-code change: an optional `das_soc_data` field, default 0xc as before. DMA mode is NOT ported. | builds, 0 warnings |
| 0011 Aura dts | `rv1126b-luckfox-aura.dts` and its Makefile line: ramoops, watchdog, UART2, SD, USB2 gadget, GMAC (RGMII), I2C0 pins | dtc clean |

## Register layouts: TRM not read

The RV1126B TRM is not on disk and is not downloaded. Every register offset, bit and value in this series is taken from the BSP driver and dtsi (Rockchip `kernel-develop-6.12`, `470f9dccb`, read-only). Each is marked "TRM not read" here, and the file:line citations are below. Nothing is checked against silicon.

- Pinctrl offsets (drive, pull, schmitt, bank iomux): BSP `drivers/pinctrl/pinctrl-rockchip.c` 2376-2501, 6368-6384. TRM not read.
- USB2 PHY offsets (tuning, port table): BSP `drivers/phy/rockchip/phy-rockchip-inno-usb2.c` 3451-3489, 5098-5148. TRM not read.
- USB3 combo PHY offsets (pipe-phy-grf 0x18000 and up, peri-grf 0x1003c): BSP `drivers/phy/rockchip/phy-rockchip-naneng-combphy.c` 1781-1801. TRM not read.
- I2C soc data: BSP `drivers/i2c/busses/i2c-rk3x.c` 1834-1838, and the i2c2 GRF comment at 1986. TRM not read.
- GMAC: VI GRF 0x50050 (interface, speed divider), IOC GRF GMACIO M0/M1 delay registers at 0x38BA0/0x38BA4 and 0x30BA8/0x30BAC: BSP `drivers/net/ethernet/stmicro/stmmac/dwmac-rk.c` 2872-2926 (defines) and 2928-2968 (set_to_rgmii and set_rgmii_speed). The M1 pair is numerically below the M0 pair; kept as the BSP has it. TRM not read.
- OTP (0008): byte-addressed read `rk3568_otp_read` in BSP `drivers/nvmem/rockchip-otp.c` 527-590 (the RV1126B entry uses it, `rv1126b_data` 1040-1045). RK3568_NBYTES 2 at line 94. TRM not read.
- Thermal (0009): BSP `drivers/thermal/rockchip_thermal.c` defines 260-268, 283, 338-351, 365-366; `rv1126b_tsadc_phy_init` 1731-1759; `rk_tsadcv14_initialize` 1292-1307; `rk_tsadcv5_get_temp` 1437-1452; `rk_tsadc_limit_amplitude` 1410-1433; `rk_tsadcv4_alarm_temp` 1536-1560; `rk_tsadcv4_tshut_temp` 1602-1617; chip data 1908-1919. TRM not read.
- SARADC (0010): BSP `drivers/iio/adc/rockchip_saradc.c` channels 512-521, data 523-530 (`das_soc_data = 0x14`), `start_v2` 124-148, `read_v3` 236-248. TRM not read.
- The dtsi base addresses and interrupt numbers: BSP `arch/arm64/boot/dts/rockchip/rv1126b.dtsi` as copied into 0006. TRM not read.

## Register and data citations (BSP: `/mnt/ssd-samsung/gaia/bsp/rockchip-bsp/kernel-develop-6.12`, read-only)

- Pinctrl (`drivers/pinctrl/pinctrl-rockchip.c`): RV1126B calculators `rv1126b_calc_drv/pull/schmitt_reg_and_bit` at lines 2376-2501 (the drive, pull and schmitt offsets). Bank iomux table `rv1126b_pin_banks` at 6368-6384 and `rv1126b_pin_ctrl` at 6387-6395. Dispatch: get-drive raw read at 4478-4484, set-drive at 4641-4646, get/set pull and pull-valid lists at 4849, 4922 and 5360. Match at 7073-7076. The BSP has no RV1126B mux route table: the mux is the bank's own 4-bit iomux registers. Mainline's `rockchip_get_pull` had no RV1126 case, so RV1126B has one.
- USB2 PHY (`drivers/phy/rockchip/phy-rockchip-inno-usb2.c`): tuning at 3451-3489, `rv1126b_phy_cfgs` at 5098-5148, match at 5220-5222. The BSP helpers take pointers; mainline maps them to `regmap_*` on `phy_base` (the PHY node's own regmap) and `grf`. `FILTER_COUNTER` (0xF4240) is defined in the port.
- USB3 combo PHY (`drivers/phy/rockchip/phy-rockchip-naneng-combphy.c`): cfg function at 1697-1779, grfcfg at 1781-1801, cfg at 1803-1813, match at 1853-1857. The mainline port uses `priv->type` and `priv->refclk`.
- I2C (`drivers/i2c/busses/i2c-rk3x.c`): BSP `rv1126b_soc_data` at 1834-1838 (`grf_offset = -1`, plus a DMA hook that this port leaves out). The BSP `rv1126_soc_data` is `grf_offset = 0x118`; the BSP comment at 1986 ties that GRF bit to i2c2 only. So the rv1126 fallback is not equivalent, and the dts names `rockchip,rv1126b-i2c` alone.

## What the ports leave out (stated, not hidden)

- USB2 PHY: the BSP's OTG ID and USB3-pipe fields (`pipe_phystatus`, `bypass_dm_en`, `bypass_sel`, `iddig_*`, `bvalid_grf_sel`, `vbus_det_en`, `port_ls_filter_con`) and the `vbus_detect` hook are not in mainline's structs. The Aura runs `dr_mode = "peripheral"`, so OTG ID is not used; this is a limit to lift before OTG host.
- USB3 combo PHY: the BSP's clamp (`u3otg0_clamp_dis` at pmu-grf 0x30000) is not ported; no pmu-grf node is in the dtsi. `force_det_out` is BSP-only.

## Not done in this PR (each with the reason)

- **dwmac (`drivers/net/ethernet/stmicro/stmmac/dwmac-rk.c`)**: not written. Mainline's `rv1126_ops` uses GRF 0x70 (the rv1126 layout). The BSP's RV1126B glue uses VI GRF 0x50050, the RK MAC-PHY registers at 0x500B4-0x500BC, and the php GRF (IOC GMACIO) at 0x38BA0 (BSP lines 2872-2995 and the `rv1126b_ops` at 3069-3077). The RGMII path needs `set_to_rgmii` and a speed hook, and the register equivalence with the rv1126 entry is not shown. The dts binds `rockchip,rv1126b-gmac` with the `snps,dwmac-4.20a` fallback, so the generic stmmac driver binds without the glue. Owner: Atlas (Lemnos review).
- **GMAC pinctrl**: the BSP tree has no RV1126B RGMII pin group (its only GMAC-named group is the on-chip PHY's `fephym1`, which the Aura does not use). `pinctrl-0` is not set. Owner: the schematic and the user, then Atlas.
- **U-Boot**: not started (coordinator). Waiting on the SDK path.

## Per-block status

Triage classes come from `/mnt/ssd-samsung/atlas-next-scratch-root/research/rv1126b-triage.md`. "Port source" is the Rockchip BSP at `/mnt/ssd-samsung/gaia/bsp/rockchip-bsp/kernel-develop-6.12` (read-only; Gaia's B7 worker fetched it, per the coordinator).

| Block | In this PR | Mainline v7.3.4 | Remaining work |
|---|---|---|---|
| CPU, PSCI, timer, GIC | `rv1126b.dtsi` (4x A53, `psci` smc, armv8 timer PPIs, `arm,gic-400`) | exists | none in the driver |
| CRU, reset | CRU node; `#reset-cells` | `rockchip,rv1126b-cru` exists | Reset names use mainline's spelling (`SRST_A_USB3OTG`, `SRST_A_GMAC`, `SRST_P_PIPEPHY`, `SRST_REF_PIPEPHY`, `SRST_P_OTPC_NS`). Vendor numeric IDs not compared (UNVERIFIED). |
| CRU PLL/clock setup | not carried | - | The vendor `assigned-clocks` (PLL and fractional rates for UART, CM, audio) are not in the dtsi. Boot uses the reset clock rates until they are added. |
| GRF, IOC GRF | `grf`, `ioc_grf` syscon nodes | `syscon` works | UNVERIFIED per consumer: the register offsets the PHYs and GMAC use. |
| PMU | node only, no power domains | binding and power domain driver exist | power-controller children (vendor `rv1126b-power.h` IDs; mainline's header is `rockchip,rv1126b-power-controller.h`) not carried. |
| Pinctrl and GPIO | pinctrl node + 8 GPIO banks (`rockchip,gpio-bank`) | 0001 adds the RV1126B banks, calculators and dispatch | builds; the pin groups are 0002 |
| UART2 (console) | `uart2` node, `rockchip,rv1126b-uart` + `snps,dw-apb-uart` | 8250_dw | pinctrl-0 set to `uart2m0_xfer_pins` (0002). Aura console pins UNVERIFIED (the EVB uses uart2m0). |
| Watchdog | `wdt`, enabled on the Aura | `snps,dw-wdt` exists | none |
| SD (microSD) | `sdmmc0` (`rk3288-dw-mshc` fallback), enabled | `dw_mmc-rockchip` binds via fallback | pinctrl-0 set (0002, bus4, cmd, clk, detn). Card-detect and clock phase UNVERIFIED. vmmc is a fixed stand-in (see PMIC). |
| eMMC | `emmc` node, disabled | same as SD | not used for the bench image (SD first). |
| USB 2 PHY | `usb2phy`, `usb2phy_otg` (interrupts per vendor) | 0003 adds the RV1126B tuning and cfg | builds; OTG ID fields left out (see above) |
| USB 3 combo PHY | `usb3phy` node, disabled | 0004 adds the RV1126B cfg, binding, and the u32 offset | builds; not in the first image |
| DWC3 (USB OTG Type-C) | `usb_drd_dwc3`, `rockchip,rv1126b-dwc3` + `rockchip,rk3576-dwc3` fallback; the Aura runs `dr_mode = "peripheral"` on `usb2phy_otg` only | glue binds rk3576 fallback UNVERIFIED | none expected; the first image is USB 2 gadget only |
| GMAC (Ethernet) | `gmac` node, `rockchip,rv1126b-gmac` + `snps,dwmac-4.20a`, enabled on the Aura with RGMII | not written (see above) | Atlas: glue. Generic stmmac binds meanwhile. Ethernet stays in the image (user ruling). |
| I2C0 | `i2c0` with `rockchip,rv1126b-i2c`, disabled; pinctrl-0 `i2c0m0_pins` | 0005 adds the RV1126B soc data | Register comparison done: the rv1126 fallback differs (GRF 0x118 vs none), so it is its own patch 0005. |
| OTP (serial source) | `otp` node, disabled, with the vendor's clocks and resets | 0008 adds the RV1126B read path and data | builds; `hwlocks` not carried; serial offsets UNVERIFIED |
| SARADC0 | `saradc0` node, disabled | 0010 adds the RV1126B data | builds; DMA mode not ported |
| TSADC (thermal) | `tsadc` node, disabled | 0009 adds the RV1126B chip and GRF trim | builds; thermal zones not in the dtsi |
| PWM, SPI, other UARTs and I2Cs, GPU/VPU/ISP/NPU | not carried | - | later patches; not needed for the boot-to-shell set |
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

## U-Boot (scoped, not started in code)

- Seed: Rockchip `u-boot-next-dev` at `1c535d65` (`/mnt/ssd-samsung/gaia/bsp/rockchip-bsp/u-boot-next-dev`, read-only). Has `arch/arm/mach-rockchip/rv1126b` (`clk_rv1126b.c`, `rv1126b.c`, `syscon_rv1126b.c`, `Kconfig`, `Makefile`; 881 lines in all) and the rv1126b defconfigs under `configs/`.
- Mainline v2026.10 on disk: `/mnt/ssd-samsung/atlas-next-scratch-root/research/u-boot` and `scratch-uboot-mainline`, both at `5508406582f6` (v2026.10 prep). No download needed.
- DDR: `drivers/ram/rockchip/sdram_rv1126b.c` in the seed is a stub that returns -1 under `CONFIG_TPL_BUILD` (seed line 1-12). The real DDR init is the closed rkbin blob `rv1126b_ddr_1332MHz_v1.10.bin`, so the TPL goes through binman with that blob.
- rkcommon: the seed's `tools/rkcommon.c` has NO rv1126b entry (its rv1126 entry is at line 149), and the vendor image comes from rkbin's `boot_merger`, not from `mkimage -T rksd`. So there is no vendor value to port for the header magic, and none is written. The mainline `rkcommon.c` has no rv1126b entry either.
- BL31: upstream TF-A `plat/rockchip/rv1126b` first (ruling 3). Not built here. The rkbin `rv1126b_bl31_v1.13.elf` is the fallback only if upstream fails; it is a closed blob.
- Closed blobs: the DDR blob, and the BL31 blob if used. An Aura image built with either stays on the bench and its image metadata must say so. The series notes and this doc say so.
- Status: a big port (the triage's class). The first patch would be the SoC plumbing (Kconfig, Makefile, the rv1126b SoC file, clk, syscon), then the board and defconfig. None of it is written or built yet; see the report for why.

## U-Boot (historical notes)

- SoC port: not written. Seed: the BSP `u-boot-next-dev` (`arch/arm/mach-rockchip/rv1126b` absent in mainline v2026.10; present in the BSP as `configs/rv1126b_defconfig`, `include/dt-bindings/clock/rockchip,rv1126b-cru.h`, `drivers/misc/rv1126b-secure-otp.S`, and others). Clean pinned series on v2026.10, Atlas's. Lemnos reviews env and boot-count parts; Gaia builds. **Owner: Atlas. Waiting: the SDK source path from Gaia (B7).**
- DDR (TPL): the rkbin closed blob. Vendor `sdram_rv1126b.c` in the BSP `next-dev` is a stub. **Gap: closed blob, not ours to write.**
- `tools/rkcommon.c`: no `rv1126b` header entry (`mkimage -T rksd` fails). Magic and version UNVERIFIED (vendor `rkcommon.c` not read). **Small port, Atlas.**
- BL31: upstream TF-A `plat/rockchip/rv1126b` first (ruling 3). **UNVERIFIED: not built.** The rkbin `rv1126b_bl31_v1.13.elf` is the fallback, used only if upstream fails, and would then be named as a closed blob.
- Maskrom flash path: the Lemnos worker's (not in this PR).

## Missing list

Grouped by owner. "Gaia" is the build owner; "Lemnos" owns non-trivial driver work and the boot-flow review (coordinator).

1. **Atlas, kernel series, still to write:** SARADC DMA mode (BSP `start_v3_dma`, `rockchip,dma`; not ported, not needed for basic reads); dt-bindings for `rockchip,rv1126b-grf`, `-pmu`, `-otp`, `-uart`, `-ioc-grf`, `-tsadc`, `-saradc`, and `-pinctrl` (the pinctrl and otp, tsadc, saradc and dwmac bindings are partly in the series; the others are not yet). The CRU is already in the tree. dwmac RMII and the integrated MAC-PHY are not ported.
2. **Atlas, dts:** CRU `assigned-clocks` (vendor PLL and fractional rates); PMU power-domain children; GMAC pin group (none in the BSP tree); `aliases` and `chosen` checked against the Aura schematic.
3. **Atlas, U-Boot port:** as above (SoC port, rkcommon entry, env and boot-flow options). Waiting on the SDK path.
4. **Gaia:** apply the seven `[kernel] patches` on the real tree (the apply is tested here, not by Gaia); build the image; `gaia lock` the rkbin and U-Boot pins when given revs. The `d8441c9` vs `c0d084f` stale-binary refusal is Gaia's pin to move.
5. **Lemnos:** an RV1126B SoC map and a serve-only board file (lemnos-board.toml is not written here); USB gadget identity (the USB id `2207:110b` is a guess from U-Boot's RV1126 default, UNVERIFIED: check `lsusb -d 2207:` in maskrom); U-Boot boot-flow review (env, boot count); the pinctrl and PHY drivers, for review.
6. **Bench (user):** power port question; identify the PMIC; read the Aura console pins, microSD card-detect and GMAC RGMII pins from the schematic or a board; confirm the PHY address and RGMII delays; confirm the ramoops region against `/proc/iomem`.
7. **Closed blob (user decision):** the DDR init `rv1126b_ddr_1332MHz_v1.10.bin` is a closed rkbin blob. The image stays bench-only and unpublished.

## Blockers (who owes what)

- SDK: the RV1126B SDK is not fetched. Gaia's B7 worker is fetching it to `/mnt/ssd-samsung/gaia/bsp/`. The BSP trees that are on disk: `rkbin` (RV1126B blobs present at `3e288fe`), `rockchip-bsp/kernel-develop-6.12` and `u-boot-next-dev`. `luckfox-pico` is the RV1106 SDK, not this chip. Used read-only. **Owner: Gaia (B7), coordinator relays the rev.**
- U-Boot: waiting on the SDK path (coordinator). **Owner: Atlas.**
- Pinctrl driver: **Atlas (series), Lemnos review.**
- Lemnos SoC map and board file: **Lemnos.**
- PMIC, power port, console and card-detect pins, PHY address: **user, on the bench.**

🤖 Generated with [Claude Code](https://claude.com/claude-code)
