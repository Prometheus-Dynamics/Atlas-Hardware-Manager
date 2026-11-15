# C1: the Cubie A7S bench image (Allwinner A733, sun60i), slot form

**Group:** C. Bench images (the coordinator's 2026-11-14 ruling: every bench image is a slot image, because each board is flashed once ever).

**Merge order:** C1 CONTAINS Z1 (pr/Z1 b787017c, merged into this branch as fa0b49eb, for `lemnos0-addressing` and the shared PC-key strip). **Z1 merges first**; then C1 is rebased or merged onto it.

**Status:** DRAFT. Planned, not built, untested on board. This PR may NOT merge while its product pins the unmerged Lemnos branch `pr/BB1b` (e2e45ce7); the pin moves to the merged rev before merge.

**Runs:** nowhere in this PR. No image is built (Gaia builds images, not this PR). No board is reached: no SSH, no flash, no boot. Mark everything untested on board.

**Needs:** Gaia `next` eae34d7 (board patch series: `[kernel] patches`, `[bootloader] patches`, `[bootloader.trusted_firmware] patches`); Lemnos `pr/BB1b` e2e45ce7 (the bench-board slot declarations; the pin follows the branch head until it merges).

## What is built here (data only)

- `devices/products/cubie-a7s-bench/`: the product `cubie-a7s-bench`, Gaia F8 form, recipe `slot-boot` (the Raze's shape). Components: five product-local Lemnos copies pinned to e2e45ce7 (`components/lemnos`, `lemnosd`, `lemnos-ctl`, `lemnos-orion`, `lemnos-slot-init`); `cubie-lemnos-board` (board file, bench trust line, the generator, bundle and boot-partition blocks); `bench-pc-ssh-key` (the PC key as the `pc-key` input, dropbear key only); `bench-network` (Ethernet DHCP, usb0 excluded, mDNS through systemd-resolved); `usb-gadget-family` (the board id, `radxa-cubie-a7s`). Shared, unchanged: `orion`, `usb-gadget-network`.
- `devices/families/sun60i/`: `family.toml` declares the patch series (the kernel directory, the U-Boot and TF-A directories of `a733-bootchain`). `assets/linux.fragment`: PSTORE, PSTORE_RAM, PSTORE_CONSOLE built in (`=y`), USB FunctionFS. `assets/u-boot.fragment`: the FAT environment off, the SD environment index 0.
- `devices/boards/radxa-cubie-a7s/`: `board.toml` names the Lemnos board file (`[descriptions] lemnos`), the kernel command line (`console=ttyS0,115200`), and the complete partition list (`u-boot` at 128 KiB, `uboot-env` at 16 MiB, `boot`, `data`). `lemnos-board.toml` (new): the A7S's Lemnos board file, with the r_wdt watchdog, the U-Boot slot record and security off.
- `devices/products/cubie-a7s-bench/gaia.lock`: written by `gaia lock` at the eae34d7 binary (see Proof).

## The required specifics, each with its source

1. **Kernel: pstore built in.** `CONFIG_PSTORE=y`, `CONFIG_PSTORE_RAM=y`, `CONFIG_PSTORE_CONSOLE=y` in `families/sun60i/assets/linux.fragment`. Source: Lemnos BB3 requires them built in (INPUTS, 2026-11-14 coordinator, "BB3"). The fragment is applied after the defconfig, so these override any `=m`. The C1 brief says the a7s defconfig has `PSTORE_RAM=m`; this family starts from arm64's `defconfig` (family.toml `defconfig = "defconfig"`), so the `=m` is in a file this build does not read. Not checked against a kernel tree (none on disk).
2. **Ramoops node.** The series' own node, `arm64-dts-allwinner-a733-cubie-a7s-add-a-ramoops-region.patch`, applied with the kernel series (family.toml `patches`). Its values, as the brief gives them: `reg <0 0x60000000 0 0x100000>`, console 0x40000, record 0x20000, ecc 16. UNVERIFIED: the patch file was not read in this PR; the values are the brief's.
3. **Watchdog.** r_wdt at `/soc@0/watchdog@7021000` (the series' `a733-soc-kern/arm64-dts-allwinner-a733-add-the-watchdogs.patch`, lines 43-50; Lemnos's `socs/sun60i-a733.toml` `[controllers.watchdog]` gives the same path). WDT0 stays disabled (the series' own status). Lemnos arms 15 s (`config = { timeout_s = 15 }`); the sunxi driver's hardware maximum is 16 s (`drivers/watchdog/sunxi_wdt.c:25`, as Lemnos cites). lemnosd feeds it (`keepalive = true`). No systemd `RuntimeWatchdogSec` (one opener rule, INPUTS).
4. **U-Boot environment on GPT, not FAT.** `# CONFIG_ENV_IS_IN_FAT is not set` in `assets/u-boot.fragment`. Source: Radxa U-Boot 9da4b3e8 `board/sunxi/board.c:155-158` reads the FAT `uboot.env` before the GPT `uboot-env` on MMC boots (as Lemnos cites it, `testing/boards/sun60i-a733-slots.toml`). The GPT partition is `uboot-env`, type `3de21764-95bd-54bd-a5c3-4abe786f38a8`, 128 KiB at 16 MiB (family.toml, board.toml). The image never carries a `uboot.env`: the boot partition is written by Lemnos's `boot-partition` tool, which writes none, and Lemnos's writer refuses while one exists. UNVERIFIED on a board.
5. **Pins.** Gaia eae34d7 (`gaia --version`: `gaia 2.0.6-dev (eae34d7f...)`, the binary at `/mnt/ssd-samsung/atlas-next-scratch-gaia/bin-eae34d7/gaia`). Lemnos `pr/BB1b` e2e45ce7 in the five product-local Lemnos components and the board component. `devices/gaia.lock` and the root `Cargo.toml` are NOT changed.
6. **Reflash path.** `crates/flash/fel/records/sun60i-a733.json` (next already has it) is not changed; the FEL route is the family's `[flash] methods = ["fel", "sd-card"]`. The A733 FEL record is refused until acknowledged (family.toml, "FEL"). Hands-free reflash for the family is the coordinator's plan (INPUTS: "a hands-free reflash path per family ... Allwinner FEL"); not built in this PR.

## The bench spec, item by item

- **SD boot.** The boot ROM reads the SPL at 128 KiB; `[flash] methods` names fel and sd-card (family.toml).
- **Ethernet + DHCP + mDNS.** `components/bench-network`: `Type=ether`, `Name=!usb0`, `DHCP=yes`, `MulticastDNS=yes`; `systemd-resolved` with `MulticastDNS=yes`. Driver: `CONFIG_STMMAC_ETH`, `CONFIG_DWMAC_SUN55I` (family linux.fragment). UNVERIFIED on a board; the Ethernet driver's presence in mainline is per the series (gap check per board not done here).
- **USB gadget network on the device port: Type-C J16 (the board's USB-C 1; the 5 V input).** The MUSB controller is `usb@4100000` (USB0), `dr_mode = "peripheral"`, with no VBUS supply (series fe0a8ed, patches/a733-usb0-gadget; the series is checked to list those four patches, lines 516-521). J4 is a USB 3 host and not used. Kernel: `CONFIG_USB_MUSB_HDRC`, `CONFIG_USB_MUSB_SUNXI`, `CONFIG_USB_MUSB_GADGET`, `CONFIG_PHY_SUN4I_USB`, `CONFIG_USB_CONFIGFS_NCM` (linux.fragment). Gaia's configfs NCM (shared `usb-gadget-network`, its own pin b8585a0), board id `radxa-cubie-a7s` (`usb-gadget-family`). **Addressing: the Raze/ZERO 3E serial-hash-v1 scheme is on this product** (`components/lemnos0-addressing`, from pr/Z1, merged here; it is listed in `product.toml`). CAVEAT: that component addresses the `lemnos0` NCM interface, which only the Lemnos gadget creates. This board's Lemnos socs map has no device port, so no Lemnos gadget is declared and Gaia's NCM comes up as `usb0`, which serial-hash-v1 does not address. So on this A7S the scheme addresses nothing until the MUSB controller enters the Lemnos socs map (OPEN, Lemnos side). Until then Atlas reaches the board over `usb0` at Gaia's board-id address (10.55.x), not by serial-hash-v1. The Lemnos gadget (`udc:` device) is NOT declared: Lemnos's `socs/sun60i-a733.toml` says the map has no device port. OPEN: the Lemnos side needs the MUSB controller in the map (the bring-up dts names it `usb@4100000`, B19), and the Orion link over USB waits for that.
- **Hostname from the serial.** NOT done here (pr/Z1's zero-3e-bench has a serial hostname service, not carried for the A7S). The static hostname is `cubie-a7s`. The serial script is on `pr/Z1`; no program is carried here (TODO).
- **SSH with the PC key (comment stripped).** `components/bench-pc-ssh-key`: `{ input = "pc-key" }` installed as root's `authorized_keys`, dropbear `-s` (key only). The comment strip is `scripts/pc-key-strip.sh`, which exists on disk in the Z1 worktree (untracked) and is wired by `build-product.sh --pc-key` on `pr/Z1`. Neither is on `origin/next`. `scripts/check-devices.sh` on next supplies only `bench-key`, so this product's device step fails until `pc-key` is supplied there (see the Proof section). I did not edit `scripts/` (the rules forbid it).
- **Lemnos (lemnosd) + Orion, security off.** Lemnos components at e2e45ce7; `[security] orion = false` and no record kept in the board file (the coordinator's ruling 2026-11-14, "security off"). Orion: the shared component at 455a956; its config `orion.conf` has no TCP way in (INPUTS, milestone-2 blocker B). Not fixed here.
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
4. **`uboot_env_size`** is `0x8000` (the family's `CONFIG_ENV_SIZE`, and the BB1b sample's). BB1b's RK change to `0x10000` is RK-only at e2e45ce7 (INPUTS, 2026-11-14). Whether the A733 needs the same change is not known: it is a Lemnos/Gaia question.
5. **The Lemnos board file is not validated.** `lemnos-ctl validate --socs ... radxa-cubie-a7s` and `lemnos-ctl tree` at e2e45ce7 were not run (no build in this PR). The generator's flag names are the Raze's; UNVERIFIED at BB1b.
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
