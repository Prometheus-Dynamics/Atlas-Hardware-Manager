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
| 0011 Aura dts | `rv1126b-luckfox-aura.dts`: RK801 PMIC with the vendor regulators, UART0 console, sdmmc0 with vmmc and vqmmc, the external RGMII PHY (vendor delays and 25 MHz clock), USB OTG, ramoops, watchdog | dtc clean (checked from the applied series) |

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
- **GMAC pinctrl (done, UNTESTED ON BOARD)**: the GMAC1 M0 RGMII groups come from the vendor reference `rv1126b-pinctrl.dtsi` (`eth` node 247-302, `eth_clk_25m` 397-403), copied into 0002 with the pcfg nodes from BSP pinconf 107-110 and 137-140. The Aura uses them for the external PHY, not `fephym1` (the integrated PHY). The 25 MHz output group is not used: the PHY has its own crystal (UNVERIFIED).
- **io-domain for VCCIO_PHY1 (no node added)**: mainline's io-domain driver has `rockchip,rv1126-pmu-io-voltage-domain` only (`drivers/soc/rockchip/io-domain.c` 604-605). VCCIO_PHY1 is a GRF-side rail set by the strap block "VCC_PHY0_IO Voltage Config" (R218-R226), not a PMU domain. Its voltage is UNVERIFIED (the sheet does not print it, and the RTL8211F strap table was not read). Owner: the user (board measurement) and Atlas.
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
| UART0 (console) | `uart0` node added to the SoC dtsi; pinctrl-0 `uart0m0_xfer_pins`, the vendor's choice (vendor U-Boot stdout = uart0). Settled by the vendor dts; the bench still has to confirm the pads. |
| Watchdog | `wdt`, enabled on the Aura | `snps,dw-wdt` exists | none |
| SD (microSD) | `sdmmc0` (`rk3288-dw-mshc` fallback), enabled | `dw_mmc-rockchip` binds via fallback | pinctrl-0 set (0002, bus4, cmd, clk, detn). Card-detect and clock phase UNVERIFIED. vmmc is a fixed stand-in (see PMIC). |
| eMMC | `emmc` node, disabled | same as SD | not used for the bench image (SD first). |
| USB 2 PHY | `usb2phy`, `usb2phy_otg` (interrupts per vendor) | 0003 adds the RV1126B tuning and cfg | builds; OTG ID fields left out (see above) |
| USB 3 combo PHY | `usb3phy` node, disabled | 0004 adds the RV1126B cfg, binding, and the u32 offset | builds; not in the first image |
| DWC3 (USB OTG Type-C) | `usb_drd_dwc3`, `rockchip,rv1126b-dwc3` + `rockchip,rk3576-dwc3` fallback; the Aura runs `dr_mode = "peripheral"` on `usb2phy_otg` only | glue binds rk3576 fallback UNVERIFIED | none expected; the first image is USB 2 gadget only |
| GMAC (Ethernet) | `gmac` node, `rockchip,rv1126b-gmac` (glue, 0007), external RGMII PHY U25 on GMAC1 M0, `phy-mode = "rgmii-rxid"`, PHY at address 1, pinctrl-0 = the five `ethm0` groups (0002). | glue builds; pins UNTESTED ON BOARD; Ethernet stays in the image (user ruling). |
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
| Console UART | SETTLED by the vendor SDK: UART0 on M0 (`uart0m0_xfer_pins`), 1500000 baud. The schematic's DEBUG nets are named NLUART20TX0M0 / RX0M0 (read as UART0 TX/RX on M0). Confirm the pads on the bench. |
| Ethernet: external RGMII PHY U25 (schematic ETHER sheet, not the integrated FE PHY). Vendor (SDK 202611) settles: PHY `ethernet-phy-ieee802.3-c22` at address 1, `phy-mode = "rgmii"` with `tx_delay = <0x20>` and no rx_delay (the PHY strap supplies RX delay), reset gpio6 PB6 active low (20 ms, then 100 ms), 25 MHz from the SoC (CLK_MAC_OUT2IO on eth_clk_25m_out_m0). Still unverified: the part (not legible on the render), the strap level (RXDLY high is the schematic reading). |
| PMIC | SETTLED by the vendor dtsi: RK801 at I2C0 0x27, interrupt gpio0 PC0, regulators as in the vendor dtsi (section below). The schematic text has no PMIC part name; the identity rests on the vendor dtsi. |
| DDR | Closed rkbin blob `rv1126b_ddr_1332MHz_v1.10.bin` (ruling 4). Bench-only, never published. |

## U-Boot (scoped; rkcommon blocked on a decision; SoC port not written)

rkcommon facts, all from disk (no download):
- Table fields (mainline `tools/rkcommon.c` 131-158): imagename, the 4-byte SPL header, the SPL size limit, RC4, header version. The rv1126 row is `{ "rv1126", "110B", 0x10000 - 0x1000, false, RK_HEADER_V1 }` (line 157).
- Size limit: the RV1126B `system_sram` is 0x3ffb0000 for 0x10000 bytes (seed/BSP dtsi `system_sram`, vendor dtsi lines 3929-3935; `aoa_sram` 0x3ff20000 for 0x10000 at 3920-3927). A 0x10000 - 0x1000 limit follows the rv1126 row's convention. UNVERIFIED for this SoC.
- The ROM header: NOT derivable on disk. The rkbin RV1126B SPL (`bin/rv11/rv1126b_spl_v1.06.bin`) begins with `01 00 00 14`, a raw AArch64 branch. The RK3568 and RK3576 SPLs begin the same way. So the 4-byte header is added by mkimage (mainline) or boot_merger (vendor), and the RV1126B value is in neither file. Mainline's `110B` is the rv1126 magic. Reusing it for RV1126B would be a guess.
- Header version: RK_HEADER_V1 (rv1126, rv1108) or V2 (rk3568, rk3576). RV1126B's rkbin ini has NEWIDB=true, as the RK35 and RV1106 inis do. That does not decide it.
- The vendor loader: `RKBOOT/RV1126BMINIALL.ini` (NAME=RV110F; FlashData = the DDR blob; FlashBoot = `bin/rv11/rv1126b_spl_v1.06.bin`; LOAD_ADDR=0x4fe00000; RC4 off) and boot_merger produce `rv1126b_loader_v1.10.106.bin`. The BSP's `tools/rkcommon.c` has no RV1126B entry, so the vendor never uses mkimage for this chip.

DECISION NEEDED (coordinator): (a) add an rkcommon row with a magic and header version the user supplies or the TRM gives; or (b) build the loader with rkbin's boot_merger from the RKBOOT ini, as the vendor does, and skip mkimage rksd for this chip. (b) needs no magic and matches the BSP. No rkcommon row is written until one of these is chosen.

The SoC port itself is not written. The BSP seed is `arch/arm/mach-rockchip/rv1126b` (787 lines in `rv1126b.c`, 41 in `clk_rv1126b.c`, 25 in `syscon_rv1126b.c`, 17 Kconfig, 11 Makefile), `drivers/pinctrl/rockchip/pinctrl-rv1126b.c`, the dtsi files under `arch/arm/dts`, `configs/rv1126b_defconfig` (205 lines), and `include/configs/rv1126b_common.h` (163 lines). Mainline v2026.10 has the rv1126 precedent to follow (`arch/arm/mach-rockchip/rv1126`, `clk_rv1126.c`, `pinctrl-rv1126.c`). It is a multi-file port, and it has not been built.

## U-Boot (historical scoping, superseded above)

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

## 2026-11-16 update: GMAC pins, bindings, rkcommon, gate

- **GMAC1 M0 RGMII groups** (0002, UNTESTED ON BOARD, TRM not read): vendor refs `/mnt/ssd-samsung/gaia/work/b7/ref/rv1126b-pinctrl.dtsi` 247-302 and 397-403; pcfg BSP `rv1126b-pinconf.dtsi` 107-110 and 137-140. Schematic: `/mnt/ssd-samsung/gaia/work/b7/aura-docs/schematic.pdf` (1 page, sha256 036cb8f0ba9f4e5dcf82d46a8e75c51fd7e41310c12ba9cfb8680cd0b33c5b5b), ETHER sheet nets `GMAC1_*_M0`, strap table, and the `VCC_PHY0_IO Voltage Config` block.
- **Aura gmac** (0011, superseded 2026-11-16 by the vendor values in the section below): 
- **Bindings (dt-schema)**: 0008 adds `rockchip,rv1126b-otp` with four clocks and resets (usr, sbpi, apb, phy); 0009 adds `rockchip,rv1126b-tsadc` and an RV1126B-only block for the third clock `tsadc_phyctrl` (the shared clock limit is unchanged); 0010 adds `rockchip,rv1126b-saradc`. **`dtbs_check` has NOT run**: dt-schema is not installed and no download was cleared. The YAML parses.
- **rkcommon row** (U-Boot, scratch patch at `uboot/rkcommon.patch`): `{ "rv1126b", "110F", 0x10000 - 0x1000, false, RK_HEADER_V2 }`, citing `RKBOOT/RV1126BMINIALL.ini` line 2 (`NAME=RV110F`), line 25 (`NEWIDB=true`), line 27-28 (RC4 off). Mainline V2 rows use the family tag as a label (`RK35` for rk3568/rk3576); the image gets the `RKNS` magic. The size limit follows the rk3568/rk3576 rows' 0x1000 reserve, against 64 KiB system SRAM at 0x3ffb0000. UNVERIFIED until a board boots it. **Not built**: an out-of-tree host build could not run in this pass.
- **Closed blobs**: the DDR init (`rv1126b_ddr_1332MHz_v1.10.bin`) and BL31 if used. An Aura image built with them stays on the bench, and its image metadata must say so.
- **CI gate (25065379, run 43687683943) failed for two reasons, neither the stale-lock refusal alone**:
  1. `families/rv11xx/family.toml:41: kernel.patches: unknown key`. The gate pins Gaia `d8441c9` (`devices/gaia.lock`), which does not accept `[kernel] patches`. My local check used `c0d084f`, which does, and the gate correctly refuses that binary as stale. The fix is a Gaia pin that has `patches` (the coordinator's pending pin move). I did not move the pin.
  2. `products/waveshare-luckfox-aura/product.toml: no gaia.lock beside it`. The lock cannot be written until the Gaia pin accepts the family (1).
- **Not done**: the U-Boot SoC series (rv1126b.c, clk, pinctrl, dtsi, defconfig, common header). It is a multi-file port that needs a build to check, and it is not started in code.

## Vendor SDK check (2026-11-16): what the vendor files settle

Source: Luckfox_Aura_SDK_202611.tar.gz, sha256 b9a95edbd9b17efef477509926874b498549e9df2712a2bcd6bb60d6b3283b2c. The extracted files are at `/mnt/ssd-samsung/gaia-bsp-aura/aura/` and match `aura-files.sha256` (25 of 25). Revs: kernel c67d457cf327 (6.1), U-Boot 649e8bd3474c, rkbin 094fb0b6. Read only: nothing from the SDK was executed.

### Pads statement (devices/boards/waveshare-luckfox-aura/pads.toml)

"no pad/voltage table applied by the kernel dts or by U-Boot source."

- U-Boot `rv1126b.c`: lines 85-86 define `GPIO1_IOC_IO1_VSEL` (0x908) and `VCCIO1_VD_3V3` (BIT 15). Lines 510-511 READ that bit to pick the FSPI drive strength. No line in `rv1126b.c` writes a `*_vsel` register; `ioc_rv1126b.h` declares them at 0x900-0x920 (lines 44-443) and nothing uses them.
- The vendor kernel dts has no io-domain node and no voltage table.
- The SPL is a closed blob (`rv1126b_spl_ipc_v1.05.bin`, `RV1126BMINIALL_IPC.ini`). What it programs is not visible in source, so it is not stated.
- Bank supplies, where the vendor dtsi gives them: GPIO2 (the sdmmc0 data, command and clock pins, `rv1126b-pinctrl.dtsi` sdmmc0 groups, bank 2) has `vqmmc = vccio_sd` (`rv1126b-luckfox-aura.dts:88-89`), a fixed 3.3 V regulator (`rv1126b-luckfox-aura.dtsi:111-120`), fed from `vcc3v3_sys` = RK801 DCDC_REG2 at 3.3 V (`dtsi:381-393`). Every other bank is unstated: the vendor gives no supply for them. The PHY I/O rail (VCCIO_PHY1, schematic) has no vendor regulator, and is unstated.

### RK801 (PMIC)

- The vendor dtsi places it at I2C0 0x27 with `interrupts = <gpio0 PC0 low>` (`dtsi:344-349`). The schematic text has no PMIC part name: the identity rests on the vendor dtsi only.
- Mainline v7.3.4 HAS the driver: the MFD (`drivers/mfd/rk8xx-i2c.c` 192-193, 257; `rk8xx-core.c` 65-69) and the binding `Documentation/devicetree/bindings/mfd/rockchip,rk801.yaml`; and the regulators (`drivers/regulator/rk808-regulator.c` `rk801_reg[]` 1199-1280: dcdc1-4, ldo1, ldo2, switch; buck ranges 1174-1190). No new triage item for the driver.
- Our dts now uses those regulators, with the vendor values: dcdc1 `vdd_npu` 0.5-1.5 V; dcdc2 `vcc3v3_sys` 3.3 V; dcdc3 `vcc_ddr` (no range in the vendor); dcdc4 `vdd_logic` 0.5-1.5 V; ldo1 `vdd0v9_sys`; ldo2 `vcc_1v8` 1.8-3.3 V; switch `vcc_3v3`. All are always-on, as the vendor marks them.
- Not carried: the vendor's `pwrctrl-gpios` (gpio0 PC1) and `pmic-reset` pinctrl state; the mainline binding does not accept them. `soc_pwrctrl_reset` is in the pin file, unused.
- The `vdd0v9_sys` oddity: the name says 0.9 V and the vendor value is min = max = 3.3 V (`dtsi:419-429`). The schematic has no 0V9 net (text search of the one-page sheet: none), so the rail's voltage cannot be checked against it. We keep the vendor's 3.3 V, marked UNVERIFIED in the dts, and nothing in our dts consumes it.
- Gap for bring-up: the vendor SPL is closed and may program the RK801 before the kernel runs. Our open SPL does not. The SD card's 3.3 V comes from `vcc3v3_sys` (DCDC2). Whether DCDC2 is on at reset without SPL help is UNVERIFIED on the bench.

### Our U-Boot port against the vendor's voltage registers

- Our SPL writes `VCCIO2_IOC` at 0x40, 0x44 and 0x320 and `PMUIO0_IOC` at 0x4 (the SD iomux), plus the SGRF and firewall writes in `arch_cpu_init`. None is a `*_vsel` register (the vendor's are at 0x900 and up). Our port has no FSPI drive-strength read: it is not needed for SD boot.

### Console, SD, Ethernet, blobs

- **Console:** UART0 on M0 (`uart0m0_xfer_pins`, `rv1126b-pinctrl.dtsi` 2038-2045). Vendor U-Boot `stdout-path = &uart0` (`uboot/rv1126b-u-boot.dtsi` 19-20), `CONFIG_DEBUG_UART_BASE=0x20810000` (`rv1126b_defconfig` 161-163), baud 1500000 (line 160). Our UART2 choice was wrong and is changed in the U-Boot and kernel patches. The schematic's DEBUG nets are named `NLUART20TX0M0` / `RX0M0`, which read as UART0 TX/RX on M0. Pads still to confirm on the bench.
- **SD:** `sdmmc0` 4-bit, 52 MHz; the vendor U-Boot uses the same bus4, cmd, clk and detn groups as ours. Card power: the vendor kernel enables `sdmmc0_pwren` on gpio0 PB2 (`dts:96-101`), but the vendor SPL toggles GPIO0 B0 (`uboot/rv1126b.c:204-208`, bit 8). The vendor's own two sources disagree; our SPL copies the vendor SPL. UNVERIFIED on the bench.
- **Ethernet:** `phy-mode = "rgmii"` with `tx_delay = <0x20>` and no rx_delay; the RX delay comes from the PHY strap (`rv1126b-luckfox-aura.dtsi:305-330`). PHY `ethernet-phy-ieee802.3-c22` at address 1 (`dtsi:332-338`). Reset `snps,reset-gpio = gpio6 PB6`, active low, 0 / 20 ms / 100 ms (`dtsi:309-312`). The 25 MHz clock is supplied by the SoC (`assigned-clocks = CLK_MAC_OUT2IO`, 25 MHz; `dtsi:314-315`; eth_clk_25m_out_m0 on gpio6 PC1), so the earlier note that the PHY has its own crystal is wrong for the vendor design. The schematic's RXDLY reading (R233 pulled to VCCIO_PHY1) is the same as the vendor's strap intent. The part is still not legible on the render.
- **eMMC:** the vendor Aura boots from eMMC (`&emmc` enabled, non-removable, `rv1126b-luckfox-aura.dts:55-64`). Our bench image is SD boot as ruled; the eMMC is not described.
- **DDR and SPL blobs:** the vendor loader ini names `rv1126b_ddr_1332MHz_v1.06.bin` and `rv1126b_spl_ipc_v1.05.bin` (`aura/rkbin/RV1126BMINIALL_IPC.ini` lines 8, 17, 18). Our pin is rkbin 3e288fe with `rv1126b_ddr_1332MHz_v1.10.bin`. We do not switch: the v1.06 binaries are not in the SDK extract (only the ini), so their hashes cannot be checked, and the vendor rkbin rev 094fb0b6 is not fetched. Pinning it needs a download, which is not cleared. The vendor's U-Boot defconfig also sets `CONFIG_ROCKCHIP_EMMC_IOMUX` (an eMMC boot), not SD.

### Still open (bench)

PMIC power-on with the closed SPL; the SD card-power GPIO (PB2 or B0); the console pads (UART0 M0); the PHY strap levels and the part; the `vdd0v9_sys` value; the ramoops address; the blob versions.
