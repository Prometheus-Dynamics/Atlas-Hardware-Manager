# C1: the Cubie A7S bench image (Allwinner A733, sun60i), slot form

**Group:** C. Bench images (the coordinator's 2026-11-14 ruling: every bench image is a slot image, because each board is flashed once ever).

**Merge order:** C1 CONTAINS Z1 (pr/Z1 b787017c, merged into this branch as fa0b49eb, for `lemnos0-addressing` and the shared PC-key strip). **Z1 merges first**; then C1 is rebased or merged onto it.

**Status:** DRAFT. Planned, not built, untested on board. This PR may NOT merge while its product pins the unmerged Lemnos branch `pr/BB1b` (e8472620); the pin moves to the merged rev before merge.

**Runs:** nowhere in this PR. No image is built (Gaia builds images, not this PR). No board is reached: no SSH, no flash, no boot. Mark everything untested on board.

**Needs:** Gaia `next` c0d084f (board patch series: `[kernel] patches`, `[bootloader] patches`, `[bootloader.trusted_firmware] patches`); Lemnos `pr/BB1b` e8472620 (the bench-board slot declarations; the pin follows the branch head until it merges).

## What is built here (data only)

- `devices/products/cubie-a7s-bench/`: the product `cubie-a7s-bench`, Gaia F8 form, recipe `slot-boot` (the Raze's shape). Components: five product-local Lemnos copies pinned to e8472620 (`components/lemnos`, `lemnosd`, `lemnos-ctl`, `lemnos-orion`, `lemnos-slot-init`); `cubie-lemnos-board` (board file, bench trust line, the generator, bundle and boot-partition blocks); `bench-pc-ssh-key` (the PC key as the `pc-key` input, dropbear key only); `bench-network` (Ethernet DHCP, usb0 excluded, mDNS through systemd-resolved); `usb-gadget-family` (the board id, `radxa-cubie-a7s`). Shared, unchanged: `orion`, `usb-gadget-network`.
- `devices/families/sun60i/`: `family.toml` declares the patch series (the kernel directory, the U-Boot and TF-A directories of `a733-bootchain`). `assets/linux.fragment`: PSTORE, PSTORE_RAM, PSTORE_CONSOLE built in (`=y`), USB FunctionFS. `assets/u-boot.fragment`: the FAT environment off, the SD environment index 0.
- `devices/boards/radxa-cubie-a7s/`: `board.toml` names the Lemnos board file (`[descriptions] lemnos`), the kernel command line (`console=ttyS0,115200`), and the complete partition list (`u-boot` at 128 KiB, `uboot-env` at 16 MiB, `boot`, `data`). `lemnos-board.toml` (new): the A7S's Lemnos board file, with the r_wdt watchdog, the U-Boot slot record and security off.
- `devices/products/cubie-a7s-bench/gaia.lock`: written by `gaia lock` at the eae34d7 binary (see Proof).

## The required specifics, each with its source

1. **Kernel: pstore built in.** `CONFIG_PSTORE=y`, `CONFIG_PSTORE_RAM=y`, `CONFIG_PSTORE_CONSOLE=y` in `families/sun60i/assets/linux.fragment`. Source: Lemnos BB3 requires them built in (INPUTS, 2026-11-14 coordinator, "BB3"). The fragment is applied after the defconfig, so these override any `=m`. The C1 brief says the a7s defconfig has `PSTORE_RAM=m`; this family starts from arm64's `defconfig` (family.toml `defconfig = "defconfig"`), so the `=m` is in a file this build does not read. Not checked against a kernel tree (none on disk).
2. **Ramoops node.** The series' own node, `arm64-dts-allwinner-a733-cubie-a7s-add-a-ramoops-region.patch`, applied with the kernel series (family.toml `patches`). Its values, as the brief gives them: `reg <0 0x60000000 0 0x100000>`, console 0x40000, record 0x20000, ecc 16. UNVERIFIED: the patch file was not read in this PR; the values are the brief's.
3. **Watchdog.** r_wdt at `/soc@0/watchdog@7021000` (the series' `a733-soc-kern/arm64-dts-allwinner-a733-add-the-watchdogs.patch`, lines 43-50; Lemnos's `socs/sun60i-a733.toml` `[controllers.watchdog]` gives the same path). WDT0 stays disabled (the series' own status). Lemnos arms 15 s (`config = { timeout_s = 15 }`); the sunxi driver's hardware maximum is 16 s (`drivers/watchdog/sunxi_wdt.c:25`, as Lemnos cites). lemnosd feeds it (`keepalive = true`). No systemd `RuntimeWatchdogSec` (one opener rule, INPUTS).
4. **U-Boot environment on GPT, not FAT.** `# CONFIG_ENV_IS_IN_FAT is not set` in `assets/u-boot.fragment`. Source: Radxa U-Boot 9da4b3e8 `board/sunxi/board.c:155-158` reads the FAT `uboot.env` before the GPT `uboot-env` on MMC boots (as Lemnos cites it, `testing/boards/sun60i-a733-slots.toml`). The GPT partition is `uboot-env`, type `3de21764-95bd-54bd-a5c3-4abe786f38a8`, 128 KiB at 16 MiB (family.toml, board.toml). The image never carries a `uboot.env`: the boot partition is written by Lemnos's `boot-partition` tool, which writes none, and Lemnos's writer refuses while one exists. UNVERIFIED on a board.
5. **Pins.** Gaia c0d084f (merged next: the eae34d7 patch series and slot-disk-blobs; `gaia 2.0.6-dev (c0d084fa...)`, the binary at `/mnt/ssd-samsung/atlas-next-scratch-gaia/bin-c0d084f/gaia`). Lemnos `pr/BB1b` e8472620 in the five product-local Lemnos components and the board component. `devices/gaia.lock` and the root `Cargo.toml` are NOT changed.
6. **Reflash path.** `crates/flash/fel/records/sun60i-a733.json` (next already has it) is not changed; the FEL route is the family's `[flash] methods = ["fel", "sd-card"]`. The A733 FEL record is refused until acknowledged (family.toml, "FEL"). Hands-free reflash for the family is the coordinator's plan (INPUTS: "a hands-free reflash path per family ... Allwinner FEL"); not built in this PR.

## The bench spec, item by item

- **SD boot.** The boot ROM reads the SPL at 128 KiB; `[flash] methods` names fel and sd-card (family.toml).
- **Ethernet + DHCP + mDNS.** `components/bench-network`: `Type=ether`, `Name=!usb0`, `DHCP=yes`, `MulticastDNS=yes`; `systemd-resolved` with `MulticastDNS=yes`. Driver: `CONFIG_STMMAC_ETH`, `CONFIG_DWMAC_SUN55I` (family linux.fragment). UNVERIFIED on a board; the Ethernet driver's presence in mainline is per the series (gap check per board not done here).
- **USB gadget network on the device port: Type-C J16 (the board's USB-C 1; the 5 V input).** The MUSB controller is `usb@4100000` (USB0), `dr_mode = "peripheral"` (series fe0a8ed, patches/a733-usb0-gadget). J4 is a USB 3 host and not used. Kernel: `CONFIG_USB_MUSB_HDRC`, `CONFIG_USB_MUSB_SUNXI`, `CONFIG_USB_MUSB_GADGET`, `CONFIG_PHY_SUN4I_USB`, `CONFIG_USB_CONFIGFS_NCM` (linux.fragment). The gadget is Lemnos's (`udc:4100000.usb`, controller `usb-otg0`, Lemnos pr/BB1b e8472620); Gaia's `usb-gadget-network` and `usb-gadget-family` are not in the image (one gadget per UDC). Its interface is `lemnos0`, addressed by `components/lemnos0-addressing` (serial-hash-v1). VBUS: `vbus_undriven = true` (coordinator ruling 2026-11-14): Radxa Cubie A7S schematic V1.10 sheet 13: J16 VBUS is net VCC5V0, which only feeds load switch U12 (SGM40666AS) into VCC5V0_SYS; nothing on the board drives it; USB0-DRVVBUS switches J4 (sheet 12). Reverse blocking through U12 is not proven from the sheet; the bench is powered from J16 only. Unconfirmed on a board: bus path, UDC name, PHY. The Orion link over the gadget is not wired yet.
- **Hostname from the serial.** NOT done here (pr/Z1's zero-3e-bench has a serial hostname service, not carried for the A7S). The static hostname is `cubie-a7s`. The serial script is on `pr/Z1`; no program is carried here (TODO).
- **SSH with the PC key (comment stripped).** `components/bench-pc-ssh-key`: `{ input = "pc-key" }` installed as root's `authorized_keys`, dropbear `-s` (key only). The comment strip is `scripts/pc-key-strip.sh`, which exists on disk in the Z1 worktree (untracked) and is wired by `build-product.sh --pc-key` on `pr/Z1`. Neither is on `origin/next`. `scripts/check-devices.sh` on next supplies only `bench-key`, so this product's device step fails until `pc-key` is supplied there (see the Proof section). I did not edit `scripts/` (the rules forbid it).
- **Lemnos (lemnosd) + Orion, security off.** Lemnos components at e8472620; `[security] orion = false` and no record kept in the board file (the coordinator's ruling 2026-11-14, "security off"). Orion: the shared component at 455a956; its config `orion.conf` has no TCP way in (INPUTS, milestone-2 blocker B). Not fixed here.
- **Watchdog armed.** lemnosd's hardware watchdog, 15 s (above).
- **Serial console.** The kernel console is `ttyS0` (board.toml `cmdline`). systemd starts a getty on it; root login with no password while security is off (`os.fragment`, the bench rule). UNVERIFIED on a board.
- **Crash log.** The kernel's pstore/ramoops (above). The reader is Lemnos BB3, which is NOT on the pin; so no crash log is read by Lemnos here.

## Left behind or not done (and why)

| Item | Where dev / Raze had it | Why not here |
|---|---|---|
| `lemnos0-addressing` (serial-hash-v1) | `pr/Z1`, `devices/components/raze-base-files` on next | Not on origin/next: copied nothing (brief). TODO above. |
| Raze's `raze-base-files` (journald, ttyGS0 getty, security console gate) | next | Raze-specific: the ttyGS0 console is the Raze's, and the security-console gate needs Lemnos S2 files not in this product. |
| Raze's post-build D-Bus trim | `products/raze-base/os/post-build.sh` | Not carried: the trim is a program that runs on the image build (its test ran on Raze's tree). D-Bus stays in this image. TODO. |
| `lemnos-orion-links.conf` (LEMNOS_ORION_LINKS) | `components/lemnos` (Raze) | No gadget device in the A7S board file, so no link to name. |
| `raze-lemnos-board` i2c-dev and USB power rules | `components/raze-lemnos-board` | Raze hardware (i2c buses, the Pi's USB ports). |
| bring-up watchdog and LED heartbeat (`bringup-alive`) | the bring-up product | The bring-up sets systemd `RuntimeWatchdogSec`; lemnosd is the one opener here. |
| dev bring-up host key on /data (`dev-bringup-hostkey`) | the bring-up product | Dev-only. Dropbear keeps its key in RAM here: a new host key each boot (TODO). |
| Lemnos's `pstore` reader (BB3), the BB2 episode count, the `pr/BB2` branch | Lemnos | Not on the pin: crash logs and episode counting wait for that rev. |
| Board's fastboot pair `1209:0001` | `boards.md` | Kept as the pid.codes TEST pair (Lemnos marks it test); replaced before any release. |
| The eMMC boot source | the BB1b sample | The boot ROM's eMMC support is UNVERIFIED; the bench uses SD only. |

## Open items

1. **boot_size is a placeholder.** 82,837,504 (the Raze's FAT size) is copied, not measured. The board and the product must agree (the check refuses two numbers). Set both from `lemnos-ctl boot-partition --layout-of radxa-cubie-a7s --min-size`, which needs Lemnos's tool built (not run here).
2. **The `boot` partition type** is the EFI System GUID (board.toml). F8?: Gaia's spelling for a `boot` role on GPT is not confirmed.
3. **The `pc-key` input** is not supplied by `scripts/check-devices.sh` on next (Z1 adds it). Until then the device step fails on this product by name.
4. **`uboot_env_size`** is `0x8000` (the family's `CONFIG_ENV_SIZE`, and the BB1b sample's). BB1b's RK change to `0x10000` is RK-only at e8472620 (INPUTS, 2026-11-14). Whether the A733 needs the same change is not known: it is a Lemnos/Gaia question.
5. **The Lemnos board file is not validated.** `lemnos-ctl validate --socs ... radxa-cubie-a7s` and `lemnos-ctl tree` at e8472620 were not run (no build in this PR). The generator's flag names are the Raze's; UNVERIFIED at BB1b.
6. **Gaia's `[kernel] patches` form** is taken from Gaia eae34d7's `crates/core/describe/src/board.rs` (`patches: Option<Vec<FileRef>>`, `FileRef::InSource { source, path }`). The product validates (below); the planned patch paths (`patches`, `patches/u-boot`, `patches/tf-a`) are from the family's own comments and the INPUTS note, not from the series repos (not fetched here).
7. **`devices/gaia.lock` and the root `Cargo.toml`** are not changed. Moving the Gaia pin to eae34d7 is the coordinator's step.
8. **Merge.** Do not merge while `pr/BB1b` is unmerged. Move the five components and the board component to the merged Lemnos rev before merge.

## Proof

- `gaia validate` at eae34d7 (no lock, no network):
  - `ok  product  devices/products/cubie-a7s-bench/product.toml`
  - `ok  board  devices/boards/radxa-cubie-a7s/board.toml`
  - `ok  family  devices/families/sun60i/family.toml`
- `gaia lock` and `gaia plan`: see the PR text (pasted from the real run).
- The gate: see the report; it is not claimed to pass.

## PL2 (USB0-DRVVBUS): J4's VBUS, no hog (coordinator ruling)

PL2 switches J4, the USB 3 host Type-C, which the product image needs for USB cameras. No output-low hog is written (cancelled by the coordinator; the earlier hog plan is withdrawn).

Schematic V1.10 (dl-a7s-schematic/radxa_cubie_a7s_schematic_v1.10.pdf): sheet 7 (GPIO) puts PL2 on the pin with function list `PL2/S-UART1-TX/S-UART0-TX/S-TWI1-SDA/S-PWM0-0/PL-EINT2`; sheet 12 (TYPE C) routes `USB0-DRVVBUS` through R79 (0 R) to the EN of U2, SGM2576YN5G_TR, whose output is `VBUS_USB` on J4; sheet 13 (USBC0_T-CARD) puts J16's VBUS pins on `VCC5V0`, undriven (the input), so J16 stays `vbus_undriven`.

The vendor regulator (DockSeed series fe0a8ed, patches/a733-a7s-usbc/arm64-dts-allwinner-a733-cubie-a7s-enable-the-usb-c-port.patch, lines 45-53 of the hunk at `@@ -67,6 +67,27 @@`):

    reg_usbc_vbus: regulator-usbc-vbus {
        compatible = "regulator-fixed";
        regulator-name = "usbc-vbus";
        regulator-min-microvolt = <5000000>;
        regulator-max-microvolt = <5000000>;
        gpio = <&r_pio 0 2 GPIO_ACTIVE_HIGH>; /* PL2 */
        enable-active-high;
    };

- regulator-always-on: NOT set on this node. Its later hunks (a7s-5v-sys, a7s-usb-hub, dp) add only `vin-supply`.
- regulator-boot-on: NOT set anywhere in the series (grep of patches/ finds none).
- Consumer: the HUSB311 Type-C controller, `vbus-supply = <&reg_usbc_vbus>` (the typec@4e node, same patch, `data-role = "host"`, `power-role = "source"`). The regulator is enabled only when the controller asks for VBUS.
- The series' usb1 VBUS is a different regulator: `usb1_vbus-supply = <&reg_usb1_vbus>` (on PM5, not PL2). The brief's "reg_usb_vbus on PL2 as usb1's vbus-supply" does not match the series; PL2 belongs to reg_usbc_vbus, the J4 controller's supply.

Result: nothing to remove. The image builds the whole series (the family's `patches` lists the full directory), and neither property is set on the node, so no patch is written here.

Step 5 (apply with --fuzz=0): not run here, because the applied kernel tree is not on disk; Gaia's strict build is the apply check.

## Watchdog: what arms it, the timeout, and the first-boot measurements (C1 ruling 2026-11-16)

Kernel fragment: `CONFIG_WATCHDOG_OPEN_TIMEOUT=30` beside `CONFIG_SUNXI_WATCHDOG=y` (devices/families/sun60i/assets/linux.fragment). Reason: Linux's `WATCHDOG_HANDLE_BOOT_ENABLED` (default y) feeds a bootloader-armed watchdog until userspace opens it; without an open timeout that is forever, so a boot that never reaches the opener never resets and a trial never rolls back. 30 s covers driver probe to slot-init's open. Value is Lemnos's, for every slot family until a board measures it (this PR changes only the sun60i fragment; Raze and rk35xx are their owners').

**(a) What arms r_wdt (WDT_CPUS, `/soc@0/watchdog@7021000`) before Linux**
- U-Boot, by the DockSeed series: `patches/u-boot/sunxi-cubie-a7s-watchdog-autostart.patch` sets `CONFIG_WDT`, `CONFIG_WDT_SUNXI`, `CONFIG_WATCHDOG`, `CONFIG_WATCHDOG_AUTOSTART` (patch lines 55-58); its header says U-Boot starts the 16 s watchdog (`CONFIG_WATCHDOG_TIMEOUT_MSECS` default 16000 for ARCH_SUNXI). `patches/u-boot/sunxi-cubie-a7s-watchdog-on-wdt-cpus.patch` says the watchdog U-Boot arms is WDT_CPUS and WDT0 is disabled (lines 58-62), and enables the r_wdt node (lines 65-73). Series pin b6f6872b, read from its FETCH_HEAD.
- Our `devices/families/sun60i/assets/u-boot.fragment` has no watchdog line: the arming is the series' config, not ours.
- SPL and BootROM: NOT CHECKED. The autostart patch sets U-Boot's config, not SPL's; nothing in our data says SPL arms a watchdog.
- Note: the autostart patch's own text names WDT0 (watchdog@2050000) in its DT context; the wdt-cpus patch says WDT_CPUS is the one. The wdt-cpus patch is the later and the one that disables WDT0; read as WDT_CPUS. UNVERIFIED on a board.

**(b) The boot path to the opener**
- The image boots through Lemnos's slot-init as the initramfs `/init` (product.toml `initramfs = ["lemnos-slot-init"]`; component installs the binary at `/init`). Read at 31e0f736: `service/slot-init/src/boot.rs:186` calls `ops.arm_watchdog()` right after devtmpfs mounts. `arm_watchdog` (`sys.rs` ~217) calls `arm()` (`sys.rs:60-82`): open `<dev>/watchdog` for write (this arms it), `set_timeout(15)`, read back. Feeding: `sys.rs:46` `feed()`, called from `sleep()` (`sys.rs` ~206-214, every 5 s while a wait runs) and the feed points at `sys.rs:112`. Whether the feed runs continuously until `switch_root` is NOT fully CHECKED: `switch_root` is `sys.rs` (`fn switch_root`), and the feeding stops only at `main.rs:26` (`stop_feeding`) after `boot` returns.
- Then the root's `lemnosd` binds the `hw-watchdog` device (board file `[[devices]] driver = "hw-watchdog"`, `uses = { watchdog = "watchdog:/soc@0/watchdog@7021000" }`, `config = { timeout_s = 15 }`, keepalive). Read at 31e0f736: `drivers/hw-watchdog/src/lib.rs` (header lines 37-42, 45 "Binding opens the node for writing, which arms the watchdog with whatever timeout it has", 50 feed interval half the applied timeout). The driver clamps to sysfs `min_timeout`/`max_timeout` when present (not verified on the A7S: the DockSeed watchdog patch replaces `max_timeout` with `max_hw_heartbeat_ms`, so sysfs may not show it).

**(c) The sunxi hardware maximum and what 15 s becomes**
- Hardware maximum 16 s: the Lemnos board note cites `drivers/watchdog/sunxi_wdt.c:25` (as they cite it; read in the Lemnos repo, not the kernel tree here).
- Rounding, read from the DockSeed kernel patch `patches/sunxi-wdt-heartbeat/watchdog-sunxi_wdt-let-the-watchdog-core-serve-timeouts-above-16-s.patch` (in series, line 543): `sunxi_wdt_set_timeout()` does `hw_timeout = min(timeout, WDT_MAX_TIMEOUT)`, then `if (wdt_timeout_map[hw_timeout] == 0) hw_timeout++`. A value with no entry in `wdt_timeout_map[]` is bumped to the next index.
- The map itself is NOT on disk (not in the series, Lemnos, or Gaia). So: whether 15 is in the table is NOT CHECKED. If it is not, 15 becomes 16.
- Consequence, if 15 → 16: Lemnos's slot-init `arm()` reads back 16 and returns "not within 15 s" (sys.rs `arm`), logs `slot-init: NO WATCHDOG`, drops the file, and leaves the device unfed by slot-init. The kernel core then feeds until the 30 s open timeout, and the hardware resets 16 s after the last feed. So a boot would reset rather than hang, but the slot-init stage is not fed, and the trial rollback depends on that. Lemnos is fixing the refusal on their side (coordinator).
- **KNOWN BLOCKER for a first A7S boot, pending: read `wdt_timeout_map[]` from the pinned 7.3.4 sunxi_wdt.c (with the DockSeed patch) before the first boot. If 15 is not in the table, do not boot until the Lemnos fix that accepts 16 is pinned (or the slot-init timeout is changed to a value the table holds).**

**First-boot measurement list (A7S; all UNVERIFIED until measured)**
1. Bootloader's last feed (U-Boot's WDT_CPUS, 16 s) to driver probe: from the boot log timestamps (`sunxi_wdt` probe line).
2. Driver probe to slot-init's open: the kernel log's probe time to slot-init's `watchdog armed` line.
3. slot-init's last feed to lemnosd's open at bind to first feed: slot-init's final feed (`stop_feeding` in `main.rs:26`, or the last `feed()`), lemnosd's bind line, and the first feed after it.
4. Bind to running-ok: lemnosd's `running` state in `lemnos-ctl list` (the hw-watchdog device's health).
5. The applied timeout read back at each open: slot-init's `watchdog armed: N s` (or its `NO WATCHDOG` line), and lemnosd's applied value at bind.
6. Whether `NO WATCHDOG` was logged at any step (slot-init `arm()` error path).

## Data partition sizing, keep, reserve, and the known limit (C1, 2026-11-16)

Sizing (board.toml comment has the same arithmetic): medium the A7S SD card, smallest card supported 16 GB PROPOSED (not stated in the tree; flagged for the coordinator's ruling). Fixed areas below the data partition take under 81 MiB. Slot bound S = 64 MiB, a planning bound (the Raze's measured EROFS root is 18,178,048 bytes; the ZERO 3E estimates 18 to 40 MB; the A7S root is not measured). Rule: data >= 1.10 x (keep x S + reserve_mb). With keep = 2 and reserve_mb = 128 MiB that is 281.6 MiB minimum. Chosen: 1 GiB (1073741824): above the minimum, the same as the ZERO 3E bench (Z2), and room for keep = 3 (352 MiB) plus ext4 overhead.

Settings, on the system-slots device (lemnos-board.toml): keep = 2 (Lemnos default 3, store.rs:32; two is the minimum that takes an update, three would cost one more slot of data); reserve_mb = 128 (Lemnos default 2048, lib.rs:82, which would refuse every install on a 1 GiB partition; the 64 to 128 MiB Lemnos suggests); grow_data = false (lib.rs:1344). The ZERO 3E (Z2) sets neither keep nor reserve_mb yet, so the rk35xx bench values are not yet stated: these are the A7S numbers for the coordinator to align.

UNCONFIRMED (Lemnos): whether a pushed bundle is spooled on the data partition; if it is, it adds a term to the rule.

**Known limit: the data partition does not grow on GPT boards until Lemnos GROW.** The grow is MBR-only at 31e0f736 (drivers/system-slots/src/grow.rs:3 and :33, `mbr::plan_grow`), so on this GPT disk it would refuse every pass. grow_data = false stops its 8 s refusal loop. Remove when Lemnos says.

Watchdog owner check (scripts/check-watchdog-owner.sh, next c2482fb9, run on 5ab70763): no FINDING. The lines for cubie-a7s-bench are:
- NOTE product cubie-a7s-bench: inputs say Lemnos; built config unverified
- NOTE product cubie-a7s-bench: bootloader arming: armed (Lemnos slot init in the initramfs arms the watchdog (its timeout is in Lemnos, not in this tree) at products/cubie-a7s-bench/product.toml line 62); fed by Lemnos
- NOTE product cubie-a7s-bench: bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)

Reading: one owner, Lemnos (the hw-watchdog keepalive device plus lemnosd). The first arming line is the initramfs slot-init. The U-Boot line is NOT CHECKED: the autostart line (CONFIG_WATCHDOG_AUTOSTART=y) is in the DockSeed series patch sunxi-cubie-a7s-watchdog-autostart.patch, not in this tree's explicit lines, so the check cannot see it. The expected "armed by U-Boot" is therefore the series' claim, not the check's result.

## Enforced data-size rule and layout table (supersedes the 64 MiB planning bound above)

Rule (Z2's data_size_rule, check-devices.sh; Lemnos enforces it at install.rs:698 at 31e0f736): minimum data = 1.10 x (keep x bay_set + reserve_mb + journal allowance 64 MiB). bay_set = the sum of the five `[boot]` bay sizes in lemnos-board.toml. Layout end = 16 MiB + uboot-env 128 KiB + boot + data + 1 MiB backup GPT, nothing after data. Smallest card: 8 GB taken as 7.4 GiB usable (7,945,689,497 B).

| Item | Value | Source |
|---|---|---|
| bay_set | 34,885,632 B (kernel 33,554,432; initramfs 1,048,576; device tree 262,144; overlay 16,384; cmdline 4,096) | lemnos-board.toml [boot], from Z2's ZERO 3E bench; UNVERIFIED on the first A7S build |
| keep | 2 | lemnos-board.toml, system-slots config (the minimum that takes an update) |
| reserve_mb | 128 MiB = 134,217,728 B | lemnos-board.toml (Lemnos default 2048) |
| journal allowance | 64 MiB = 67,108,864 B | Z2's rule |
| minimum data | 1.10 x (2 x 34,885,632 + 134,217,728 + 67,108,864) = 298,207,641 B (284.4 MiB) | the rule |
| data (declared) | 1 GiB = 1,073,741,824 B | board.toml; above the minimum; same as the ZERO 3E bench |
| boot (declared) | 128 MiB = 134,217,728 B | board.toml; product boot_size equal; three bay sets need 104,656,896 B of payload |
| uboot-env | 128 KiB at 16 MiB (16,777,216 B) | board.toml |
| layout end | 16,777,216 + 131,072 + 134,217,728 + 1,073,741,824 + 1,048,576 = 1,225,916,416 B; nothing after data | the rule's layout check |
| free on the 7.4 GiB card | 7,945,689,497 - 1,225,916,416 = 6,719,773,081 B | the rule's layout check |

Boot size changed from 79 MiB to 128 MiB: three bay sets (docs/slots.md:1054) of these bays do not fit 79 MiB. The first 79 MiB number was the Raze's, and it is a fact of the Raze's bays, not this board's.

Flags (as on Z2's board): the slot root image has no enforced cap. Whether a pushed bundle is spooled on the data partition is unknown; if it is, it adds a term to the minimum. The SD card's smallest supported size is 8 GB here (7.4 GiB usable); it is not stated in the tree, and the coordinator ruled on it.

Bay sizes: the kernel bay of 32 MiB is a guess for the A733 Image; the first A7S build's Image size checks it. An over-size kernel is refused by name at install (Lemnos install.rs:697-698), not silently broken.
