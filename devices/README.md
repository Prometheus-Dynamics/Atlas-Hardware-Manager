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
  boards/<board>/app/                  app data for the board (format set by the app's PR, not yet)
  components/<component>/component.toml  one piece of software and what it installs
  products/<product>/product.toml      which components go on which board, and the recipe
  tests/fixtures/                      examples and a broken copy, used by `--self-test`
```

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
scripts/check-devices.sh               # devices/, what the gate runs
scripts/check-devices.sh --root DIR    # another tree, laid out the same way
scripts/check-devices.sh --self-test   # good must pass, broken must fail, f8 is reported
```

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

## Limits

- Boards are checked through products. Gaia has no command that checks a board alone, so a board
  with no product (or extended family with no board) fails the check as unused.
- `gaia validate` does not exist on Gaia `next` yet. The check then uses `gaia plan`, which needs
  the lock and the tools above.
- The blob key rule and `@here@` are not checked by Gaia today; reviewers check them until F8 merges.
