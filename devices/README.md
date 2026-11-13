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

A board directory holds `board.toml`, or, when it is app-only, only `app/` (see App data).

## App data

Two kinds of file here are for the app, not for Gaia. The check never reads them as Gaia files.

- `boards/<board>/app/app.json`: one per board. `model` is the board directory's name. The rest
  is the display name, the USB id keys, an optional hostname prefix, and optional picture and
  case files, paths relative to `app/`.
- `usb-ids.json`: the USB ids, keyed by name. Each entry has `state` (`download`, `storage` or
  `gadget`), `vid`, `pid` and `name`. A board's `app.json` refers to these by key.

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
- **component.toml**: `version = 1`, `name`, `description`, `[sources.<name>]`, `[build]`
  (`builder`, `source`, `[build.settings]`), `[[install]]`, `[[services]]`, `[[users]]`,
  `[[groups]]`, `[package]` (for an `.ipk`).
- **product.toml**: `version = 1`, `name`, `description`, `[sources.<name>]` (the OS source, see
  below), `[components]` (name = path to a component.toml), `[[images]]` with `name`, `board` (path
  to a board.toml), `components` (names), `os` (see below), `data` (names), `recipe`, `budget`, and
  `[images.settings]` for the recipe's own settings.

## The product form (Gaia F8, at the pin)

A product is one image. Its OS comes from a source the product names; the board gives the kernel,
U-Boot and TF-A; the product's OS fragment gives the rest. A minimal product that `gaia validate`
and `gaia plan` accept at the pin is `tests/fixtures/good-f8/products/f8-product/product.toml`:

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

A real product names a Buildroot git source (`git` + `ref`) instead of the fixture's path. The
`slot` recipe is the one B2/B3 want; see point 2 of the known limits before using it.

The fragment file is Buildroot config (`BR2_...=y`). `@here@` in it is that file's directory.

## F8 format points, as Gaia 70a5133 treats them

Each row was tried with Gaia `70a5133` (`gaia 2.0.6-dev`). "validate" is `gaia validate`, "plan"
is `gaia plan` with the product's lock and `--tool python=...`. Only what was seen is written here.

| Point | Format for B-series | Gaia at 70a5133 |
|---|---|---|
| Product sources | `[sources.<name>]` in product.toml | accepted: validate and plan pass (good-f8) |
| Image OS | `os = { provider = "buildroot", source = "<name>", fragments = [...] }` on the image | accepted: validate and plan pass; plan lists the `buildroot-host` step (it needs `make` on PATH) |
| Blob keys | `[bootloader.blobs]` keys are U-Boot's make variables: capitals, digits and `_` only, e.g. `ROCKCHIP_TPL` | `ROCKCHIP_TPL` plans. `tpl` and `ROCKCHIP-TPL` pass validate but plan refuses them: `is not a make variable: capitals, digits and _` |
| Kernel defconfig | `kernel.defconfig = "defconfig"` is the architecture default; set it only to differ | accepted (the family in good-f8) |
| Kernel overlays | `kernel.overlays` is refused for an OS build for now | validate accepts; plan refuses: `device tree overlays are not given to an OS build yet` |
| Recipe `slot` | the recipe is named `slot`; its one setting is `device` (`[images.settings] device = "/data/slots/root.img"`) | the name is accepted (validate, and plan gets past it). Plan refuses without `veritysetup` on PATH: `needs its input veritysetup, and nothing gives it`. With a stub passed as `--tool veritysetup=...` the plan passed. `veritysetup` is not installed on this machine |
| `@here@` | in a fragment file it is that file's directory | accepted: plan (`--json`) resolves it to the fragment's directory |
| Derivation | arch, CPU, kernel, U-Boot, TF-A sources, defconfigs, fragments, DTS and blobs come from the board; toolchain, packages and output format go in the image's fragments | the OS step's inputs are `board.target`, `board.kernel` and `board.bootloader` (plan output) |

Recipes at this pin, from `recipes/` of the checkout: `slot`, `initramfs-boot`, `recovery`,
`firmware`, `data-ext4`. What plan reports for each (a setting that is not a recipe parameter is
refused by name):

- `firmware`: takes `level`, `threads`. The good-f8 product uses it with no settings.
- `slot`: takes `device` (required, a string), `compression`, `level`.
- `initramfs-boot`: takes no settings.
- `recovery`: takes `key_env`, `level`, `public_key`, `threads`.
- `data-ext4`: takes `journal`, `label`, `margin`, `size`.

A bogus recipe name passes validate and plan refuses it: `no recipe "nope"`.

## The Gaia pin

`gaia.lock` is Atlas's, written by hand: a Gaia URL and a full commit. It is not Gaia's lock. The
commit is `70a513341635b3df7a1971d1522c8eeedff68ac1` (Gaia `next`, after F8 and relative lock
paths). To move it: change `commit`, run `scripts/check-devices.sh --write-locks`, and commit the pin
and every regenerated `gaia.lock` together.

## Product locks

Each product's `gaia.lock` is committed beside its `product.toml`. It is written by `gaia lock` and
records the sources the product names. Tree paths in it are relative to the product's directory
(`../../components/<name>/src`), so the same lock holds in every checkout.

To write or update them after a change:

```
scripts/check-devices.sh --write-locks
```

The check fails when a product has no `gaia.lock` beside it, or when that lock differs from what
`gaia lock` writes now. The message names the command above. Never edit a lock by hand.

## Running the check

```
scripts/check-devices.sh                        # devices/, then the fixture self-check (the gate runs this)
scripts/check-devices.sh --write-locks          # regenerate every product's gaia.lock, then check
scripts/check-devices.sh --root DIR             # another tree, laid out the same way, no self-check
scripts/check-devices.sh --write-locks --root DIR  # the same, for that tree
scripts/check-devices.sh --self-test            # verbose: both good fixtures pass, broken fails
```

The default run ends with the self-check: the two good fixtures (`good`, `good-f8`) must pass and
the broken one must fail, so the gate shows the checker still catches a broken file.

The check does the following. It reads the Gaia URL and commit from `gaia.lock`. It then needs a
gaia binary: `$GAIA_BIN` if set (used as given, and its commit is not checked because gaia prints
no commit); otherwise a build from that commit, cached in
`${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/`. The first run clones
Gaia from GitHub and builds it with cargo; later runs reuse `bin/gaia`. The recipes come from the
same checkout. With a binary that has `gaia validate` (70a5133 does), it validates every family,
board, component and product file (with `--kind` from the directory). Then, for each product, it
locks the product into a temp file, requires the committed `gaia.lock` to match it, and plans the
product with the committed lock. A binary without `gaia validate` plans every product only.

What it needs on the machine:

- `git` and network access to GitHub (Gaia's repository is public, so no token is needed).
- `cargo` for the first build (Gaia's `rust-toolchain.toml` pins 1.99.0; rustup installs it).
- `python3`: the python builder names it as a tool, and `gaia plan` needs every tool a recipe
  names. The check passes it as `--tool python=<python3>`.
- The tools a recipe names. `firmware`, `initramfs-boot`, `recovery` and `data-ext4` need none that
  this check passes. `slot` needs `veritysetup`, so it cannot be planned where that is not installed.
- `make` on PATH for a product with an `os` (the Buildroot host step).

## Known limits at the pin

These hold at Gaia `70a5133`, the pin in `gaia.lock`, and each was tested with that commit's binary
(`gaia 2.0.6-dev`). They lift when the pin moves.

1. **`slot` needs `veritysetup`.** A product on `recipe = "slot"` cannot be planned where that tool
   is not on PATH (or passed with `--tool veritysetup=PATH`). The check passes no such tool, so
   the fixtures use `firmware`. The check does not install it.
2. **`gaia plan` needs every tool a recipe names on PATH**, or passed with `--tool NAME=PATH`. The
   check passes `--tool python=<python3>` for the python builder. Other tools a recipe names must
   be on PATH on the machine (and on CI). Buildroot products need `make` on PATH.
3. **`$GAIA_BIN` is trusted, not verified.** `gaia --version` prints `gaia 2.0.6-dev` and no commit,
   so the check cannot confirm the binary was built from the pinned commit. The cached build is built
   from it by the script itself.
4. **Gaia's `Cargo.lock` is untracked** at this commit, so a build is pinned by commit only, not by
   dependency versions.
5. **`gaia validate` does not look at recipe names, tools or the store.** A bogus recipe name passes
   validate, and only `gaia plan` refuses it. Blob keys and `kernel.overlays` are the same: validate
   passes them, plan refuses them.

A board with no product is validated on its own (`gaia validate --kind board`) and listed as "in no
product yet". It is never planned, so plan-time rules (recipe names, blob keys, `kernel.overlays`)
are not checked for it until a product names it.
