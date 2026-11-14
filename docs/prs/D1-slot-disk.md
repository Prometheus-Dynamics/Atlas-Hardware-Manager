# D1: raze-base on `slot-disk` (DRAFT, held for Lemnos M36's rev)

Draft. Base: `pr/S1` (Gaia pin 498873cf, `--setting` for version and slot, `scripts/version-rule.sh`). Base `next` 515f6be7 underneath.
Status: draft until Lemnos M36's rev is pinned (see OPEN).

## What moved and why

- raze-base's recipe moves from `slot-boot` to `slot-disk` (Gaia 498873cf). The flash build makes a whole first-flash disk: the boot
  partition and a data partition with the slot installed on it, in the board's disk layout, as one first-flash file (`flash.file`).
- The Gaia pin is S1's: `devices/gaia.lock` = 498873cf67126a4cf52f0fd4d8d88e9bbde798d0. This PR does not change it.
- version and slot are not set in `product.toml`: S1's `--setting` gives both (`scripts/build-product.sh`, `scripts/version-rule.sh`).
- The disk comes from `devices/boards/raze/board.toml` `[disk]` and `[[partitions]]` (PR 110), with the data partition's explicit size (below).
- `[generate.data]` on `raze-lemnos-board` (the component that carries `[generate.bundle]`), spelt as Gaia PR 145 takes it (below).
- `scripts/check-devices.sh`: the slot id rule and the boot_size rule (S1's, unchanged except the recipe) accept `slot-disk` as well as
  `slot-boot`, and `SLOT_RECIPES` includes `slot-disk`. No data_size rule: 498873cf takes the size from the board (below).
- Side change: `product.toml` `dictsize` and `cluster` comments state the kernel limits checked in raspberrypi/linux 53679a5 (Linux 7.2.9):
  lzma dictionary 4 KiB to 8 MiB (`fs/erofs/decompressor_lzma.c:93`, `Z_EROFS_LZMA_MAX_DICT_SIZE = 8 * Z_EROFS_PCLUSTER_MAX_SIZE`,
  `erofs_fs.h:326`), so 2 MiB is inside; a compressed cluster at most 1 MiB physical (`Z_EROFS_PCLUSTER_MAX_SIZE`, `erofs_fs.h:297`) and
  12 MiB decompressed (`zmap.c:754`), so 1 MiB is exactly at the physical limit, inside it. Fragments add no separate limit. Comments only.

## New settings (`product.toml` `[images.settings]`)

| setting | value | source |
|---|---|---|
| `id` | `"raze-base"` | slot-disk's `id` (names the disk; GUIDs and MBR signature derive from it). The image name, as Gaia's own example does (`examples/products/zero-3e-demo/product.toml`). |
| `board_id` | `"raze"` | The Raze's `[board] id` in `devices/boards/raze/lemnos-board.toml`. The flasher checks the first-flash file against it. |
| `data_label` | `"lemnos-data"` | The coordinator's value (Lemnos's slot-init finds p2 by this label). Not checked against Lemnos's source: M36 is not at the pin. |

Not set, on purpose:
- No `data_size`: slot-disk at 498873cf has none. The data partition's size is the board's (below).
- `boot_name` and `data_name` default to `boot` and `data`, which match `board.toml`'s partition names. Roles default to the same words.
- `version` and `slot`: from `--setting` (S1).

## The data partition's size: one place, `board.toml`

p2 has an explicit size, **39,845,888 B = 38 MiB** (`devices/boards/raze/board.toml`, `[[partitions]]` role `data`). It is the installed
first slot and the ext4 overhead, rounded up to 1 MiB. No room for a second slot, on purpose: the first start grows p2 to the disk's end,
and the live-install reserve covers updates (the coordinator's ruling).

- Installed slot, 34,578,432 B: erofs 27,058,176 (f8e7a2c6's bundle); slot.json and the boot list (4 KiB each); six boot files by content,
  rounded to blocks (7,471,104; sizes from `tar tvf` of the bundle); six state files and four directories, one block each (upper bound).
- Root directory 4,096 B.
- ext4 overhead, measured with Gaia 498873cf's options (`crates/nodes/fs-ext4`: `-b 4096 -I 256 -m 0`, inode count paths x 2 + 64,
  journal clamp(size/64, 4..128 MiB)) on a scratch file, mke2fs 1.47.3, read with `dumpe2fs -h` and `debugfs`: lost+found 16,384
  (4 blocks), journal 4 MiB, inode table 28,672 (112 inodes x 256 B), superblock, GDT, 4 reserved GDT and 2 bitmaps (8 blocks), 33 blocks
  residual. Total 38,989,824 B.
- Measured minimum: mke2fs accepts 38 MiB and refuses 37 MiB. It leaves 209 free blocks (856,064 B).
- The slot is f8e7a2c6's, before B40's lzma root (18,178,048 B, against 26,841,088 B here). Recompute from the M2 build's lzma root.

Gaia's wiring (`crates/core/graph/src/wire.rs`, `data_size`): the data run's `@data-size@` and the ext4's size are both the board's
partition size, so they cannot differ. The plan shows `raze-base::data` with `data_size` 39845888 and `raze-base::datafs` with size 39845888.

## `[generate.data]` (in `devices/components/raze-lemnos-board/component.toml`)

```
[generate.data]
args = ["data-partition", "--layout-of", "@description@", "--socs", "@source@/socs", "--install", "@bundle@", "--boot-image", "@boot-image@", "--keys", "@keys@", "--time", "@time@", "--data-size", "@data-size@", "--out", "@out@"]
verify = ["data-partition", "--verify", "@out@", "--layout-of", "@description@", "--boot-image", "@boot-image@", "--deep"]
keys = "files/slot-keys"
```

Placeholders, as Gaia 498873cf defines them (`crates/nodes/tree/src/generate_image.rs`, `crates/core/types/src/lib.rs`):
- `@bundle@`: the slot's bundle (slot-boot's `bundle.image`). `@boot-image@`: the boot step's image (`boot.image`, wired to `data.boot_image`).
- `@keys@`: `files/slot-keys` (`keys` relative to the component file), the bench key's public line, the trust list the image ships.
- `@time@`: Gaia's fixed build date, `1970-01-01T00:00:00Z` (epoch 0), the same for every build.
- `@data-size@`: the size of the board's partition named by data_name (`data`): 39,845,888.
- `--layout-of` and `--socs` are named as `[generate.bundle]` names them. `--switch off` is the default and is left out.

Lemnos pr/M36 9f5e0b0f reference: `lemnos-ctl data-partition --layout-of BOARD [--socs DIR] --install BUNDLE --boot-image FILE --keys FILE
--time UTC --data-size BYTES --out DIR [--switch off]`; verify `--verify DIR --layout-of BOARD [--socs DIR] --boot-image FILE --deep`.
The spelling above follows Gaia PR 145 as the coordinator gave it; the flags match M36's. The verify run keeps `--layout-of`, `--boot-image`
and `--deep` and adds nothing else. M36 reads `--socs` optionally, so `--socs` is not in verify.

## Remaining OPEN (what the placeholders did not close)

1. **Lemnos M36's rev** must carry three things before this image is proven:
   - the data-partition tool (`lemnos-ctl data-partition`, pr/M36 9f5e0b0f), which the `[generate.data]` run calls;
   - the offline writer with no reserve: the first write checks only that the slot fits `--data-size` (`drivers/system-slots/src/offline.rs`
     at M36 still checks bundle + 2048 MiB; the ruling is that this changes);
   - the first-start grow of p2 to the disk's end (Lemnos M36b, boot-side). M36 has `grow_target` in `slot-store mbr.rs` and no driver step
     that calls it. If the grow fails at first start, installs are refused with words.
   Until that rev is pinned here, the `[generate.data]` run and the first write are not proven. Lemnos stays at f4a295c6 in this draft; it moves
   to M36's merged rev when the coordinator sends it. The `data_label` value is confirmed against the same rev.
2. Recompute the data partition's size from the M2 build's lzma root (see above).

Closed by 498873cf's placeholders (were OPEN in the previous draft): `--boot-image`, `--keys`, `--time`, `--data-size`, and the verify run.

## Pins and what moves at M36

- Gaia: 498873cf (S1's pin; this PR does not change it).
- Lemnos: f4a295c6 (unchanged). At M36, Lemnos moves to M36's merged rev in `devices/gaia.lock` (if the source is listed there), in
  `devices/products/raze-base/gaia.lock` (regenerate with `gaia lock`), and in the component sources (`lemnos`, `lemnosd`, `lemnos-ctl`,
  `lemnos-orion`, `lemnos-slot-init`, `raze-lemnos-board`). The `[generate.data]` lines are rechecked against M36's `lemnos-ctl data-partition --help`.

## Checks (GAIA_BIN = bin-498873c, GAIA_STORE = store-build)

`./scripts/check-devices.sh`: exit 0.
```
check-devices: passed (3 families, 7 boards, 16 components, 8 products)
check-devices: self-test: the slot id rule passes the real tree and refuses a product that sets version or slot, an id over 64 bytes and a bad product name, and hands Gaia the test value's settings
check-devices: self-check passed (good fixtures pass, broken fixture fails, a wrong or unknown gaia commit is refused)
```

`./scripts/check-config-only.sh`: exit 0.
```
check-config-only: ok products/raze-base: configuration resolved, nothing compiled (1 s)
check-config-only: passed (every product's configuration resolves; nothing compiled)
```

`./scripts/build-product.sh raze-base --plan`: exit 0 (plan only; nothing built). Output at
`/mnt/ssd-samsung/atlas-next-scratch-D1/build-plan.out`:
```
build-product: version 2026.1.0-dev.1062+3ce215ad (the dev scheme of 3ce215ad at /mnt/ssd-samsung/atlas-next-D1)
build-product: --setting raze-base.version=2026.1.0-dev.1062+3ce215ad
build-product: --setting raze-base.slot=raze-base-2026.1.0-dev.1062-3ce215ad
build-product: plan of raze-base with gaia 2.0.6-dev (498873cf67126a4cf52f0fd4d8d88e9bbde798d0) (--plan: nothing built)
graph: 87 nodes, 165 edges
plan: 129 steps
```

Data run: its args resolve. The plan has `raze-base.raze-lemnos-board.generate.data.keys` (the `keys` file) and `raze-base::data` takes
`bundle`, `payload`, `boot_image` (from `boot.image`), `tool`, `root`, `source`, `description` and `keys`. The data node's `runs` JSON
carries the args above with the placeholders unexpanded (Gaia expands them at run time). `@time@` is a Gaia constant, not a wire.

Plan outputs, the disk and flash lines (host outputs) — `raze-base::...` -> `host.raze-base.*`:
```
raze-base::data.tree -> host.raze-base.data
raze-base::datafs.image -> host.raze-base.data-fs
raze-base::data.report -> host.raze-base.data-report
raze-base::disk.image -> host.raze-base.disk
raze-base::flash.file -> host.raze-base.flash
raze-base::boot.image -> host.raze-base.boot
raze-base::bundle.image -> host.raze-base.bundle
raze-base::verity.image -> host.raze-base.slot
host.raze-base.disk-layout -> raze-base::disk.layout
```
Outputs (`outputs:`): `boot`, `boot-report`, `bundle`, `bundle-report`, `cmdline`, `data`, `data-fs`, `data-report`, `disk`, `flash`,
`initramfs`, `overlay`, `payload`, `slot`, `table`.
Plan JSON check: `raze-base::datafs` label `lemnos-data`, size 39845888; `raze-base::data` data_size 39845888.

## NOT proven

- No real build (Gaia runs those). No image, no disk, no first-flash file, no xz stream. The plan is not a build.
- No board. Nothing on Raze hardware.
- The `[generate.data]` run cannot succeed until Lemnos's rev with the data-partition tool, the offline writer change, and the first-start grow is pinned.
- `data_label` is not checked against Lemnos.
- The kernel limits for dictsize and cluster are read from source in 53679a5, not run on a kernel.
- The data size (38 MiB) is measured with mke2fs on a scratch file, not on a built image, and its slot is f8e7a2c6's.
