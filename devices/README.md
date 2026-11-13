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
  products/<product>/product.toml      which components go on which board, and the recipe
  usb-ids.json                         USB ids shared by every board (see "App data" below)
  tests/fixtures/                      examples and a broken copy, used by `--self-test`
```

## App data

Two kinds of file here are for the app, not for Gaia. The check never reads them as Gaia files.

- `boards/<board>/app/app.json`: one per board. `model` is the board directory's name. The rest
  is the display name, the USB id keys, an optional hostname prefix, and optional picture and
  case files, paths relative to `app/`.
- `usb-ids.json`: the USB ids, keyed by name. Each entry has `state` (`download`, `storage` or
  `gadget`), `vid`, `pid` and `name`. A board's `app.json` refers to these by key.

`crates/core/catalog` is the reader, and it is the authority on the exact keys. The app embeds the
catalog it generates from these files; the files themselves are not embedded.

Every family, board, component and product is its own directory, holding its description file.
Other files may sit beside the description (assets, a component's `src/`, a fragment, `app/`). The
check validates only the description files, but it does require that every family, board and
component is named by a product or a board, because Gaia reads a file only through one of those.

## Gaia's format (what Gaia `next` reads today)

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
- **product.toml**: `version = 1`, `name`, `description`, `[components]` (name = path to a
  component.toml), `[[images]]` with `name`, `board` (path to a board.toml), `components` (names),
  `data` (names), `recipe`, `budget`, and `[images.settings]` for the recipe's own settings.

## F8 format points, and what Gaia accepts today

These come from Gaia `next`'s F8, which is not merged. Each row was tried against Gaia at the
locked commit (`e90ee03`). Write to the F8 column; today's column says what the check would say.

| Point | Format for B2/B3 | Gaia at e90ee03 |
|---|---|---|
| Product sources | `[sources.<name>]` in product.toml | refused: `sources: unknown key` |
| Image OS | `os = { provider = "buildroot", source = "<name>", fragments = [...] }` on the image | refused: `images[0].os: unknown key` |
| Blob keys | `[bootloader.blobs]` keys are U-Boot's make variables: capitals, digits and `_` only, e.g. `ROCKCHIP_TPL` | accepted, and so is lowercase `tpl`: no rule yet |
| Kernel defconfig | `kernel.defconfig = "defconfig"` is the architecture default; set it only to differ | accepted |
| Kernel overlays | `kernel.overlays` is refused for an OS build for now | not testable: no OS build exists yet |
| Slot recipe | the recipe is named `slot` | refused: `no recipe "slot"`; today's name is `ab-slot`, which also needs `veritysetup`, not installed here |
| `@here@` | in a fragment file it is that file's directory | not in Gaia's source; not testable |
| Derivation | arch, CPU, kernel, U-Boot, TF-A sources, defconfigs, fragments, DTS and blobs come from the board; toolchain, packages and output format go in the image's fragments | F8 |

So B2/B3 cannot write an OS image until F8 merges. Until then the check fails on any product using
`os` or `sources`, which is the signal. The fixture `tests/fixtures/f8/` is that F8 form; the
self-test prints its result and does not gate on it.

## The Gaia pin

`gaia.lock` is Atlas's, written by hand: a Gaia URL and a full commit. It is not Gaia's lock. The
commit is `origin/next` of Gaia at the time of writing (`e90ee03`); it will move when F8 merges.
To move it: change `commit`, run `scripts/check-devices.sh`, and commit both together.

## Running the check

```
scripts/check-devices.sh               # devices/, then the fixture self-check (the gate runs this)
scripts/check-devices.sh --root DIR    # another tree, laid out the same way, no self-check
scripts/check-devices.sh --self-test   # verbose: good must pass, broken must fail, f8 is printed
```

The default run ends with the self-check: the good fixture must pass and the broken one must fail,
so the gate shows the checker still catches a broken file.

The check does the following. It reads the Gaia URL and commit from `gaia.lock`. It then needs a
gaia binary: `$GAIA_BIN` if set (used as given, and its commit is not checked because gaia prints
no commit); otherwise a build from that commit, cached in
`${ATLAS_GAIA_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/atlas/gaia}/<commit>/`. The first run clones
Gaia from GitHub and builds it with cargo; later runs reuse `bin/gaia`. The recipes come from the
same checkout. It runs `gaia validate` if the binary has that command and `gaia plan` otherwise.

What it needs on the machine:

- `git` and network access to GitHub (Gaia's repository is public, so no token is needed).
- `cargo` for the first build (Gaia's `rust-toolchain.toml` pins 1.99.0; rustup installs it).
- `python3`: the python builder names it as a tool, and `gaia plan` needs every tool a recipe
  names. The check passes it as `--tool python=<python3>`.
- The tools a recipe names. The `firmware` recipe names none this check needs. `ab-slot` names
  `veritysetup`, so it cannot be planned where that is not installed.

Gaia writes a tree path relative to a product's directory only when the path lies under it. The
components live in `devices/components/`, outside a product's directory, so `gaia lock` would write
this checkout's absolute paths. For that reason the check locks each product in a temporary file
(`gaia plan --lock`) and never reads a product lock committed beside it. If a product's `gaia.lock`
is committed and does not match, the check fails and says so. Do not commit product locks while
their components sit outside the product directory.

## Known limits at the locked Gaia commit

These hold at Gaia `e90ee03`, the pin in `gaia.lock`. They lift when the pin moves.

1. **A product's lock cannot be committed.** Gaia writes a tree path relative to the product's
   directory only when the path lies under it. Components sit in `devices/components/`, outside
   every product directory, so the lock would hold this checkout's absolute paths. The check locks
   each product in a temp file (`gaia plan --lock`) and fails if a committed one does not match.
2. **`ab-slot` needs `veritysetup`**, which is not on every machine. A product that uses it cannot
   be planned where that tool is missing. The check does not install it.
3. **`gaia plan` needs every tool a recipe names on PATH**, or passed with `--tool NAME=PATH`. The
   check passes `--tool python=<python3>` for the python builder. Other tools a recipe names must
   be on PATH on the machine (and on CI).
4. **`$GAIA_BIN` is trusted, not verified.** Gaia prints a version but no commit, so the check
   cannot confirm the binary was built from the pinned commit. The cached build is built from it
   by the script itself.
5. **Gaia's `Cargo.lock` is untracked**, so a build is pinned by commit only, not by dependency
   versions.

Gaia also has no `gaia validate` yet; the check uses `gaia plan` until it does.

The F8 format points are refused by today's Gaia. Write B2/B3 descriptions to the F8 form above;
the check fails on them until the pin moves:

- product `[sources]`: refused, `sources: unknown key`;
- image `os = {...}`: refused, `images[0].os: unknown key`;
- recipe `slot`: refused, `no recipe "slot"` (today's is `ab-slot`, see point 2);
- a lowercase blob key is accepted, and no rule enforces the capitals rule yet;
- `kernel.overlays` for an OS build and `@here@` cannot be tested until an OS build exists.
  Reviewers check these until F8 merges.

A board with no product is reported as unused. Gaia has no command that checks a board alone.
