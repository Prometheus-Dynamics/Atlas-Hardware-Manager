# D1: raze-base on `slot-disk` (DRAFT)

Draft PR on the Atlas `next` rebuild, base `next` 515f6be7. Status: draft, held for the Lemnos M36 pin.

## What moved and why

- raze-base's recipe moves from `slot-boot` to `slot-disk` (Gaia e9684c5). The flash build now makes a whole first-flash disk: the boot partition
  and a data partition with the slot installed on it, in the board's disk layout, as one xz stream (`flash.file`).
- Gaia's pin moves b8585a0 -> e9684c5a70d2358a657fde0f4e90801162d90b71 (`devices/gaia.lock`, comment kept).
  raze-base's own `devices/products/raze-base/gaia.lock` regenerated with `gaia lock` (pinned binary): "up to date", the sources did not change.
- The disk comes from `devices/boards/raze/board.toml` `[disk]` and `[[partitions]]` (PR 110). No numbers are duplicated here.
- `[generate.data]` added to `raze-lemnos-board` (the component that carries `[generate.bundle]`), with the expressible flags only. See OPEN.
- `scripts/check-devices.sh`: the slot id rule, the boot_size rule and `SLOT_RECIPES` accept `slot-disk` as well as `slot-boot`.
  No data_label rule (no single source yet).
- Side change (coordinator's note, 2026-11-14): the `dictsize` and `cluster` comments in `product.toml` now state the kernel limits as checked
  in raspberrypi/linux 53679a5 (Linux 7.2.9): lzma dictionary 4 KiB to 8 MiB (`fs/erofs/decompressor_lzma.c:93`,
  `Z_EROFS_LZMA_MAX_DICT_SIZE = 8 * Z_EROFS_PCLUSTER_MAX_SIZE`, `erofs_fs.h:326`), so 2 MiB is inside; a compressed cluster at most 1 MiB physical
  (`Z_EROFS_PCLUSTER_MAX_SIZE`, `erofs_fs.h:297`) and 12 MiB decompressed (`zmap.c:754`), so cluster 1 MiB is exactly at the physical limit;
  fragments add no separate limit. The 4-stream / 8 MiB RAM cost note is kept. Comments only: no setting changed.
- The product header's pin sentence and the image description now name slot-disk and e9684c5.

## New settings and their sources (`[images.settings]` of raze-base)

| setting | value | source |
|---|---|---|
| `id` | `"raze-base"` | Gaia's parameter `id` (names the disk; GUIDs and MBR signature derive from it). The image name, as Gaia's own example does (`examples/products/zero-3e-demo/product.toml`, `id = "zero-3e-demo"`). |
| `board_id` | `"raze"` | The Raze's `[board] id` in `devices/boards/raze/lemnos-board.toml`. The flasher checks the first-flash file against it. |
| `data_label` | `"lemnos-data"` | The coordinator's value for this draft (Lemnos's slot-init finds p2 by this label). Not checked against Lemnos source: M36 is not at the pin. |

Not set, on purpose:
- `boot_name` / `data_name` (defaults `boot` and `data`) match `board.toml`'s partition names; roles default to the same words.
- `data_size` = 39845888 (set, see "Data partition size"; was 0, content-sized, before the D1 follow-up).
- `boot_size`, `slot`, `version`, `product`, `device`, and the erofs settings are unchanged from slot-boot.

## The `[generate.data]` lines (in `devices/components/raze-lemnos-board/component.toml`)

Written:

```
[generate.data]
args = ["data-partition", "--layout-of", "@description@", "--socs", "@source@/socs", "--install", "@bundle@", "--out", "@out@"]
```

`--layout-of` and `--socs` are named as `[generate.bundle]` names them. `--install` is `@bundle@` (the slot bundle). `--out` is `@out@`, the
directory Gaia makes the ext4 filesystem of. `--switch off` is the default and is left out.

Lemnos pr/M36 9f5e0b0f reference: `lemnos-ctl data-partition --layout-of BOARD [--socs DIR] --install BUNDLE --boot-image FILE --keys FILE --time UTC --data-size BYTES --out DIR [--switch off]`.
Verify, from the same PR: `lemnos-ctl data-partition --verify DIR --layout-of BOARD [--socs DIR] --boot-image FILE --deep`.

## OPEN items (exact gaps in Gaia e9684c5)

A `[generate.data]` run gets only `@source@`, `@description@`, `@slot@`, `@bundle@` and `@out@`. Gaia refuses any other name.

1. `--boot-image FILE`: the boot partition's output. No name for it in a data run. Not written. Blocks the build and the verify run.
2. `--keys FILE`: the trust list (`files/slot-keys`, the bench key's public line). A component file has no name here; `@description@` is the board file only. Not written.
3. `--time UTC`: the build's source date. e9684c5 has no source-date name, and the generate run's environment is `PATH` and `LC_ALL` only. Not invented.
4. `--data-size BYTES`: p2's size, 74,448,896 (the D1 ruling; see "Data partition size" below). Gaia has no placeholder for it: a data run has no `@size@` (refused for a tree). When Gaia gives the run one, it takes this number, the same as `product.toml` `data_size` and `board.toml`'s data partition.
5. `verify` (`data-partition --verify ... --boot-image FILE --deep`): not written, because it needs item 1.
6. Dependency (ruled, not a blocker in this PR): M36's rev must carry the offline-writer change (no reserve on the first write) and the first-start grow of p2 (Lemnos M36b). Until the pin moves, the first write is not proven. See "Data partition size".

Consequence: with these four flags missing, the Lemnos step will be refused at build time. The plan still resolves, because planning does not run the tool.
The Gaia gap for 1-5 is for HeliOS/Gaia to close: a placeholder for the boot output, the keys file, the source date, and the partition size.

Also OPEN:
- `data_label` = `lemnos-data`: not checked against Lemnos slot-init. Clears at the M36 pin, or with a Lemnos source reference.
- The Lemnos pin stays at f4a295c6 in this draft. It moves to M36's merged rev when the coordinator sends it.

## Data partition size (D1 follow-up 2, coordinator's ruling 2026-11-14)

p2 has an explicit size: **39,845,888 B = 38 MiB**. It is the installed first slot and the ext4 overhead, rounded up to 1 MiB. There is
no room for a second slot, on purpose: the grow covers it, and the live-install reserve is the rule for updates. The same number is in
`devices/boards/raze/board.toml` (with the arithmetic and each term's source) and `product.toml` `data_size` (check-devices refuses two).

- Installed slot, 34,578,432 B: erofs 27,058,176 (f8e7a2c6's bundle, `slot.json` image.size); slot.json and the boot list (4 KiB each);
  six boot files by content, rounded to blocks (7,471,104; sizes from `tar tvf` of the bundle); six state files and four directories,
  one block each (upper bound).
- Root directory 4,096 B.
- ext4 overhead, measured: lost+found 16,384 (4 blocks, debugfs), journal 4 MiB (Gaia's clamp of size/64), inode table 28,672 (112 inodes
  x 256 B), superblock, GDT, 4 reserved GDT blocks and 2 bitmaps (8 blocks), 0 reserved (`-m 0`), 33 blocks residual. Total 38,989,824 B.
- Measured minimum: the search (Gaia's exact mke2fs options, a regular file in scratch, `dumpe2fs -h`) accepts 38 MiB and refuses 37 MiB.
  It leaves 209 free blocks (856,064 B). Earlier numbers for the same method: 71 MiB with one more slot (superseded, not used).
- The slot is f8e7a2c6's, before B40's lzma root (18,178,048 B against 26,841,088 here). Recompute from the M2 build's lzma root when it
  exists. The number is an upper bound until then.

Lemnos's install checks, read at M36 (`9f5e0b0f`), and the ruling on them:

- Offline first write (`drivers/system-slots/src/offline.rs:263`): at M36 it refuses unless `--data-size` >= bundle + the 2048 MiB default
  reserve. The ruling: the offline writer checks only that the first slot fits `--data-size`, with no reserve, because p2 grows to the disk's
  end at the first start, before any update can happen. That change is in Lemnos M36b's commits, not in M36.
- Live install (`room()`, `lib.rs:692`): refuses unless free >= size + the board's `reserve_mb` (2048). The reserve stays for live installs only.
  It is kept for /data on the Raze's ~14.9 GB eMMC; the reason is in `devices/boards/raze/lemnos-board.toml` beside `reserve_mb`.

Dependency: M36's rev must carry (1) the offline-writer change (no reserve on the first write) and (2) the first-start grow of p2 to the
disk's end (Lemnos M36b, its boot-side commits). The grow is not in M36 today: `slot-store mbr.rs` has `grow_target` and nothing in
`system-slots` calls it. If the grow fails at first start, installs are refused with words. Until the pin moves, the first Lemnos write of
this image is not proven.

## Pins and what moves at M36

- Gaia: b8585a0 -> e9684c5 (this PR).
- Lemnos: f4a295c6 (unchanged). At M36, the Lemnos pin moves to M36's merged rev in `devices/gaia.lock` (if the source is listed there), in `devices/products/raze-base/gaia.lock` (regenerate with `gaia lock`), and in the component sources (`lemnos`, `lemnosd`, `lemnos-ctl`, `lemnos-orion`, `lemnos-slot-init`, `raze-lemnos-board`) with `[generate.data]` flags reconciled against M36's `lemnos-ctl data-partition --help`.
- Only the Lemnos pin moves at M36 (and the four flags above, once Gaia can express them).

## Checks

`GAIA_BIN=/mnt/ssd-samsung/atlas-next-scratch-gaia/bin-e9684c5/gaia GAIA_STORE=/mnt/ssd-samsung/atlas-next-scratch-gaia/store-build ./scripts/check-devices.sh`
- exit 0, self-check passed (output at `/mnt/ssd-samsung/atlas-next-scratch-D1/check-devices.out`).
- Tail: `check-devices: passed (3 families, 7 boards, 16 components, 8 products)`, `check-devices: self-check passed`.

`GAIA_BIN=... GAIA_STORE=... ./scripts/check-config-only.sh`
- exit 0: `check-config-only: passed (every product's configuration resolves; nothing compiled)`; raze-base `ok ... configuration resolved (5 s)`.

`gaia plan devices/products/raze-base/product.toml` (pinned binary) exit 0. Disk and flash outputs (host outputs):

```
raze-base::boot.image        -> host.raze-base.boot
raze-base::boot.report       -> host.raze-base.boot-report
raze-base::bundle.image      -> host.raze-base.bundle
raze-base::bundle.report     -> host.raze-base.bundle-report
raze-base::data.tree         -> host.raze-base.data
raze-base::datafs.image      -> host.raze-base.data-fs
raze-base::data.report       -> host.raze-base.data-report
raze-base::disk.image        -> host.raze-base.disk
raze-base::flash.file        -> host.raze-base.flash
```

Follow-up run (data size): `check-devices.sh` exit 0, `check-config-only.sh` exit 0, `gaia plan` exit 0. New lines: the data partition rule
`check-devices: self-test: the data partition rule passes the real tree and refuses a data_size that is not the board's`, and the
self-check passed. Plan: `datafs` size 39845888, label `lemnos-data`; the disk layout's partitions[1] size 39845888.

The plan resolves the Lemnos tool from the f4a295c6 pin (the generate node is `raze-lemnos-board.generate`). It does not check the M36 flags, so this is not proof that `data-partition` accepts them.

## NOT proven

- No real build (Gaia runs those). No image, no disk, no flash file, no xz stream.
- No board. Nothing on Raze hardware.
- The `[generate.data]` run cannot succeed yet (OPEN 1-4).
- Lemnos `data-partition` flags are from M36's commit message, not from a run of the tool at a pin that has them.
- `data_label` is not checked against Lemnos.
- The kernel limits for dictsize and cluster are read from source in 53679a5, not run on a kernel.
- The data size (38 MiB) is measured with mke2fs on a scratch file, not on a built image. Its slot is f8e7a2c6's, not the M2 build's.
  It is not proven to install until the Lemnos rev with the offline-writer change and the first-start grow is pinned.
