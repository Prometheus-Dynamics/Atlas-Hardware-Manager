# Flashing from a SoC's download mode

Atlas flashes a blank or bricked board from its SoC's USB download mode, in
Rust, with no vendor tool at run time. Status: **written, untested on a
board.** The boards are on order. Each board record says
`"status": "untested"`, and Atlas logs that before it flashes.

| Board | SoC | Mode (USB id) | Storage | Record |
|---|---|---|---|---|
| Radxa ROCK 5C | RK3588S2 | maskrom 2207:350b | eMMC | `devices/radxa-rock-5c` |
| Radxa ROCK 4D | RK3576 | maskrom 2207:350e (a guess: `pid_unconfirmed`) | eMMC | `devices/radxa-rock-4d` |
| Luckfox Pico Pro | RV1106G2 (Flame's SoC) | maskrom 2207:110c | SPI-NAND | `devices/luckfox-pico-pro` |
| BPI-CanMV-K230D-Zero | K230D | USB boot 29f1:0230 | microSD (to confirm) | `devices/bpi-canmv-k230d-zero` |

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
is offered for any Rockchip in a download mode whose PID no record names.
Once the usbplug runs, Atlas reads the chip number the board reports
(`READ_CHIP_INFO`) and checks it against the record's `platform.soc`. If it
differs, the flash stops before anything is written. Every Rockchip flash
runs this check, not only the guessed ones.

## Checked against real loaders

Gaia's builds produced rkbin's `rk3588_spl_loader_v1.18.113.bin` (ROCK 5C)
and `rv1106_download_v1.15.108.bin` (Luckfox Pico Pro).
`crates/atlas-socflash/tests/real_loaders.rs` parses any loaders named in
`ATLAS_TEST_LOADERS`. Both pass, and they settled four points:

- The header layout and entry tables are as derived. The 471 stage has two
  entries: `UsbHead`, then the DDR init.
- The trailer CRC-32 uses rkdeveloptool's polynomial, 0x04C10DB7.
- The header's chip field is the chip number in reversed ASCII ("3588",
  "1106"). Atlas refuses a loader whose chip isn't the record's SoC.
- FlashHead is the RKNS header, RC4-scrambled. Its image table puts
  FlashData and FlashBoot exactly where Atlas's idblock does, and Atlas
  refuses a loader where they disagree.

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
  - the burner's load address (0x80360000 or 0x80300000);
  - whether the boot ROM wants Canaan's firmware header over USB;
  - the medium codes;
  - the VERIFY reply's layout (if the burner can't checksum, Atlas logs
    "written, not read back");
  - whether the K230D Zero has SPI-NAND at all (its page lists only a TF
    slot).
- **UBI:** a kernel attach of an image `atlas-ubi` wrote. There is no
  nandsim here and no NAND board yet.
