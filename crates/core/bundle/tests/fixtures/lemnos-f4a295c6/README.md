# Lemnos slot bundle fixture (Lemnos f4a295c6)

`slot-a1.bundle` is a slot bundle written by Lemnos's own writer, `lemnos-ctl bundle`, built from
Lemnos at `f4a295c611972ba422df20a78112b9fb409c47d0` (the image's Lemnos pin, `devices/`). Atlas's
reader (`atlas-bundle`, `tests/lemnos_fixture.rs`) must read and verify it. Nothing here is a product
or board key.

**This is a TEST key.** `test-key.pub` is the public half of a throwaway Ed25519 key made by
`atlas-next bench-key` for this fixture. Its seed was never committed and is not needed to read the
bundle. Key id `600ebf633600aabc22786f2320e2fa57afe30bec95deda750c6b32883fa58c44`.

Size: the bundle is 16384 bytes (SHA-256 `91805ea4ed30ba1095ecfec39c19d4be362f3eb18e5e903060af59ff9bac30f4`).
Contents: `slot.json`, `slot.sig` (a 108-byte `LMNSSIG1` record), `rootfs.erofs` (one 4096-byte erofs
block, so no hash tree), and the five boot files (`kernel`, `initramfs`, `device-tree`, `board-overlay`,
`cmdline.txt`). The kernel is 3000 zero bytes: a shape for the format, not a bootable kernel.

## How it was made (exact inputs, all from the Lemnos checkout at f4a295c6)

1. Board file: `testing/boards/reference.toml` (copied as `board.toml`).
2. SoC map directory: `socs/`.
3. Root image: `mkfs.erofs` on a directory holding one README, padded to 4096 bytes.
4. Payload: `lemnos-ctl tree --socs socs board.toml --slot a1 --cmdline-slot payload/cmdline`, with
   `payload/kernel`, `payload/initramfs`, `payload/device-tree`, `payload/board-overlay` written first.
5. Verity: `lemnos_slot_store::verity::tree_of` appends the hash tree to the image and writes the
   `key value` table (the same function Lemnos's tests use), into `verity.txt`.
6. The bundle, signed with the test key's seed (`bench.seed`, not committed):

```text
lemnos-ctl bundle --layout-of board.toml --socs socs --slot a1 --payload payload \
  --rootfs rootfs.erofs --verity-table verity.txt --product fixture-os --version 1.0 \
  --key-file bench.seed --expect-key-id 600ebf633600aabc22786f2320e2fa57afe30bec95deda750c6b32883fa58c44 \
  --out slot-a1.bundle
```

Lemnos's own check, which passes on this file:

```text
lemnos-ctl bundle --verify slot-a1.bundle --layout-of board.toml --socs socs --pubkey test-key.pub
```

Refresh when the Lemnos pin moves (`devices/` and the `lemnos-slot-store` rev in the workspace
`Cargo.toml`, which `scripts/check-devices.sh` holds equal): rebuild `lemnos-ctl` from the new pin, make
the bundle again with the same inputs and command, and replace this file. The Atlas test fails if the
format moves, so a refresh that the reader refuses is a real finding.
