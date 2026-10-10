# raze-flasher: USB-boot flashing Atlas controls end to end

Status: design, for review (HeliOS, Gaia, Lemnos). Nothing here is built yet.

Today a USB-boot flash goes: Atlas boots the board over USB (its own Rust
port of rpiboot, `crates/atlas-usbboot`), using Raspberry Pi's
`mass-storage-gadget64` boot files; the board comes up as a USB disk; the
elevated `atlas-helper` writes the image to that block device through the
host kernel; Atlas types `sync && reboot` into the gadget's console.

This design replaces the vendor boot image and the host-side disk write with
our own flasher: a tiny Linux from the Raze build that runs an Atlas daemon,
which receives the image over USB and writes, verifies and reports on the
board itself, driving the status ring as it goes.

What it removes from the host: pkexec / UAC elevation for writes, the
desktop's disk handling (auto-mount, eject), `mass-storage-gadget64`, and
any need for rpiboot or rpi-eeprom tooling. What stays from Raspberry Pi:
the second-stage bootloader the CM5 boot ROM needs (`bootcode5.bin`, a
firmware blob, redistributable) and its outer `config.txt`.

## 1. USB boot (Atlas, Rust): already done, a small change

`atlas-usbboot` already implements what rpiboot does, over nusb, on Linux,
Windows and macOS:

- device ids `0a5c:2711` (BCM2711/CM4) and `0a5c:2712` (BCM2712/CM5);
- round 1: the boot ROM takes the second stage (`bootcode5.bin` for 2712)
  over a vendor control request plus bulk transfers;
- round 2: the second stage re-enumerates and runs the file server loop
  (`GetFileSize`, `ReadFile`, `Done`): it asks for `config.txt`, then the
  files that names (`boot.img` with `boot_ramdisk=1`), until done.

It reads its files from a directory or tar (`BootFiles`), so serving
raze-flasher instead of mass-storage-gadget64 is a different directory:

    raze-flasher/
      bootfiles/2712/bootcode5.bin   Raspberry Pi firmware (usbboot release)
      config.txt                     outer: boot_ramdisk=1, uart_2ndstage=1
      boot.img                       ours (Gaia), FAT: kernel, dtb, initramfs
      boot.sig                       only for secure-boot boards (not today)

Change: `atlas-driver-rpi` picks raze-flasher when the board's device
package ships it (or Atlas bundles it), and keeps mass-storage-gadget64 as
the fallback until the new path is proven on hardware (setting, plus an
automatic fallback when the flasher doesn't answer within 30 s of the file
server finishing).

## 2. The flasher image (Gaia builds, Atlas defines)

An extra assembly output of the same Raze Buildroot build (Gaia's
recommendation): the OS's own kernel `Image`, dtbs, firmware overlays and
modules, with an external initramfs (`initramfs initramfs.cpio.zst
followkernel`). Published as its own output (`raze-flasher-boot.img` plus
its `bootcode5.bin` and outer `config.txt`), versioned with the device
package; Atlas bundles a pinned one and also accepts one from a device
package or release, newest compatible wins.

### Kernel modules (Gaia `kernel_modules`, closure via modules.dep)

    libcomposite usb_f_fs usb_f_acm u_serial   # gadget: FunctionFS + console
    dwc2                                        # if not built in
    rp1-pio ws2812-pio-rp1                      # the status ring
    vfat nls_cp437 nls_iso8859_1                # p1/p2/p3 (if not built in)
    ext4 crc32c_generic                         # /data, read and UUID (if not built in)

EROFS, MMC and SDHCI are built in on Raze. Anything above that is built in
is skipped by Gaia, which is fine.

Shared-config needs (raze.config): nothing new beyond `USB_FUNCTIONFS`
(=m) and `USB_CONFIGFS_F_FS`; both are expected on but must be checked.

### Initramfs files

    /init                       busybox sh: mounts proc, sys, devtmpfs,
                                configfs, functionfs; loads the modules;
                                builds the gadget; starts the daemon;
                                respawns it; drops to a shell on ttyGS0
    /bin/busybox (+ applets)    sh, mount, modprobe, mdev, sync, reboot
    /usr/bin/atlas-flasher      the daemon (static aarch64-musl, Rust)
    /usr/share/lemnos/looks/    the system looks (TOML) the ring uses
    /etc/flasher.toml           ring geometry (count, offset, direction,
                                wire) from the device package's leds.env

No e2fsprogs, no Python, no vcgencmd: the daemon does the few filesystem and
firmware things it needs itself (below).

### config.txt (inside boot.img)

    [all]
    kernel=Image
    initramfs initramfs.cpio.zst followkernel
    arm_64bit=1
    enable_uart=1
    dtoverlay=dwc2,dr_mode=peripheral
    dtoverlay=ws2812-pio,gpio=13,num_leds=16,brightness=255,dev_name=leds%d
    dtparam=watchdog=on

(The ring's overlay line comes from gen-raze, like raze-device.txt.)

### cmdline.txt

    console=serial0,115200 console=tty1 rdinit=/init loglevel=4 quiet

No `root=`: the board runs from the initramfs only and never mounts its own
root.

### Size

Gaia's estimate, ~12–16 MB (gzip'd kernel ~8–9 MB, initramfs 3–6 MB; the
daemon is ~3 MB static). The file server moves ~25–35 MB/s on USB 2.0, so
under a second. mass-storage-gadget64's boot.img is 29 MB for comparison.

## 3. The flasher protocol

### Transport: a vendor bulk interface (FunctionFS), not a network

The daemon owns a FunctionFS interface (class 0xff, interface string
`atlas-flasher`, one bulk IN and one bulk OUT endpoint); Atlas opens it with
nusb, the same library and permissions as USB boot.

Why not CDC-NCM/ECM: a network link needs the host to configure an
interface (NetworkManager, Windows' UsbNcm, macOS), addresses, firewalls,
and each OS does it differently and slowly; bulk endpoints need nothing but
USB access Atlas already has, are as fast (~35–40 MB/s on USB 2.0), and
leave no network interface behind. The gadget also carries an ACM console
(ttyGS0, a shell) for debugging. On Windows the interface reports MS OS 2.0
descriptors (WinUSB compatible id), so it binds without an installer.

USB ids: `1d6b:0104` (as the Raze's own gadget), product string `Raze
flasher`, serial = the board serial (as the boot ROM reports it, so Atlas
matches the board it booted).

### Framing

Each message: a 4-byte little-endian length, then a JSON object (control),
or for data an 8-byte header (`'D'`, stream id, length) and raw bytes.
Version 1; the first exchange is `hello`.

### Trust

The link is a cable between Atlas and a board in USB boot: there is no
remote attacker on it. The daemon still binds every session to the board:
`hello` returns the serial, and Atlas refuses a flasher whose serial isn't
the board it booted. No other authentication (a passphrase would protect
nothing a person holding the cable couldn't bypass by booting their own
image).

### Operations

| op | request | reply / stream |
|---|---|---|
| `hello` | `{protocol: 1, atlas}` | `{protocol, daemon, serial, model, revision}` |
| `info` | — | serial, eMMC (size, `life_time`, `pre_eol_info`, name, manfid), partition table, per slot the OS version (os-release from p5/p6 EROFS, read only), the active slot (p1 `autoboot.txt`), `board/flash-id`, /data present (p7 ext4 UUID, size), EEPROM bootloader version and config (below) |
| `write` | `{size, sha256, format: "zst" \| "raw", keep_data}` then data | progress `{written, of}`; the daemon decompresses on the board (zstd in Rust), writes `/dev/mmcblk0` sequentially with O_DIRECT, computes sha256 of the image bytes it wrote |
| `verify` | — | re-reads what was written; `{ok, sha256}` against the stream's |
| `reboot` | `{into: "emmc" \| "usb-boot"}` | then the link goes |
| `logs` | `{what: ["pstore", "journal", "events"]}` | a tar stream: p7's `/board/pstore`, `/board/events.jsonl`, `/var/log/journal` (mounted read only) |
| `backup-data` / `restore-data` | — / data | p7 as an ext4 image stream (`zstd`), for keeping /data on another machine |
| `eeprom` | `{get}` | the bootloader version and config (from the firmware mailbox, `/dev/vcio`, as vcgencmd reads it) |
| `ring` | `{look}` | shows a look (locate, test) |

The truncated packed-EBR image needs nothing special: the write is the
image's bytes from sector 0, however long; the EBRs are in it.

### Keeping /data across a reflash (`keep_data`)

Today a flash resets /data by design (data-setup sees a new flash id: an ext4
UUID that doesn't match). With the flasher:

- default (factory): after the write, zero p7's first MiB, as `atlas flash`
  promises today;
- `keep_data`: leave p7 alone (the truncated image ends before it), and set
  p7's ext4 UUID to the one the new image's `board/flash-id` gives (the
  daemon patches the superblock's `s_uuid` and its checksum itself), so
  data-setup keeps it on first boot. Refused when the new image's table
  moves p7. `backup-data` first is offered in the UI for safety.

### EEPROM updates

Not in the flasher: Atlas already updates the bootloader EEPROM over USB
boot with Raspberry Pi's recovery flow (`atlas-driver-rpi/src/eeprom.rs`:
`recovery5.bin` plus `pieeprom.bin`), which needs no Linux at all. The
flasher only reads the version and config (`eeprom get`), and Atlas offers
the update through the existing path when it's older than the package
wants. This keeps rpi-eeprom's Python and flashrom out of the image.

## 4. The ring while flashing

The daemon links Lemnos's `lemnos-light` (no_std renderer) and the system
looks, and writes frames to `/dev/leds0` (4 bytes per LED, the board's
offset and direction from flasher.toml), the way lemnosd does:

- booted, waiting for Atlas: `system.booting`
- writing: `update.writing` with the progress arc (written / of)
- verifying: `update.verifying` (the purple orbit)
- done, rebooting: `system.confirmed` (the drain), then the reboot ember
  held across the reset (the ws2812 `clear_on_probe=0` patch keeps it)
- failed: `system.failed` (red breath), until Atlas or a power cycle

So the board shows its progress with no host UI. Atlas mirrors the same
state in its job view.

## 5. Atlas UX

- `atlas flash` (app and CLI) uses the flasher when available: no block
  device on the host, so no pkexec, no udisks, no eject; it streams the
  `.img.zst` as it is (the board decompresses), then verify, then reboot.
- "Erases the board's eMMC" stays the default; `--keep-data` (a checkbox in
  the app, with the backup offer) keeps /data as above.
- The job view shows what the ring shows (writing n %, verifying, done) and
  `info` before the write (eMMC health, installed version, what will be
  erased).
- The old path stays as a fallback (setting `flasher = auto | off`).

## 6. Open questions, answered

- **Unsigned boot.img on CM5?** Yes, unless the board has secure boot
  provisioned (customer key in OTP plus `SIGNED_BOOT=1`): the 2712 USB boot
  second stage loads an unsigned `boot.img` with `boot_ramdisk=1`, exactly as
  mass-storage-gadget64's is. A secure-boot board needs `boot.sig` from our
  key; none are provisioned today.
- **Minimum EEPROM version?** None: in USB boot the boot ROM loads the
  second stage from the host (`bootcode5.bin` we ship), not the board's
  EEPROM. We pin the usbboot firmware release whose bootcode5.bin we ship
  (the one mass-storage-gadget64 is tested with today) and update it
  deliberately.
- **Is a gadget link reliable from the USB-loaded kernel?** It is a normal
  kernel boot after the second stage; mass-storage-gadget64 already runs
  dwc2 with configfs (mass storage + ACM) this way, and c3a0241's restart
  uses its ACM console. FunctionFS on the same dwc2 is the same machinery.
- **How big?** ~12–16 MB (above).

## Ownership and order

1. Gaia: the flasher output from the Raze build (modules, initramfs, boot.img)
   — first build to boot on a CM5 with the ACM shell only.
2. Atlas: `crates/flasher` (the daemon: gadget, protocol, write/verify/info,
   ring) and `crates/atlas-flasher` (the host client), the driver-rpi switch,
   tests against a fake FunctionFS pipe.
3. Lemnos: `lemnos-light` as a no_std dependency the daemon builds for
   aarch64-musl, plus the looks.
4. On hardware: boot, `info`, a write and verify of r20, timings, the ring.
