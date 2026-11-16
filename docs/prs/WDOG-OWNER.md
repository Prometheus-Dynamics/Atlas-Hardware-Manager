Planned, not built: no image was built or booted for this PR. UNVERIFIED on a board: nothing here has run on hardware.

# WDOG-OWNER: one watchdog owner per slot product; arming checked for every product

**Built config NOT CHECKED until a built tree is passed.** Without one, the systemd side reads the product's inputs only,
and every product's line says so: `inputs say Lemnos; built config unverified`. To check the image, pass its system
tree (Gaia's `<image>.system` merge tree, the root of what the image holds under etc/ and usr/lib/) with
`ATLAS_BUILT_TREES="NAME=SYSTEM_TREE ..."` (scripts/check-devices.sh reads it) or `--built NAME=SYSTEM_TREE`
(scripts/check-watchdog-owner.sh). Gaia's output layout is not known to this tree (UNVERIFIED); D1 must pass its
`<image>.system` tree. U-Boot's resolved config is passed the same way: `ATLAS_BUILT_BOOTLOADERS` or
`--built-bootloader NAME=DIR`, DIR holding `u-boot.config` beside `u-boot.bin` in the bootloader output tree.

## The rule (scripts/check-watchdog-owner.sh)

Two questions per product.

**Owners: exactly one feeds the hardware watchdog.** Never both, never neither.

| Owner | Counts when |
|---|---|
| Lemnos | the board file its board names (`[descriptions] lemnos`) has a `[[devices]]` block with `driver = "hw-watchdog"` and `keepalive = true`, AND a `[[hardware]]` block with `controller = "watchdog"`, AND the product lists `lemnosd` in its `[components]`. A keepalive with no lemnosd is declared but not fed: not an owner. |
| systemd | the effective `RuntimeWatchdogSec` is armed (not empty, `0`, `0s`, `off`, `no`, `false`, `infinity`), or the kernel command line says `systemd.watchdog-sec=N` with N armed. Read in systemd's order: main file, then drop-ins by name, /etc over /run over /usr/lib. |
| feeder | any other file the product installs that writes `/dev/watchdog*` on a line that is not a comment. |

**Arming: does the boot chain arm the watchdog?** An armed watchdog with no owner is a reset loop: a FINDING for any
product. Only explicit lines in the tree count:

- `CONFIG_WATCHDOG_AUTOSTART=y` in a U-Boot fragment (families/<family>/assets/*u-boot*, or a board's);
- a `wdt` command line in a boot script (`boot.cmd` or a `*.cmd` beside a board or family);
- the Lemnos slot init in the initramfs (`initramfs = [... "lemnos-slot-init"]`); its timeout is in Lemnos, not in this
  tree (UNVERIFIED).

Where the answer depends on resolved U-Boot defaults (not in the tree, and Gaia's `--config-only` does not resolve
U-Boot's .config) the output is `bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)`. A product with no
U-Boot in the tree (raze, a Pi firmware blob) prints `bootloader arming: NOT CHECKED (no U-Boot fragment or boot script
in the tree for this board: ...)`. An explicit `# CONFIG_WATCHDOG_AUTOSTART is not set` prints `not armed`.

## Slot product and what a FINDING is

A slot product's recipe is `slot`, `slot-boot` or `slot-disk`. Its owner problems are FINDINGs. An sd-image product is not
a slot product: its owner state is one NOTE line, not a gate failure. Its arming finding is still a FINDING.
`SLOT_RECIPES` in scripts/check-devices.sh gains `slot-disk` (a plain omission: version-rule.sh names it as a slot recipe;
`slot_chain_check` is the only user of that list, so the chain check now also covers slot-disk products; no product on
next has that recipe).

## Results on next (9d5990fc, the merged base), the real tree

Run: `scripts/check-watchdog-owner.sh` (exit 0, no FINDING).

| Product | Recipe | Owner (inputs) | Arming | Result |
|---|---|---|---|---|
| raze-base | slot-boot | Lemnos keepalive, boards/raze/lemnos-board.toml:131, lemnosd installed | armed by Lemnos slot init, products/raze-base/product.toml:41 (fed by Lemnos); Pi firmware NOT CHECKED | `inputs say Lemnos; built config unverified` |
| zero-3e-bringup | sd-image | systemd, components/bringup-alive/files/10-watchdog.conf:24 | U-Boot NOT CHECKED | NOTE: systemd owner |
| rock-5c-bringup | sd-image | systemd, same file:24 | U-Boot NOT CHECKED | NOTE: systemd owner |
| rock-4d-bringup | sd-image | systemd, same file:24 | U-Boot NOT CHECKED | NOTE: systemd owner |
| zero-3e-base | sd-image | none | U-Boot NOT CHECKED | NOTE: no watchdog owner (not a FINDING; not fixed here) |
| rock-5c-base | sd-image | none | U-Boot NOT CHECKED | NOTE: no watchdog owner (not a FINDING; not fixed here) |

rock-4d-base is deleted in this PR (below).

## Results on the bench branches (read-only; origin/pr/Z2, pr/R1, pr/C1 extracted with git archive, not changed)

Run against each branch's devices/ tree with this PR's rule, before any merge of this PR into them.

| Branch | Product | Result |
|---|---|---|
| Z2 | raze-base | `inputs say Lemnos; built config unverified`; armed by slot init, fed |
| Z2 | zero-3e-bench (slot-disk) | `inputs say Lemnos`; armed by slot init and by U-Boot (below), fed by Lemnos |
| Z2 | zero-3e-base | NOTE: no owner; keepalive at boards/radxa-zero-3e/lemnos-board.toml:91 nothing feeds (no lemnosd); **FINDING, arming**: U-Boot autostart (below) |
| Z2 | rock-5c-base | NOTE: no owner; **FINDING, arming** (below) |
| Z2 | rock-4d-base | NOTE: no owner; **FINDING, arming** (below); deleted by this PR |
| Z2 | rock-4d-bringup, rock-5c-bringup | NOTE: systemd owner; U-Boot armed, fed by systemd |
| R1 | raze-base | `inputs say Lemnos; built config unverified`; armed, fed |
| R1 | rock-5c-bench (slot-disk) | `inputs say Lemnos`; armed by slot init and U-Boot, fed by Lemnos |
| R1 | rock-5c-base | NOTE: no owner; keepalive at boards/radxa-rock-5c/lemnos-board.toml:62 nothing feeds; **FINDING, arming** |
| R1 | zero-3e-base | NOTE: no owner; **FINDING, arming** |
| R1 | rock-4d-base | NOTE: no owner; **FINDING, arming** |
| R1 | zero-3e-bringup, rock-4d-bringup | NOTE: systemd owner; U-Boot armed, fed by systemd |
| C1 | raze-base | `inputs say Lemnos; built config unverified`; armed, fed |
| C1 | cubie-a7s-bench (slot-disk) | `inputs say Lemnos`; armed by slot init, fed |
| C1 | zero-3e-bench | NOTE: systemd owner (components/bench-watchdog/files/10-watchdog.conf:3) |
| C1 | zero-3e-base, rock-5c-base, rock-4d-base | NOTE: no owner each; U-Boot NOT CHECKED |

**Finding for the coordinator (not pushed; reported before push):** the rk35xx U-Boot fragment on **pr/Z2 and pr/R1** states
`CONFIG_WDT=y`, `CONFIG_WATCHDOG=y`, `CONFIG_WATCHDOG_AUTOSTART=y` (families/rk35xx/assets/u-boot.fragment:59-61 on those
branches). That arms the watchdog at U-Boot, so every rk35xx product without an owner resets in a loop: zero-3e-base,
rock-5c-base, rock-4d-base on those branches. On next the fragment has no WDT line (`grep -c WATCHDOG` = 0), so next has no
such FINDING. The rule does not decide which is right: the owner-less products need an owner or the U-Boot line goes.

## Deleted: rock-4d-base (every reference, checked with git grep over the whole tree)

- `devices/products/rock-4d-base/` (`product.toml`, `os.fragment`, `gaia.lock`): deleted (git rm).
- `devices/products/rock-4d-bringup/product.toml`: its `os` fragments were `["../../products/rock-4d-base/os.fragment", "os.fragment"]`, now `["os.fragment"]`; the description and the settings comment no longer name the base.
- `devices/products/rock-4d-bringup/os.fragment`: the base's lines (158 lines) moved in above the bring-up section; the base's `BR2_SYSTEM_DHCP="eth0"` line dropped (the bring-up's `BR2_SYSTEM_DHCP=""` is the value; bringup_rule refuses two values); the base's false watchdog comment replaced: this product has no lemnosd, its one owner is systemd (bringup-alive).
- Not referenced by: scripts/, .github/workflows/next.yml, Gaia examples (none in the tree), tests, fixtures.
- Historical PR records still name it: docs/prs/B16.md:29, B22.md:72,106,134, B31.md:23,291, B32.md:36,63, L2d.md:93, PADS.md:89. These are records of past states and were left unchanged; say so if they should be annotated.

## Found, not fixed here (listed for the coordinator)

- devices/products/zero-3e-base/os.fragment:20-21 and devices/products/rock-5c-base/os.fragment:20-21 say "lemnosd feeds the hardware watchdog itself", while neither product lists lemnosd in its components (products/zero-3e-base/product.toml:17-19, products/rock-5c-base/product.toml:17-19). The same false claim the rock-4d-base file had. Not edited: Z2 and R1 delete those products. The rule reports them as "no watchdog owner" (NOTE).
- The U-Boot autostart line on pr/Z2 and pr/R1 (above).
- boards/radxa-zero-3e/lemnos-board.toml and boards/radxa-rock-5c/lemnos-board.pending.toml on the branches: keepalives that nothing feeds (see the branch table).

## Self-test

`scripts/check-watchdog-owner.sh --self-test` (run also by scripts/check-devices.sh's self-check, so by the gate). It copies
devices/ and refuses, by name, or passes:

- the real tree: raze-base's line is exactly `inputs say Lemnos; built config unverified`; zero-3e-bringup has the systemd owner;
  zero-3e-base is one NOTE line, not a FINDING; no FINDING in the tree;
- neither (slot): raze-base without its keepalive: `no watchdog owner`, and the slot init's arm with no owner is a FINDING;
- both (slot): the keepalive and an overlay drop-in: `two owners`, naming both;
- one systemd (slot): the keepalive removed, the drop-in alone: passes, the arm fed by systemd;
- RuntimeWatchdogSec=0: off, not an owner;
- a feeder unit beside the keepalive: a second owner;
- a keepalive nothing feeds on an sd-image product: one NOTE line naming it; on a slot product: a FINDING;
- slot-disk, no owner: a FINDING;
- armed, no owner (any product): a U-Boot autostart line, a `wdt` boot-script command, a built U-Boot config: a FINDING;
- armed with an owner (zero-3e-bringup with the same U-Boot line): a NOTE, not a FINDING;
- a built system tree that disagrees with the inputs: a FINDING (two owners and a disagreement);
- an agreeing built tree: `built config checked`, not `unverified`;
- an unresolved U-Boot: `bootloader arming: NOT CHECKED (resolved U-Boot config needs a build)`.

## Gate

`scripts/gate.sh` as a named unit, one run. See the report for its last lines and whether it is green.

## Not in this PR

- No Gaia build was run. The built trees are read only when passed.
- No Lemnos or Orion rev moved.
