# Atlas `next`: the PR list

Every PR has a brief in `docs/prs/`. `python3 scripts/pr-status.py` shows what is ready.
Needs named `X<n>` are outside blocks, listed in `docs/design.md`.

## A. Foundation

The core every other group builds on: types, the driver contract, inventory, jobs, logging, the catalog and trust.

- [ ] [A1](docs/prs/A1.md) Types and the driver contract (wave 1; needs nothing)
- [ ] [A2](docs/prs/A2.md) Inventory and store (wave 2; needs A1)
- [ ] [A3](docs/prs/A3.md) Scan, jobs and events (wave 2; needs A1)
- [ ] [A4](docs/prs/A4.md) The simulated node and mock devices (wave 2; needs A1)
- [ ] [A5](docs/prs/A5.md) Logging (wave 1; needs nothing)
- [ ] [A6](docs/prs/A6.md) The device catalog (wave 2; needs A1)
- [ ] [A7](docs/prs/A7.md) Releases and trust (wave 2; needs A1)
- [ ] [A8](docs/prs/A8.md) Core follow-ups from the first wave (wave 3; needs A1, A6, D1)
- [ ] [A9](docs/prs/A9.md) The simulated node out of the drivers, and one driver for every family (wave 4; needs A1, A4)
- [ ] [A10](docs/prs/A10.md) A key that says what it is, a board's Lemnos state, and the board session as its own crate (wave 4; needs A9, C1)
- [ ] [A12](docs/prs/A12.md) The session keeps the key it first saw (wave 5; needs A10, C8)
- [ ] [A14](docs/prs/A14.md) The session carries calls while a transfer runs: long operations stepped between calls (needs A12; the real interleaving waits for Orion R2)
- [ ] [A15](docs/prs/A15.md) A board's lease is kept while Atlas holds something on it (a hold guard, a keep-alive only while holds exist); absent `hello.offers` is NOT REPORTED, not "offers nothing" (needs A14; the renewal is `describe` at Lemnos I6h, the offers cap at M8c)

## B. Board descriptions

Every board as Gaia `next` data in `devices/`. Starts first; the Raze and the ZERO 3E before the rest.

- [ ] [B1](docs/prs/B1.md) Layout, schema and checks (wave 1; needs nothing)
- [ ] [B2](docs/prs/B2.md) BCM2712 family and the Raze (wave 2; needs B1)
- [ ] [B3](docs/prs/B3.md) RK35xx family and the ZERO 3E (wave 2; needs B1)
- [ ] [B4](docs/prs/B4.md) ROCK 5C and ROCK 4D (wave 3; needs B3)
- [ ] [B5](docs/prs/B5.md) Allwinner A733 family and the Cubie A7A, A7S, A7Z (wave 2; needs B1)
- [ ] [B6](docs/prs/B6.md) Luckfox Pico Pro and Aura (wave 2; needs B1)
- [ ] [B7](docs/prs/B7.md) K230D Zero and AI-CAM (wave 2; needs B1)
- [ ] [B8](docs/prs/B8.md) SystemCore and robot devices (wave 2; needs B1)
- [ ] [B9](docs/prs/B9.md) The flasher image, per family (wave 3; needs B2, B3)
- [ ] [B10](docs/prs/B10.md) App data for every board (wave 3; needs A6, B1)
- [ ] [B11](docs/prs/B11.md) Bring-up products for the ROCK 5C and ROCK 4D (wave 4; needs B4)
- [ ] [B12](docs/prs/B12.md) Allwinner A733: a candidate source tree, and a bring-up product for the Cubie A7 boards (wave 4; needs B5)
- [ ] [B13](docs/prs/B13.md) The bring-up and base consoles, and the SSH host key on the data partition (wave 5; needs B11)
- [ ] [B14](docs/prs/B14.md) The flasher's built-in kernel options, per family (wave 4; needs B2, B3, B9)
- [ ] [B15](docs/prs/B15.md) A USB device-mode port in each board's description, and stable gadget addresses on the bring-up products (wave 5; needs B11, Lemnos Q2e)
- [ ] [B18](docs/prs/B18.md) Lemnos lock to 4a2d3b7d: every Lemnos pin moved, one-rev check per product, ZERO 3E port validated, Raze refusals on record (needs B15; Lemnos 4a2d3b7d)
- [ ] [B19](docs/prs/B19.md) Cubie A7 (Allwinner A733): vendor sources pinned, and a bring-up that shows life without a console (wave 5; needs B12, B13, B15)
- [ ] [B20](docs/prs/B20.md) The Raze board file, written to Lemnos Q2d (6905b681); takes effect when the lock moves past 4a2d3b7d (needs B18; Lemnos 6905b681)
- [ ] [B21](docs/prs/B21.md) VBUS on each board's device-mode port, prepared for Lemnos Q2f (ZERO 3E complete; ROCK 5C held by Lemnos Q2g; ROCK 4D held; needs B15, B18, B19; Lemnos Q2f at 96080d36)
- [ ] [B22](docs/prs/B22.md) Gaia lock to 7db227f (next head): the checker refuses a binary of another commit, the plain check is offline, the bring-up products leave pending/ with a named bench-key input (needs B17, B18, B19)
- [ ] [B24](docs/prs/B24.md) Bring-up USB network: Gaia's usb-gadget-network (configfs NCM, pinned Gaia 4509f6b) for the ZERO 3E and the Cubie A7S; addresses from the board id and serial; the board-id check (needs B15, B16, B19, B22)
- [ ] [B16](docs/prs/B16.md) Each family's kernel trimmed to what its boards use, stripping on, and dtbs built with symbols (wave 5; needs B2, B3, B14; Lemnos 4c6678f3 kernel doc)

## C. One board driver

Every job on a board through the Orion client, against a simulated node.

- [ ] [C1](docs/prs/C1.md) Connect, discover and identify (wave 3; needs A1, A4)
- [ ] [C2](docs/prs/C2.md) Readings and logs (wave 4; needs C1, A10)
- [ ] [C3](docs/prs/C3.md) Actions (wave 4; needs C1, A10)
- [ ] [C4](docs/prs/C4.md) Files (wave 4; needs C1, A10)
- [x] [C5](docs/prs/C5.md) Slots and updates (wave 5; needs C3, C4, A7, X1)
- [x] [C6](docs/prs/C6.md) Trial and confirm watch (wave 6; needs C5)
- [ ] [C7](docs/prs/C7.md) Board data: backup, restore, files (wave 5; needs C4)
- [ ] [C8](docs/prs/C8.md) SSH and the recovery shell (wave 4; needs C1, A10)
- [ ] [C9](docs/prs/C9.md) NetworkTables viewer (wave 4; needs C1)
- [ ] [C10](docs/prs/C10.md) Identity facts as Lemnos publishes them (wave 4; needs C1, A10)

## D. Flashing

Our own loader for every chip family, then the flasher image, then an ordinary update.

- [ ] [D1](docs/prs/D1.md) USB transport, hotplug and ids (wave 2; needs A1)
- [ ] [D2](docs/prs/D2.md) Raspberry Pi USB boot (wave 3; needs D1)
- [ ] [D3](docs/prs/D3.md) Rockchip maskrom and loader (wave 3; needs D1)
- [ ] [D4](docs/prs/D4.md) Kendryte K230 burn (wave 3; needs D1)
- [ ] [D5](docs/prs/D5.md) Allwinner FEL (wave 3; needs D1)
- [ ] [D16](docs/prs/D16.md) FEL: an AArch64 thunk for the A733 (needs D5; untested on board)
- [ ] [D6](docs/prs/D6.md) Flash a blank board (wave 6; needs C5, B9, D2, D3, D10, D11, D12, D13)
- [ ] [D6](docs/prs/D6.md) Flash a blank board (wave 6; needs C5, B9, D2, D3, D10, D11, D12, D13, D15)
- [ ] [D7](docs/prs/D7.md) Disk writer and its helper (wave 2; needs A1)
- [ ] [D8](docs/prs/D8.md) UBI images for NAND boards (wave 2; needs A1)
- [ ] [D9](docs/prs/D9.md) Slot bundles from the image (wave 3; needs A7)
- [ ] [D10](docs/prs/D10.md) Fastboot client (wave 3; needs D1)
- [ ] [D11](docs/prs/D11.md) Read a slot out of the first-flash image (wave 4; needs D9)
- [ ] [D12](docs/prs/D12.md) Find a board's Orion link over USB by its interface string (wave 4; needs D1, A8)
- [ ] [D17](docs/prs/D17.md) atlas-usb hands out a claimed link as a read/write pair (wave 5; needs D12)
- [ ] [D13](docs/prs/D13.md) slot.json schema 3, and the bundle made from the image (wave 5; needs D9, D11)
- [ ] [D14](docs/prs/D14.md) Verity keys in the slot manifest, and the bundle from an image in one call (wave 6; needs D13)
- [ ] [D15](docs/prs/D15.md) The first-flash manifest, made by Atlas from the image (wave 6; needs D11, D13)

## E. Robot network and SystemCore

The robot's own devices, and Atlas installing its package on the controller.

- [ ] [E1](docs/prs/E1.md) The robot's own devices (wave 3; needs A1, A3)
- [ ] [E2](docs/prs/E2.md) Install on a SystemCore (wave 4; needs E1, C1, B8)
- [ ] [E3](docs/prs/E3.md) Through the SystemCore (wave 7; needs E2, D6, X6)
- [ ] [E4](docs/prs/E4.md) The SystemCore's page data (wave 5; needs E2, C2)
- [ ] [E5](docs/prs/E5.md) Install a product on a SystemCore (wave 5; needs E2, X4)

## F. App and CLI

Mockups first, then a faster, cleaner app on the new core.

- [ ] [F1](docs/prs/F1.md) Design language and mockups (wave 1; needs nothing)
- [ ] [F2](docs/prs/F2.md) The app shell (wave 3; needs A2, A3, A5)
- [ ] [F3](docs/prs/F3.md) Device list and discovery (wave 4; needs F1, F2, C1, A6)
- [ ] [F4](docs/prs/F4.md) The board page (wave 6; needs F3, C2, C3, C5, C7, C8)
- [ ] [F5](docs/prs/F5.md) The flash flow (wave 7; needs F3, D6, D7)
- [ ] [F6](docs/prs/F6.md) Robot and SystemCore pages (wave 6; needs F3, E1, E2, E4)
- [ ] [F7](docs/prs/F7.md) Settings, keys and security (wave 4; needs F2, A7)
- [ ] [F8](docs/prs/F8.md) The CLI (wave 7; needs C5, D6, E2)
- [ ] [F9](docs/prs/F9.md) Packaging and self-update (wave 4; needs F2)

## G. Board runs and finish

Every job with real clicks on real boards, then docs and the lock.

- [ ] [G1](docs/prs/G1.md) Raze, direct (wave 8; needs F4, F5, B2, B9, X1, X2, X3; board run)
- [ ] [G2](docs/prs/G2.md) Raze behind the SystemCore (wave 9; needs G1, F6, E3, X5, X6; board run)
- [ ] [G3](docs/prs/G3.md) ZERO 3E (wave 8; needs F5, B3, D3; board run)
- [ ] [G4](docs/prs/G4.md) ROCK 5C (wave 9; needs G3, B4; board run)
- [ ] [G5](docs/prs/G5.md) Cubie A7 (wave 8; needs F5, B5, D5; board run)
- [ ] [G6](docs/prs/G6.md) Luckfox (wave 8; needs F5, B6, D8; board run)
- [ ] [G7](docs/prs/G7.md) K230D (wave 8; needs F5, B7, D4; board run)
- [ ] [G8](docs/prs/G8.md) Docs, lock, hand-over (wave 10; needs G1, G2, G3, F8, F9)
- [ ] [G9](docs/prs/G9.md) What must build on Windows and macOS is checked in CI (needs nothing; native CI on windows-latest and macos-latest; scripts/check-portable.sh)
