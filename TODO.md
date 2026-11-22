# Atlas `next`: the PR list

Every PR has a brief in `docs/prs/`. `python3 scripts/pr-status.py` shows what is ready.
Needs named `X<n>` are outside blocks, listed in `docs/design.md`.

Status of every item, checked on 2026-11-22 against `origin/next` f9daf4c2 and every `origin/pr/*` branch:
- **Done**: the brief's deliverables are on `next`; the landing commits are named.
- **Partly**: names what is missing, including work that exists only on an unmerged branch.
- **Not started**: nothing of it is on any branch.

Totals: 56 done, 18 partly, 14 not started (88 items; the duplicate D6 row is gone). The six items added on 2026-11-22
(B42-B46, D19) are not started.

## A. Foundation

The core every other group builds on: types, the driver contract, inventory, jobs, logging, the catalog and trust.

- [x] [A1](docs/prs/A1.md) Types and the driver contract (wave 1; needs nothing). **Done** on next: 8538d00f
- [x] [A2](docs/prs/A2.md) Inventory and store (wave 2; needs A1). **Done** on next: 69cbde49
- [x] [A3](docs/prs/A3.md) Scan, jobs and events (wave 2; needs A1). **Done** on next: 0da5ff01
- [x] [A4](docs/prs/A4.md) The simulated node and mock devices (wave 2; needs A1). **Done** on next: 59e5f4d1
- [x] [A5](docs/prs/A5.md) Logging (wave 1; needs nothing). **Done** on next: 31049753
- [x] [A6](docs/prs/A6.md) The device catalog (wave 2; needs A1). **Done** on next: 67fd5cb7
- [x] [A7](docs/prs/A7.md) Releases and trust (wave 2; needs A1). **Done** on next: 73c8795d
- [x] [A8](docs/prs/A8.md) Core follow-ups from the first wave (wave 3; needs A1, A6, D1). **Done** on next: 8c5e3e92
- [x] [A9](docs/prs/A9.md) The simulated node out of the drivers, and one driver for every family (wave 4; needs A1, A4). **Done** on next: 3311ceaa
- [x] [A10](docs/prs/A10.md) A key that says what it is, a board's Lemnos state, and the board session as its own crate (wave 4; needs A9, C1). **Done** on next: 231c763f, 7ffeda37
- [x] [A12](docs/prs/A12.md) The session keeps the key it first saw (wave 5; needs A10, C8). **Done** on next: 0e610ce6, 71303a10
- [x] [A14](docs/prs/A14.md) The session carries calls while a transfer runs: long operations stepped between calls (needs A12; the real interleaving waits for Orion R2). **Done** on next: 07c88735, 698a5933. Note: The real interleaving waits for Orion R2, as the brief says. Not verified on a board.
- [ ] [A15](docs/prs/A15.md) A board's lease is kept while Atlas holds something on it (a hold guard, a keep-alive only while holds exist); absent `hello.offers` is NOT REPORTED, not "offers nothing" (needs A14; the renewal is `describe` at Lemnos I6h, the offers cap at M8c). **Partly**: The lease is switched OFF: crates/board/session/src/lease.rs:48 pub const LEASE_IN_FORCE: bool = false; on next. The keep-alive is not in effect at the locked Lemnos.

## B. Board descriptions

Every board as Gaia `next` data in `devices/`. Starts first; the Raze and the ZERO 3E before the rest.

- [x] [B1](docs/prs/B1.md) Layout, schema and checks (wave 1; needs nothing). **Done** on next: 319fa57b, a026f8a2
- [x] [B2](docs/prs/B2.md) BCM2712 family and the Raze (wave 2; needs B1). **Done** on next: 9fe1b312
- [x] [B3](docs/prs/B3.md) RK35xx family and the ZERO 3E (wave 2; needs B1). **Done** on next: ef505140. Note: devices/products/zero-3e-base was deleted on next by Z2 (30859db4, 2f9c1268, PR #125 merge 1ab54a74), folded into zero-3e-bench. Recorded …
- [x] [B4](docs/prs/B4.md) ROCK 5C and ROCK 4D (wave 3; needs B3). **Done** on next: bf2a7b42
- [x] [B5](docs/prs/B5.md) Allwinner A733 family and the Cubie A7A, A7S, A7Z (wave 2; needs B1). **Done** on next: c55abfa5
- [ ] [B6](docs/prs/B6.md) Luckfox Pico Pro and Aura (wave 2; needs B1). **Partly**: devices/families/rv11xx is not on next. The brief's family is missing from the baseline. Not in PR-merged form.
- [ ] [B7](docs/prs/B7.md) K230D Zero and AI-CAM (wave 2; needs B1). **Partly**: devices/families/k230 is not on next. The K230D bring-up products are not on next.
- [ ] [B8](docs/prs/B8.md) SystemCore and robot devices (wave 2; needs B1). **Partly**: devices/products/systemcore-package is not on next.
- [ ] [B9](docs/prs/B9.md) The flasher image, per family (wave 3; needs B2, B3). **Partly**: on origin/pr/B9 (b2efaf42) as flasher-bcm2712 and flasher-rk35xx, not on next. Brief path devices/products/flasher does not exist; per-family products are on the unmerged branch only.
- [x] [B10](docs/prs/B10.md) App data for every board (wave 3; needs A6, B1). **Done** on next: c2720db3
- [x] [B11](docs/prs/B11.md) Bring-up products for the ROCK 5C and ROCK 4D (wave 4; needs B4). **Done** on next: 5edb759c. Note: Brief path devices/pending/rock-*-bringup is not used; a38e2070 (B22) moved them out of pending to products. Recorded as the moved location.
- [ ] [B12](docs/prs/B12.md) Allwinner A733: a candidate source tree, and a bring-up product for the Cubie A7 boards (wave 4; needs B5). **Partly**: Cubie bring-up product deleted on next by PADS bf46b1c8 (PR #138, "cubie bring-up deleted"). devices/pending/cubie-a7-bringup absent.
- [ ] [B13](docs/prs/B13.md) The bring-up and base consoles, and the SSH host key on the data partition (wave 5; needs B11). **Partly**: Brief's devices/pending/zero-3e-bringup, rock-5c-bringup, rock-4d-bringup, cubie-a7-bringup decision set: zero-3e-bringup deleted by Z2 2f9c1268; cubie-a7-bringup …
- [x] [B14](docs/prs/B14.md) The flasher's built-in kernel options, per family (wave 4; needs B2, B3, B9). **Done** on next: e33f224b
- [ ] [B15](docs/prs/B15.md) A USB device-mode port in each board's description, and stable gadget addresses on the bring-up products (wave 5; needs B11, Lemnos Q2e). **Partly**: devices/pending/zero-3e-bringup/components/dev-zero-3e-usb-extlinux and its product.toml: deleted on next by Z2 2f9c1268 (bench product replaces it). Fixed g_ether …
- [x] [B18](docs/prs/B18.md) Lemnos lock to 4a2d3b7d: every Lemnos pin moved, one-rev check per product, ZERO 3E port validated, Raze refusals on record (needs B15; Lemnos 4a2d3b7d). **Done** on next: 1aad8733, 203b5096
- [ ] [B19](docs/prs/B19.md) Cubie A7 (Allwinner A733): vendor sources pinned, and a bring-up that shows life without a console (wave 5; needs B12, B13, B15). **Partly**: devices/pending/cubie-a7-bringup/{board,product.toml,os.fragment,components/...} deleted on next by PADS bf46b1c8. The bring-up that shows life without a console is not …
- [x] [B20](docs/prs/B20.md) The Raze board file, written to Lemnos Q2d (6905b681); takes effect when the lock moves past 4a2d3b7d (needs B18; Lemnos 6905b681). **Done** on next: 6aebe13b, 03ef52a8
- [x] [B21](docs/prs/B21.md) VBUS on each board's device-mode port, prepared for Lemnos Q2f (ZERO 3E complete; ROCK 5C held by Lemnos Q2g; ROCK 4D held; needs B15, B18, B19; Lemnos Q2f at 96080d36). **Done** on next: 3f5cef6e, 9174ecd6. Note: ROCK 5C and ROCK 4D stay held (Lemnos Q2g, held), as the brief states. Not a gap in this PR.
- [x] [B22](docs/prs/B22.md) Gaia lock to 7db227f (next head): the checker refuses a binary of another commit, the plain check is offline, the bring-up products leave pending/ with a named bench-key input (needs B17, B18, B19). **Done** on next: 68e7b43b, 05ec67c6
- [x] [B24](docs/prs/B24.md) Bring-up USB network: Gaia's usb-gadget-network (configfs NCM, pinned Gaia 4509f6b) for the ZERO 3E and the Cubie A7S; addresses from the board id and serial; the board-id check (needs B15, B16, B19, B22). **Done** on next: 65845581, e53afe34
- [x] [B16](docs/prs/B16.md) Each family's kernel trimmed to what its boards use, stripping on, and dtbs built with symbols (wave 5; needs B2, B3, B14; Lemnos 4c6678f3 kernel doc). **Done** on next: 5ff282f6, d2ec7bec
- [x] [B25](docs/prs/B25.md) Every kernel fragment line takes, checked in the gate (wave 6; needs B16, B24). **Done** on next: b8b71845, 68606f5b
- [ ] [B42](docs/prs/B42.md) The board answers for its hostname by mDNS on lemnos0 (needs B24, B41). **Not started**
- [ ] [B43](docs/prs/B43.md) check-devices checks the board's DHCP server against the lemnos0 scheme (needs B24). **Not started**
- [ ] [B44](docs/prs/B44.md) No root console with an empty password in a product image (needs B13). **Not started**
- [ ] [B45](docs/prs/B45.md) check-devices runs lemnos-ctl validate on every board file (needs B20, B18). **Not started**
- [ ] [B46](docs/prs/B46.md) build-product.sh --build-mode and --medium (needs B22). **Not started**

## C. One board driver

Every job on a board through the Orion client, against a simulated node.

- [x] [C1](docs/prs/C1.md) Connect, discover and identify (wave 3; needs A1, A4). **Done** on next: f3d6a065, 45f4b79c. Note: Branch-only origin/pr/C1 head 34dee858 is not an ancestor of next; its own commits are board-data, not the driver, and the driver content …
- [x] [C2](docs/prs/C2.md) Readings and logs (wave 4; needs C1, A10). **Done** on next: dcc54b1e, 5173ef08
- [x] [C3](docs/prs/C3.md) Actions (wave 4; needs C1, A10). **Done** on next: ee2cbc6a, d6873262
- [x] [C4](docs/prs/C4.md) Files (wave 4; needs C1, A10). **Done** on next: 65fcd9d6, b7cfe61c
- [x] [C5](docs/prs/C5.md) Slots and updates (wave 5; needs C3, C4, A7, X1). **Done** on next: 4167bbea, 931a9b11
- [x] [C6](docs/prs/C6.md) Trial and confirm watch (wave 6; needs C5). **Done** on next: d4980aa4, 17afe516
- [ ] [C7](docs/prs/C7.md) Board data: backup, restore, files (wave 5; needs C4). **Not started**
- [x] [C8](docs/prs/C8.md) SSH and the recovery shell (wave 4; needs C1, A10). **Done** on next: 544d0d03, 2817b81e
- [x] [C9](docs/prs/C9.md) NetworkTables viewer (wave 4; needs C1). **Done** on next: 2b171f8c
- [x] [C10](docs/prs/C10.md) Identity facts as Lemnos publishes them (wave 4; needs C1, A10). **Done** on next: c760cfcd, 17ae39ac
- [x] [C11](docs/prs/C11.md) The device description as Lemnos sends it (wave 4; needs C2). **Done** on next: 53a8ee94, 698030b7

## D. Flashing

Our own loader for every chip family, then the flasher image, then an ordinary update.

- [x] [D1](docs/prs/D1.md) USB transport, hotplug and ids (wave 2; needs A1). **Done** on next: 3c28a32a
- [x] [D2](docs/prs/D2.md) Raspberry Pi USB boot (wave 3; needs D1). **Done** on next: 0309989b
- [x] [D3](docs/prs/D3.md) Rockchip maskrom and loader (wave 3; needs D1). **Done** on next: 177a93c7
- [x] [D4](docs/prs/D4.md) Kendryte K230 burn (wave 3; needs D1). **Done** on next: 917cd9bf
- [x] [D5](docs/prs/D5.md) Allwinner FEL (wave 3; needs D1). **Done** on next: f131517e
- [x] [D16](docs/prs/D16.md) FEL: an AArch64 thunk for the A733 (needs D5; untested on board). **Done** on next: ec718494, 742282e8. Note: Untested on a board, as the TODO line says.
- [ ] [D19](docs/prs/D19.md) atlas-rpiboot boots a real BCM2712; first, since D6 stands on it (needs D2; board step on Raze #2). **Not started**
- [ ] [D6](docs/prs/D6.md) Flash a blank board (wave 6; needs C5, B9, D2, D3, D10, D11, D12, D13, D15). **Not started**
- [x] [D7](docs/prs/D7.md) Disk writer and its helper (wave 2; needs A1). **Done** on next: 36a82285
- [x] [D18](docs/prs/D18.md) Writing a card on Windows and macOS, as far as it can be proven here (needs D7; host test: docs/host-tests/disk-writer.md). **Done** on next: 6516193f, e035e81b. Note: Windows/macOS proof is only as far as the brief says (CI in G9).
- [x] [D8](docs/prs/D8.md) UBI images for NAND boards (wave 2; needs A1). **Done** on next: bcb99680
- [x] [D9](docs/prs/D9.md) Slot bundles from the image (wave 3; needs A7). **Done** on next: 612af214
- [x] [D10](docs/prs/D10.md) Fastboot client (wave 3; needs D1). **Done** on next: 6ad6634f
- [x] [D11](docs/prs/D11.md) Read a slot out of the first-flash image (wave 4; needs D9). **Done** on next: 5481c8d5
- [x] [D12](docs/prs/D12.md) Find a board's Orion link over USB by its interface string (wave 4; needs D1, A8). **Done** on next: d184b333
- [x] [D17](docs/prs/D17.md) atlas-usb hands out a claimed link as a read/write pair (wave 5; needs D12). **Done** on next: bc21a127, 2a94889f
- [x] [D13](docs/prs/D13.md) slot.json schema 3, and the bundle made from the image (wave 5; needs D9, D11). **Done** on next: ed39d3cf
- [x] [D14](docs/prs/D14.md) Verity keys in the slot manifest, and the bundle from an image in one call (wave 6; needs D13). **Done** on next: 13d83259
- [x] [D15](docs/prs/D15.md) The first-flash manifest, made by Atlas from the image (wave 6; needs D11, D13). **Done** on next: f370e114, c9aba065. Note: Commit subjects carry "PROVISIONAL" markers, as the brief expects.

## E. Robot network and SystemCore

The robot's own devices, and Atlas installing its package on the controller.

- [x] [E1](docs/prs/E1.md) The robot's own devices (wave 3; needs A1, A3). **Done** on next: 780542d8
- [ ] [E2](docs/prs/E2.md) Install on a SystemCore (wave 4; needs E1, C1, B8). **Not started**
- [ ] [E3](docs/prs/E3.md) Through the SystemCore (wave 7; needs E2, D6, X6). **Not started**
- [ ] [E4](docs/prs/E4.md) The SystemCore's page data (wave 5; needs E2, C2). **Not started**
- [ ] [E5](docs/prs/E5.md) Install a product on a SystemCore (wave 5; needs E2, X4). **Not started**

## F. App and CLI

Mockups first, then a faster, cleaner app on the new core.

- [ ] [F1](docs/prs/F1.md) Design language and mockups (wave 1; needs nothing). **Partly**: on origin/pr/F1 (288c070e) and pr/F-app, not merged into next.
- [ ] [F2](docs/prs/F2.md) The app shell (wave 3; needs A2, A3, A5). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F3](docs/prs/F3.md) Device list and discovery (wave 4; needs F1, F2, C1, A6). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F4](docs/prs/F4.md) The board page (wave 6; needs F3, C2, C3, C5, C7, C8). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F5](docs/prs/F5.md) The flash flow (wave 7; needs F3, D6, D7). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F6](docs/prs/F6.md) Robot and SystemCore pages (wave 6; needs F3, E1, E2, E4). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F7](docs/prs/F7.md) Settings, keys and security (wave 4; needs F2, A7). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.
- [ ] [F8](docs/prs/F8.md) The CLI (wave 7; needs C5, D6, E2). **Not started**
- [ ] [F9](docs/prs/F9.md) Packaging and self-update (wave 4; needs F2). **Partly**: on origin/pr/F-app (head d1f44d60), not merged into next.

## G. Board runs and finish

Every job with real clicks on real boards, then docs and the lock.

- [ ] [G1](docs/prs/G1.md) Raze, direct (wave 8; needs F4, F5, B2, B9, X1, X2, X3; board run). **Not started**
- [ ] [G2](docs/prs/G2.md) Raze behind the SystemCore (wave 9; needs G1, F6, E3, X5, X6; board run). **Not started**
- [ ] [G3](docs/prs/G3.md) ZERO 3E (wave 8; needs F5, B3, D3; board run). **Not started**
- [ ] [G4](docs/prs/G4.md) ROCK 5C (wave 9; needs G3, B4; board run). **Not started**
- [ ] [G5](docs/prs/G5.md) Cubie A7 (wave 8; needs F5, B5, D5; board run). **Not started**
- [ ] [G6](docs/prs/G6.md) Luckfox (wave 8; needs F5, B6, D8; board run). **Not started**
- [ ] [G7](docs/prs/G7.md) K230D (wave 8; needs F5, B7, D4; board run). **Not started**
- [ ] [G8](docs/prs/G8.md) Docs, lock, hand-over (wave 10; needs G1, G2, G3, F8, F9). **Partly**: docs/guide/ absent on every ref; the hand-over docs are not written.
- [x] [G9](docs/prs/G9.md) What must build on Windows and macOS is checked in CI (needs nothing; native CI on windows-latest and macos-latest; scripts/check-portable.sh). **Done** on next: 8d586b49, 9e1e9328
