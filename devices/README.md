# devices/

Board descriptions as data. Gaia `next` reads them; the app embeds only what it needs to recognise a
board (a small generated catalog). Nothing here is code.

This directory holds no real boards yet. B2 and B3 add them. The fixtures under `tests/fixtures/`
are examples for the check only; they are not devices.

## Layout

```
devices/
  README.md                this file
  gaia.lock                the Gaia `next` commit these files are checked with (see below)
  families/<family>/family.toml        one target shared by boards
  boards/<board>/board.toml            extends a family; keeps only what differs
  boards/<board>/app/app.json          app data for the board (see "App data" below)
  components/<component>/component.toml  one piece of software and what it installs
  products/<product>/product.toml      one image: its board, components, OS and recipe
  products/<product>/gaia.lock         the product's lock, written by `gaia lock` (see below)
  usb-ids.json                         USB ids shared by every board (see "App data" below)
  tests/fixtures/                      examples and a broken copy, used by `--self-test`
```

Boards, plainly: the Allwinner A733 boards (Cubie A7A, A7S, A7Z) are listed with their blockers and open items at the top of `families/sun60i/family.toml`.

A board directory holds `board.toml`, or, when it is app-only, only `app/` (see App data).

## App data

Two kinds of file here are for the app, not for Gaia. The check never reads them as Gaia files.

- `boards/<board>/app/app.json`: one per board. `model` is the board directory's name. Required:
  the display name and the USB id keys. Optional: a hostname prefix, `found_by` (the driver family
  that finds a device with no USB id, free text), named `ports` (kind and label), `look` (pictures,
  a case file, and the case's `orientation`), and `variants` (options of the board, such as lenses,
  each with a key, a display name, `confirmed`, and the fields it overrides). Files are paths
  relative to `app/`.
- `usb-ids.json`: the USB ids, keyed by name. Each entry has `state` (`download`, `storage` or
  `gadget`), `vid`, `name`, and `pid` (left out for a vendor-wide id that matches any product of
  the vendor; an exact `vid:pid` match ranks above it). A board's `app.json` refers to these by key.

`crates/core/catalog` is the reader, and it is the authority on the exact keys. The app embeds the
catalog it generates from these files; the files themselves are not embedded.

App-only boards: a board directory that holds only `app/` and no `board.toml` is app-only. It is a
recognition-only device (a radio, a roboRIO) or a board whose description is not merged yet. The
check skips it, does not count it as a board, and prints one line with how many it skipped. A board
directory with no `board.toml` that is not app-only fails the check, and the other directories are
still checked.

Every family, board, component and product is its own directory, holding its description file.
Other files may sit beside the description (assets, a component's `src/`, a fragment, `app/`). The
check validates only the description files, but it does require that every family, board and
component is checked. One that no product or board names yet is validated on its own and
listed in a NOTE line as "in no product yet": it is a record, never planned or built, until a
product names it.
An app-only board is named by nothing, which is why it is skipped rather than failed.

## Gaia's format (what Gaia `next` reads at the pin)

Gaia's source of truth is `crates/core/describe/src/*.rs` at the locked commit. Each struct there
is `deny_unknown_fields`, so a misspelled key is refused, not ignored. The examples are in
`examples/` of the same commit.

- **family.toml, board.toml**: `version = 1`. A board may set `extends = "../../families/<f>/family.toml"`
  (the path is relative to the board file). Board tables override the family's, field by field.
  `[target]` (`arch`, `cpu`, `os`), `[sources.<name>]` (`git` + `ref`, or `path`), `[kernel]`
  (`source`, `defconfig`, `fragments`, `device_tree`, `device_tree_dir`, `overlays`),
  `[bootloader]` (`source`, `defconfig`, `fragments`, `[bootloader.blobs]`,
  `[bootloader.trusted_firmware]`), `[[firmware]]`, `[[partitions]]`, `[disk]`, `[flash]`.
  - `[[partitions]]` keys: `name`, `role`, `offset`, `size`, `type`, `in_table`, `content`. `role` is
    one of `bootloader`, `boot`, `esp`, `rootfs`, `data`, `raw`. `in_table = false` is an area outside
    the partition table; it needs `offset`, and only it may name `content` (a file from the OS's
    bootloader output). `bootable` is not a key here (validate refuses it): bootability is a recipe
    setting (see `sd-image` below).
  - `[disk]`: `scheme` (`gpt` or `mbr`) is required when there are partitions (validate refuses a
    missing one); `first`, `alignment`, `size` are optional.
- **component.toml**: `version = 1`, `name`, `description`, `[sources.<name>]`, `[build]`
  (`builder`, `source`, `[build.settings]`), `[[install]]`, `[[services]]`, `[[users]]`,
  `[[groups]]`, `[package]` (for an `.ipk`). The Rust builder's form is under "Rust components" below.
- **product.toml**: `version = 1`, `name`, `description`, `[sources.<name>]` (the OS source, see
  below), `[components]` (name = path to a component.toml), `[[images]]` with `name`, `board` (path
  to a board.toml), `components` (names), `os` (see below), `data` (names), `recipe`, `budget`, and
  `[images.settings]` for the recipe's own settings.

## The product form (Gaia F8 and sd-image, at the pin)

A product is one image. Its OS comes from a source the product names; the board gives the kernel,
U-Boot and TF-A; the product's OS fragment gives the rest.

The `firmware` form, which `gaia validate` and `gaia plan` accept at the pin, is
`tests/fixtures/good-f8/products/f8-product/product.toml`:

```toml
version = 1
name = "f8-product"

[sources.kernel]
path = "sources/kernel"

[[images]]
name = "f8-product"
board = "../../boards/example-board/board.toml"
recipe = "firmware"
os = { provider = "buildroot", source = "kernel", fragments = ["fragments/os.fragment"] }
```

The `sd-image` form (the disk recipe; Gaia's own example is `examples/products/zero-3e-demo`) is
`tests/fixtures/good-sd-image/products/sd-image-product/product.toml`. It validates and plans at the
pin with the board in `tests/fixtures/good-sd-image/boards/example-board/board.toml`, which has
`[disk] scheme = "gpt"` and a `u-boot` area at `32K` with `in_table = false`:

```toml
version = 1
name = "sd-image-product"

[sources.kernel]
path = "sources/kernel"

[[images]]
name = "sd-image-product"
board = "../../boards/example-board/board.toml"
recipe = "sd-image"
os = { provider = "buildroot", source = "kernel", fragments = ["fragments/os.fragment"] }

[images.settings]
id = "sd-image-product"
```

A real product names a Buildroot git source (`git` + `ref`) instead of the fixture's path.

The fragment file is Buildroot config (`BR2_...=y`). `@here@` in it is that file's directory. Plan
shows it as the fragment's input tree (`@files.0@` in the `os-config` host input).

## Rust components (at the pin)

A component that builds a Cargo package uses Gaia's `rust` builder. The form is Gaia's
`examples/components/hello-rust/component.toml` at `aa987cc`, as `tests/fixtures/good-rust` has it:

```toml
version = 1
name = "hello-rust"

[sources.src]
path = "src"

[sources.root]
image = "docker.io/library/debian:trixie-slim"
platform = "host"

[build]
builder = "rust"
source = "src"

[build.settings]
package = "hello-rust"

[[install]]
from = "bin/hello-rust"
to = "/usr/bin/hello-rust"
mode = "0755"
```

- `[sources.src]` is the crate: a path, or a git source (`git` + `ref`). Gaia's example comment says a
  git source works the same way; only the path form was tried here.
- `[sources.root]` is the build image. Its digest goes in the product's lock: `gaia lock` resolves it
  (it reaches the registry), and `gaia plan` pulls nothing (the store stayed empty after a plan).
- One component is one cargo package. `[build.settings] package` names it, and the build runs
  `cargo build --package <package>`. A workspace with several binaries is one component per package.
- The target is the board's: no triple in the description. The crates come from the source's
  `Cargo.lock`. The Gaia node refuses a source with no `Cargo.lock` at build time (read in the node's
  source at aa987cc, not run).
- Other `[build.settings]` the rust builder takes: `profile` (a cargo profile name; unset is `release`),
  `features`, `cc` and `env` (for a crate that compiles C), `triple` and `cpu` (the board's when unset).
  Plan accepts them all; it refuses `vendor` and an unknown key by name. Leave `profile` unset until
  the crate's own profile name is confirmed.
- Planning needs two tools on PATH, or passed with `--tool`:
  - `bwrap`: plan refuses without it (`needs its input bwrap, and nothing gives it`). The check gives
    planning a stand-in when it is missing, with a NOTE line, as it does for veritysetup. A build needs
    bubblewrap.
  - `rustc`: its sysroot is the `toolchain` input. Plan refuses without it (`needs its input
    toolchain`).
- A Rust product with the firmware recipe and no `os` plans without `veritysetup`, `mkfs.erofs`,
  `mke2fs`, `debugfs` and `make` on PATH (all removed, planned, passed). `python3` is needed only for a
  python component.
- The build machine needs Rust 1.99.0 (Gaia's `rust-toolchain.toml`) and bubblewrap. Not built here:
  this check plans only.

## F8 format points, as Gaia aa987cc treats them (historical: not re-run at 7db227f)

Each row was tried with Gaia `aa987cc` (`gaia 2.0.6-dev`). "validate" is `gaia validate`, "plan"
is `gaia plan` with the product's lock and `--tool veritysetup=...` where a recipe needs it.
Only what was seen is written here.

| Point | Format for B-series | Gaia at aa987cc |
|---|---|---|
| Product sources | `[sources.<name>]` in product.toml | accepted: validate and plan pass (good-f8, good-sd-image) |
| Image OS | `os = { provider = "buildroot", source = "<name>", fragments = [...] }` on the image | accepted: validate and plan pass; plan lists the `buildroot-host` step. With `make` absent from PATH, plan refuses: `the host gives no Buildroot setup (make, jobs, download directory)` |
| Blob keys | `[bootloader.blobs]` keys are U-Boot's make variables: capitals, digits and `_` only, e.g. `ROCKCHIP_TPL` | `ROCKCHIP_TPL` validates and plans. validate passes `tpl` and `ROCKCHIP-TPL` too; plan refuses them (`bootloader.blobs.tpl: ... is not a make variable: capitals, digits and _`). (An earlier note here said validate refused them; 10beed8's validate passes them as well) |
| Kernel defconfig | `kernel.defconfig = "defconfig"` is the architecture default; set it only to differ | accepted (the family in good-f8 and good-sd-image) |
| Kernel overlays | `kernel.overlays` names overlays by path under the kernel's device tree directory, with no extension (`rockchip/rk3566-radxa-zero-3e-gadget`) | validate accepts either form; plan accepts the name without an extension and refuses one with it: `kernel.overlays[0]: ... is not an overlay's name: a path under the kernel's device tree directory, with no extension` (refused outright at 4abf626) |
| Partition `bootable` | not a board key | validate refuses: `partitions[0].bootable: unknown key` |
| Partition content | `content = "<file>"` only on `in_table = false`, with an `offset` | validate refuses `content` on `in_table = true` (a partition's content comes from the recipe); refuses `in_table = false` with no `offset` |
| Recipe `slot` | the recipe is named `slot`; its one setting is `device` (`[images.settings] device = "/data/slots/root.img"`) | the name is accepted. `device` is required (plan: `missing; it has no default and wants a string`). With `device` set, plan still refuses without `veritysetup` on PATH: `needs its input veritysetup, and nothing gives it`. Seen with `--tool veritysetup=` unset; passing a stub makes plan pass |
| Recipe `sd-image` | the disk recipe: `[images.settings]` with `id` (required string) and optional parameters (see below) | validate and plan pass for good-sd-image (with a veritysetup stub; plan also refuses without one, through its verity step). A board with no `[disk]` table is refused by plan: `the board has no [disk] table` |
| Bogus recipe | `recipe = "nope"` | validate refuses when given the recipes (`--recipes DIR`, which the check passes): `no recipe named \`nope\`; the recipes are ...`. Without `--recipes` (and no `recipes/` beside the product) validate passes; plan refuses: `images[0].recipe: no recipe "nope"` |
| Rust component | `[build] builder = "rust"` with `[build.settings] package`, see "Rust components" | validate passes; plan needs `bwrap` and `rustc` on PATH (or the check's stand-in for `bwrap`) |
| Component build settings | the rust builder's keys in `[build.settings]` | validate does not read them; plan refuses `vendor` and unknown keys by name, accepts `profile`, `features`, `cc`, `env`, `triple`, `cpu` |
| `@here@` | in a fragment file it is that file's directory | accepted: plan shows it as the fragment's input tree (`@files.0@`) |
| Derivation | arch, CPU, kernel, U-Boot, TF-A sources, defconfigs, fragments, DTS and blobs come from the board; toolchain, packages and output format go in the image's fragments | the OS step's inputs are `board.target`, `board.kernel` and `board.bootloader` (plan output) |
| `gaia --version` | the commit is the pin | prints `gaia 2.0.6-dev`: no commit (unchanged) |

Recipes at this pin, from `recipes/` of the checkout: `slot`, `initramfs-boot`, `recovery`,
`firmware`, `data-ext4`, `sd-image`. A setting that is not a recipe parameter is refused by name.

- `firmware`: takes `level`, `threads`. The good-f8 product uses it with no settings.
- `slot`: takes `device` (required, a string: where the slot is read from, a file or a device;
  it sets the verity device), `compression`, `level`. A slot is an EROFS image with dm-verity.
- `sd-image`: takes `id` (required string: names the disk; the GUIDs come from it), `root_size`,
  `root_margin`, `data_size`, `data_margin`, `journal`, `root_name`, `data_name`, `root_role`,
  `data_role`, `partition_type`, `root_bootable` (default true), `data_bootable` (default false),
  `auto`, `compression`, `level`, `slot_path` (default `slots/root.img`: the slot's file inside the
  data partition), `slot_device` (default `/data/slots/root.img`: where the running system reads
  the slot, for its dm-verity table).
- `initramfs-boot`: takes no settings.
- `recovery`: takes `key_env`, `level`, `public_key`, `threads`.
- `data-ext4`: takes `journal`, `label`, `margin`, `size`.

What `sd-image` builds, from its recipe file (`recipes/sd-image.toml`) and the plan edges:

- The **rootfs partition** (`rootp`, ext4, name `rootfs` by default, marked bootable by default) gets
  the OS's root tree: with `os` set, the `build` input is the OS's rootfs (merged with the image's
  component layers) and feeds `root.tree`. The slot's EROFS image is built from the same tree.
- The **data partition** (`datap`, ext4, name `data`) gets the image's `data` component's tree if the
  product has one, and the slot file: the verity image is placed at `slot_path` in it
  (`verity.image -> slot.file -> slot.tree -> data.tree`). A product with no `data` component
  plans; the data partition then holds only the slot file.
- `device` has no meaning here. The slot's read path is `slot_device`. The word `device` in
  `slot` is the same thing under that recipe's name.
- Gaia's `docs/design.md` does not describe `sd-image`. The recipe file's own comment and
  `docs/prs/F7.md` (the brief it came from) are the description at this rev.
- Its `id` is the only required setting; Gaia's `examples/products/zero-3e-demo` sets it to the
  product's name. The recipe's own comment says `Untested on board.`

Partition roles at this rev (for `role`): `bootloader`, `boot`, `esp`, `rootfs`, `data`, `raw`.

A bogus recipe name is refused by validate when it is given the recipes (`--recipes DIR`), and by plan always:
`no recipe "nope"`.

## The Gaia pin

`gaia.lock` is Atlas's, written by hand: a Gaia URL and a full commit. It is not Gaia's lock. The
commit is `7db227f6648917434b14da4117150ad6ab007530` (Gaia `next`: PR #85 `gaia lock` resolves nothing a
lock already holds and works offline for full-commit refs, #86 `gaia --version` prints the commit it was
built from, #87 `--input NAME=PATH`, #88 the OS build split into kernel, bootloader and root filesystem
builds with `gaia.tree.under`, and the `system` input). Before it: `f5a32f9fe242a7fb2686e48037b6e2fe23553646`,
`aa987ccc54f13d415bc42623da30805243746ed6` (PR 52), `4abf626316a8d3f8c2bb9561e1895a0e89745f9b` and
`10beed87d9591175305997a2ebf1f88e3f5e4b2b`. To move it: change `commit`, build Gaia at the new commit,
and run `scripts/check-devices.sh --write-locks` with that binary (`GAIA_BIN=...`); the check refuses a
binary that prints another commit or none. Commit the pin and every changed `gaia.lock` together.

## Product locks

Each product's `gaia.lock` is committed beside its `product.toml`. It is written by `gaia lock` and
records the sources the product names. Tree paths in it are relative to the product's directory
(`../../components/<name>/src`), so the same lock holds in every checkout.

The plain check reads the committed locks offline: it runs `gaia lock --check`, then `gaia lock` on a copy
with git and curl replaced by programs that fail (so any request is an error), and the regenerated copy must
equal the committed lock byte for byte (this re-hashes every directory source). Then the product is planned
with the committed lock. A lock that needs the network (a source it does not hold, or a description that
changed) fails the plain check with the command below.

To write or update them after a change (this is the only mode that asks the network):

```
scripts/check-devices.sh --write-locks
```

It starts each lock from the committed one, so a source the lock already holds for what its description
says is not asked again; a new source is resolved. A host that is unreachable or rate limited is retried a
few times and then FAILS the check, and the committed lock is left as it was. A branch or tag that moved
is refreshed with `gaia lock --update SOURCE` (`SOURCE` is the name, e.g. `kernel.src`). Never edit a lock
by hand.

## Named inputs (the bench key)

A component may install a file the command line gives, `from = { input = "bench-key" }`, instead of a file
committed beside it (`gaia plan --input bench-key=PATH`). The bring-up products (`zero-3e-bringup`,
`rock-4d-bringup`, `rock-5c-bringup`, `cubie-a7-bringup`) take the bench's SSH public key that way. The check
gives every product that names `bench-key` a throwaway public key it makes in a temp dir for that run (only
the `.pub` file is kept; the private half is deleted at once). No key is committed. A product that names
an input is planned with it; gaia refuses an input nothing uses, so no other product gets one.

PR 48's rule (no description installs per-unit state or keys) is checked by a grep on `to = "..."` for
`/data/`, `/var/lib/orion`, `/etc/dropbear/`, and any path with `host_key`, `node-identity` or `.ed25519`.
`/root/.ssh/authorized_keys` (the bench key's install) is not in that pattern, and the pattern is unchanged: an
authorized_keys file for a dev-only bring-up is not a host key or a node identity. A bench input installed at
one of the listed paths would still fail the check.

## Lemnos rev

Every Lemnos component of a product (lemnosd, lemnos-ctl, lemnos-orion, lemnos-slot-init and the
`lemnos` packaging) is locked to ONE Lemnos commit: they are one program in several files, and a mix
does not boot reliably. The pin is the `ref` of each component in `components/lemnos*/component.toml`
(and `components/lemnos/component.toml` for the packaging); the product lock records the commit each
source resolved to. The check fails a product whose lock pins two Lemnos commits, and names both.
The current rev is `4a2d3b7d` (PR B18). To move it: change every `ref` above to the full commit, run
`scripts/check-devices.sh --write-locks`, and commit the components and every changed `gaia.lock`
together. The board files (`lemnos-board.toml`) follow the rev's board-file format, and `lemnos-ctl
validate` and `lemnos-ctl tree` at that rev are the check for them (run from a build of that rev).

## Running the check

```
scripts/check-devices.sh                        # devices/, then the fixture self-check (the gate runs this)
scripts/check-devices.sh --write-locks          # the network: update each product's gaia.lock, then check
scripts/check-devices.sh --root DIR             # another tree, laid out the same way, no self-check
scripts/check-devices.sh --write-locks --root DIR  # the same, for that tree
scripts/check-devices.sh --self-test            # verbose: the good fixtures pass, broken fails
```

The default run ends with the self-check: the good fixtures (`good`, `good-f8`, `good-rust`) must pass,
the broken one must fail, and a gaia that prints another commit, or none, must be refused. The good
fixture `tests/fixtures/good-sd-image` is checked with `--root`:
`scripts/check-devices.sh --root devices/tests/fixtures/good-sd-image`.

What the check does: it reads the Gaia URL and commit from `gaia.lock`. It needs a gaia binary: `$GAIA_BIN`
if set, otherwise a build from that commit, cached in `${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/`
(the first run clones Gaia from GitHub and builds it with cargo). Either binary must print
`gaia <version> (<commit>)` with the pinned commit. The recipes come from the same checkout. Every family,
board, component and product file is validated with `gaia validate` (products against the checkout's
`recipes/`), then each product's lock is checked offline (see Product locks) and the product is planned.

What it needs on the machine:

- `git` and network access to GitHub, only for `--write-locks` and the first build (Gaia's repository is public).
- `cargo` for the first build (Gaia's `rust-toolchain.toml` pins 1.99.0; rustup installs it).
- `ssh-keygen`, for the throwaway bench key (every bring-up product plans with it).
- `python3`: the python builder names it as a tool; the check passes it as `--tool python=<python3>`.
- The tools a recipe names. `slot` and `sd-image` need `veritysetup`; where it is not installed the check
  passes a stand-in for planning (a build needs the real one). A Rust component needs `bwrap` and `rustc`
  for planning (the check passes a stand-in for `bwrap`).
- `make` on PATH for a product with an `os` (the Buildroot host step).

## Known limits at the pin

These hold at Gaia `7db227f`, the pin in `gaia.lock`. Items 1 and 2 were re-run only through the products: the
sd-image products plan with the veritysetup stand-in and the Rust component with the bwrap stand-in, as at `aa987cc`;
the refusals themselves were not re-tested at `7db227f`. Item 5 is carried from `aa987cc` and was not re-run. They lift
when the pin moves.

1. **`slot` and `sd-image` need `veritysetup`.** A product on either recipe cannot be planned where
   that tool is not on PATH (or passed with `--tool veritysetup=PATH`). The check passes a stand-in
   when it is missing, so planning works; a build needs the real one (cryptsetup's tool).
   Checked at aa987cc: `slot` with `device` set and `sd-image` both refuse without it.
2. **`gaia plan` needs every tool a recipe names on PATH**, or passed with `--tool NAME=PATH`.
   Checked at aa987cc: a Rust component needs `bwrap` (the check passes a stand-in) and `rustc`; a
   Buildroot product needs `make`. `python` is needed only for a product with a python component.
3. **The gaia binary's commit is checked.** `gaia --version` prints `gaia <version> (<commit>)` at `7db227f`
   and at `f5a32f9` and `7db227f` (PR #86); the check refuses a binary that prints another commit, `-dirty`, or no commit.
   `aa987cc` and older print no commit and are refused.
4. **Gaia's `Cargo.lock` is untracked** at this commit (`git ls-files Cargo.lock` is empty at
   aa987cc; the root `.gitignore` has `/Cargo.lock`), so a build is pinned by commit only, not by
   dependency versions.
5. **`gaia validate` does not check tools, blob names beyond their form, component build settings, the store,
   or kernel overlay names.** Recipe names are checked when validate is given the recipes (`--recipes DIR`,
   which the check passes); without it, validate skips them. Plan refuses an overlay name that carries an
   extension, refuses a blob name that is not a make variable (`tpl`), and refuses a component's
   `[build.settings]` key it does not take (`vendor`), where validate accepts all three.

A board with no product is validated on its own (`gaia validate --kind board`) and listed as "in no
product yet". It is never planned, so plan-time rules (overlay names, blob names, a `[disk]`
table for an sd-image product) are not checked for it until a product names it.
