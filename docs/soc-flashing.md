# Flashing from a SoC's download mode

Atlas flashes a blank or bricked board from its SoC's USB download mode, in
Rust, with no vendor tool at run time. Status: **written, untested on a
board.** The boards are on order. Each board record says
`"status": "untested"`, and Atlas logs that before it flashes.

| Board | SoC | Mode (USB id) | Storage | Record |
|---|---|---|---|---|
| Radxa ROCK 5C | RK3588S2 | maskrom 2207:350b | eMMC | `devices/radxa-rock-5c` |
| Radxa ROCK 4D | RK3576 | maskrom 2207:350e (a guess: `pid_unconfirmed`) | eMMC | `devices/radxa-rock-4d` |
| Luckfox Pico Pro | RV1106G2 (Flame's SoC) | maskrom 2207:110c | SPI-NAND, 128 MB | `devices/luckfox-pico-pro` |
| BPI-CanMV-K230D-Zero | K230D | USB boot 29f1:0230 | microSD | `devices/bpi-canmv-k230d-zero` |

## Pieces

- `crates/atlas-socflash`, the protocols, over one small USB trait
  (`UsbIo`: vendor control requests, one bulk pair). Tests replay traces
  through `Script`, and `tests/usbplug.rs` simulates a usbplug.
  - `rockchip`, in the order rkdeveloptool runs it:
    - the loader file (471/472/loader entries, trailing CRC-32);
    - maskrom download (request 0x0C, wIndex 0x471/0x472, 4096-byte chunks,
      big-endian CRC-16);
    - rockusb commands (31-byte CBW, 13-byte CSW; LBA and count big-endian);
    - image writes read back by SHA-256;
    - the FlashHead idblock;
    - SPI-NAND through LBAs (`LbaNand`).
  - `k230`:
    - the boot ROM (CPU info, load address, 1000-byte pages, start);
    - Canaan's burner U-Boot ("kburn": 60-byte packets for probe, info,
      erase, write, SHA-256 verify, reboot).
- `crates/atlas-ubi`, UBI on raw NAND (Gaia G8):
  - reads Gaia's UBI images with every CRC checked;
  - writes them as `ubiformat -f` does, through any `RawNand`;
  - updates a board's inactive `rootfs_a`/`rootfs_b` volume with
    `UBI_IOCVOLUP` and reads it back. The `atlas-ubi` binary
    (`info`, `slot-write`) is for a NAND board's device package.
- `crates/atlas-driver-soc` holds the Rockchip and K230 drivers. They find
  boards in a download mode on this computer's USB, name them from the
  catalog's board records, and flash:
  1. **Preflight:** the record, its loader file, the image decoded to raw.
  2. **Transfer:** maskrom DDR init and usbplug, or the K230 burner. The
     board re-enumerates on the same port.
  3. **Apply:** the write, then the readback.
  4. **Confirm:** the board restarts.
- Board records are `devices/<model>/manifest.json` with
  `"record_only": true`. The schema requires only `platform` and `recovery`
  for them. `recovery.flash` names:
  - the loader glob;
  - the storage;
  - the image offset;
  - K230's load address;
  - NAND geometry.

## Whose board is it

A record whose PID is a guess (`usb_boot.pid_unconfirmed`, RK3576's today)
is offered for a Rockchip in a download mode whose PID no record names and
no chip claims. Once the usbplug runs, Atlas reads the chip number the board
reports (`READ_CHIP_INFO`) and checks it against the record's `platform.soc`:

- Another chip stops the flash before anything is written, on every board.
- A chip it can't read stops the flash only when the PID is a guess; on a
  confirmed PID it's logged and the flash goes on.

## Windows

The installer and the helper's Fix button bind WinUSB to the boot IDs:
Raspberry Pi's, and the download modes in `atlas_usbboot::SOC_BOOT_IDS`. A
test keeps the installer's list and that one together. Rockchip's loader
mode is assumed to keep its maskrom PID, as rkdeveloptool's table has it.

## Checked against real loaders

Gaia's builds (006284a) produced rkbin's `rk3588_spl_loader_v1.18.113.bin`
(ROCK 5C), `rk3576_loader_v1.13.109.bin` (ROCK 4D) and
`rv1106_download_v1.15.108.bin` (Luckfox Pico Pro).
`crates/atlas-socflash/tests/real_loaders.rs` parses any loaders named in
`ATLAS_TEST_LOADERS`. Both pass, and they settled four points:

- The header layout and entry tables are as derived. The 471 stage has two
  entries: `UsbHead`, then the DDR init.
- The trailer CRC-32 uses rkdeveloptool's polynomial, 0x04C10DB7.
- The header's chip field is the chip number in reversed ASCII ("3588",
  "1106"). Atlas refuses a loader whose chip isn't the record's SoC.
- FlashHead is the RKNS header, RC4-scrambled per 512 bytes. Its image
  table lays out the idblock: FlashData then FlashBoot on RK3588 and RV1106,
  and on RK3576 a 4 KiB head with FlashBoost first. Atlas builds the idblock
  from that table, with the head unscrambled as rkbin's idblock images and
  Gaia's carry it. It refuses a loader whose table and entries disagree.
- The loader stores every loader entry RC4-scrambled per 512 bytes, even
  with its header's RC4 flag off. The idblock carries them unscrambled, the
  head included.
- The idblock Atlas builds from rv1106_download_v1.15.108.bin is byte for
  byte Gaia's idblock.img, which is the vendor's rv1106_idblock_v1.15.102.img.
  Gaia's ROCK images carry their own idblock (RKNS, unscrambled) at sector 64
  of sdcard.img.

The K230D Zero's sdcard.img checks out too: valid K230 firmware heads (SPL)
at 1 MiB and 1.5 MiB. Atlas checks both before every K230 SD or eMMC flash
and refuses a damaged one.

## A SPI-NAND partition set

Gaia's Luckfox Pico Pro image is a set of partition images, laid out by the
`mtdparts` in its `env.img`:

| Partition | Offset | Size | Image |
|---|---|---|---|
| env | 0 | 256K | env.img |
| idblock | 256K | 256K | idblock.img |
| uboot | 512K | 512K | uboot.img |
| boot | 1M | 6M | boot.img |
| boot_b | 7M | 6M | none (empty slot) |
| ubi | 13M | the rest | ubi.img |

Atlas reads that table from the `env.img` next to the chosen file, so a
layout change in Gaia needs no Atlas change. It writes each image at its
partition's offset, erasing the eraseblocks first and reading them back. It
writes `ubi.img` as ubiformat does over the rest of the flash. A single
NAND image (loaders, then UBI at an eraseblock boundary) still works.

## The loader

Atlas never downloads a vendor loader. It looks for the record's glob in two
places: next to the image, then in the record's folder. Gaia's build of the
board fetches rkbin and the K230 SDK, so it is the natural place for the
loader to come from. The loaders are:

- rkbin's boot_merger names (`rk3588_loader_v*.bin`, `rk3576_loader_v*.bin`,
  `rv1106_download_v*.bin`) and Radxa's (`rk3588_spl_loader_v*.bin`,
  `rk3576_spl_loader_v*.bin`); a record's `loader` lists both;
- the Luckfox SDK's `download*.bin`;
- Canaan's `loader_mmc*.bin`.

Gaia's board builds copy the matching loader next to each image.

## SPI-NAND

Gaia lays a NAND image out as the loaders, then a UBI image at an
eraseblock boundary. Atlas finds that boundary by its EC headers:

- What comes before the UBI image is written as is.
- The UBI region is written as ubiformat does:
  - bad blocks are skipped;
  - erase counters are carried over;
  - trailing erased pages are never programmed;
  - the rest of the flash gets EC headers.

A flash whose good blocks can't hold the volumes' reservations plus UBI's
bad-block reserve is refused before anything is erased.

## NAND slots on a running board

`atlas-ubi nand-slot` carries out Gaia's contract
(Gaia `examples/boards/README.md#nand-slots`). The board has two 128K copies
of U-Boot's environment. U-Boot reads the first and, if its CRC fails, the
second. A slot is a whole prebuilt environment image, which Atlas writes
unchanged after checking that it boots the slot its use requires.

| Command | What it does |
|---|---|
| `status` | The running slot, what the next boot runs, what's committed. |
| `install` | From Gaia's `slot-bundle.tar`, by the target slot: writes the other slot's kernel (skipping bad blocks) and root volume, each read back; keeps its commit image on `/data`; then writes the trial image to the first copy. The caller reboots. |
| `trial-check` | On a trial boot: if U-Boot's erase failed, erases the trial copy and reports it. |
| `watchdog` | On a trial boot: resets the board if the trial hangs or isn't committed by the deadline; stands down once committed. |
| `commit` | The kept image to the first copy, then only once that's read back, to the second. If `/data` didn't keep it, it asks for one with `--env`, and the trial goes on. |

Rejecting a trial is rebooting without committing. What it refuses, before
writing anything:

- a bad env block;
- a second copy that isn't valid (a torn write would leave nothing to boot);
- an image that doesn't boot the expected slot and trial state;
- an install during a trial.

Tests cut the power at each write of a trial and a commit, on a RAM flash,
and check U-Boot's choice each time.

The board's side comes from `devices/luckfox-pico-pro/gaia/nand-slot.toml`,
which Gaia's Luckfox build imports. It holds the binary at `/usr/sbin/atlas-ubi`
(about 473 KB, static ARMv7) and `/etc/init.d/S00nand-slot`, first in rcS, which
runs `trial-check` and starts `watchdog` with a 600 s deadline. Not built yet:
Atlas's slot install driving this over SSH or Orion, which needs the Luckfox
serving the identity contract.

## To settle on the boards

These are marked UNCONFIRMED in the code where they are used, and listed in
each record's `todo`.

- **Rockchip:**
  - the CSW residue's byte order (Atlas ignores the residue);
  - the READ_FLASH_INFO layout beyond the size;
  - RK3576's maskrom PID;
  - whether RV1106 wants RC4 on its 471/472 entries;
  - SPI-NAND: how the usbplug maps LBAs (Atlas assumes directly; it reads
    no block as bad itself), and its CHANGE_STORAGE id.
- **K230:**
  - whether the boot ROM starts Gaia's burner (plain u-boot.bin) at
    0x80300000, where it links;
  - whether the boot ROM wants Canaan's firmware header over USB;
  - the medium codes;
  - the VERIFY reply's layout (if the burner can't checksum, Atlas logs
    "written, not read back");
  - whether the K230D Zero has SPI-NAND at all (its page lists only a TF
    slot).
- **UBI:** a kernel attach of an image `atlas-ubi` wrote. There is no
  nandsim here and no NAND board yet.
