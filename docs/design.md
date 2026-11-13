# Atlas `next`: the design

Atlas is the tool on your computer for every board: find it, flash it, update it and manage its
slots, watch it, and get in when it is broken. It is also where the description of every board
lives, because Atlas owns the build config.

`dev` is the frozen reference. Proposal page: https://claude.ai/artifact/EbDv4F3VqRK1oyMySECPZe

## Why a rebuild

`dev` holds three things in one repository: the host tool (about 87k lines), everything that runs
on a board (seven programs, 9,548 lines of shell, 45 systemd units), and the description of 13
boards in an old format. It reaches a board seven different ways (Orion actions, SSH commands, a
USB console, an HTTP gateway, an identity server, a flasher protocol, an agent on the SystemCore),
and slots, status, actions and readings are each written three times.

The libraries under it have been rebuilt. Orion `next` is on every board in every state. Lemnos
`next` owns board management as drivers. Gaia `next` builds from plain descriptions.

## The shape

```
  your computer                                   a board, in any state
 +---------------------------------+
 | desktop app      CLI            |             running:   Orion + lemnosd (drivers add the
 |        \        /               |                        actions: update, slots, reboot)
 |        atlas core               |   Orion
 |  inventory, jobs, events, store | <---------> recovery or freshly loaded over USB:
 |        |             |          |   one wire             the flasher image (Orion recovery
 |  board driver    robot driver   |                        build + storage drivers)
 |  (Orion client)  (read-only)    |
 |        |                        |             behind a SystemCore: the controller runs
 |  loaders: Pi USB boot, Rockchip |             Orion and forwards
 |  maskrom, K230 burn, Allwinner  |
 |  FEL, disk writer               |   USB boot ROM (only until the flasher image is running)
 +---------------------------------+

  devices/   board descriptions as data: families, boards, components, products, the lock
```

## Rules of the design

1. **One way to a board: Orion.** Every job on a board that is running, in recovery, or freshly
   loaded is an Orion action, topic or file. No SSH commands, no console commands, no HTTP
   gateway, no identity server, no protocol of Atlas's own. SSH and the recovery shell exist for
   a person, served by Orion.
2. **Atlas runs nothing on a board.** Board management (update, slots, confirm, rollback, reboot,
   USB) is Lemnos drivers. Identity, readings, logs, files and access are Orion. What Atlas ships
   for a board is data: its description, and the description of its flasher image.
3. **Our own flasher, for every chip family.** Atlas loads a board from its boot ROM itself, in
   Rust, with no vendor tool: Raspberry Pi USB boot, Rockchip maskrom, Kendryte K230 burn,
   Allwinner FEL. What it loads is the flasher image: a small initramfs with the Orion recovery
   build and the storage drivers. From that point flashing is an ordinary update over the USB
   link, with the same calls as a running board. SD cards and removable disks are written
   directly.
4. **Board descriptions are data, in `devices/`.** Gaia `next` format: a family, a board that
   extends it, components, a base OS product per board, one lock. They are not part of the app.
   The app embeds only a small generated catalog (how to recognise a board and what it looks
   like). A product such as HeliOS keeps its own product file in its own repository and locks
   `devices/` by commit.
5. **Slots.** Root slots are EROFS image files on one ext4 data partition, checked by dm-verity,
   any number side by side, no A/B. Install, set default, trial boot (falls back if not
   confirmed), guarded remove, keep-last-N with a per-update keep and per-slot pins. The only
   build output is the first-flash `.img.xz`; Atlas makes the slot bundle from it on the computer.
6. **The SystemCore works like other vendors' tuner tools.** Atlas finds the controller and
   offers to install a package on it (an `.ipk` built by Gaia): Orion and Lemnos. After that the
   controller is an ordinary Orion node: Atlas talks through it to boards plugged into it, its
   flash job keeps running if Atlas goes away, and its page shows real device information.
   Installing a product on the controller (HeliOS) is one more package offered the same way.
7. **The app is fast and plain.** It opens at once. A slow or missing device never slows it.
   Nothing blocks on a scan. No confirmation on anything reversible, and a confirmation never
   moves the layout. Health ignores robot power cycles. Nothing overflows. Mockups come before
   screens are built, and every screen is checked with real pointer clicks against a live or
   simulated device.
8. **Apps log themselves.** UI errors, every failed call and breadcrumbs go to a log file a
   support bundle includes. Nobody is asked to reproduce something.
9. **Trust.** Releases and images are signed. Atlas holds a trusted key list with owner
   information. The board checks signatures when security is on: off by default for FRC, one
   switch.
10. **Failure rule.** Never give up, backoff capped at 5 to 10 s, no default auto-off, an easy
    manual off.

## Repository layout

```
apps/desktop/         the app: UI and its Tauri side
crates/core/*         types, the driver contract, inventory, jobs, events, store, logging
crates/drivers/*      board (Orion), robot, one per boot ROM family, disk
crates/flash/*        the boot ROM protocols and image formats, no Atlas types
crates/tools/*        the CLI, the privileged disk helper
devices/              families/, boards/, components/, products/, gaia.lock, app data
testing/              simulated node, mock devices, board-run records
```

## What goes, what stays

- Goes to Lemnos drivers: the update and slot writers, slot-init, atlas-ubi's board side, USB
  gadget and route scripts, LEDs, locate, hardware self-test.
- Goes to Orion: identity, the push stream, SSH keys and host keys, the support bundle action.
- Deleted: board-agent, board-stream, board-lite, the identity servers, the flasher daemon and its
  protocol, the SystemCore USB agent and its HTTP port, the image server (Orion carries files),
  the SSH and console command layers, the gateway layer, every "pick a route" wrapper.
- Stays, rewritten to fit: the app, the core (inventory, scan, jobs), the release catalog and
  trust, Pi USB boot, Rockchip, K230 and Allwinner loaders, the disk writer and its helper, the
  robot driver, the UBI image reader.
- Stays as data, rewritten: `devices/`.

## Outside blocks

A PR that needs one waits for it; don't work around it.

| Id | Item | Stops |
|---|---|---|
| X1 | Lemnos I5: board-management drivers and the long-job form | C5, C6, C7 on a real board |
| X2 | Orion's USB link run on a board (its C13) | D6 on a real board |
| X3 | Gaia F8 and F7: the OS and the board in the graph, the disk image | building B-series boards (writing them waits on nothing) |
| X4 | A HeliOS product on `next` | E5 |
| X5 | Orion and Lemnos packaged for the SystemCore (Gaia's ipk recipe is merged) | E2 on a real controller |
