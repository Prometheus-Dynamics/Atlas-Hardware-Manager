# Host test: writing a card on Windows and macOS

Run by a person on real machines. It is the proof that the disk writer's
Windows and macOS paths work, which nothing in CI or on the Linux bench can
show. It needs:

- a Windows 10 machine, a Windows 11 machine, and a Mac (Apple silicon or Intel);
- a spare SD card (its contents are erased) and a USB card reader;
- a small test image: any `.img` of 64 MiB or less. Make one with
  `fsutil file createnew test.img 67108864` (Windows) or
  `mkfile 64m test.img` (macOS), then put some bytes in it (anything else is fine).
- the Rust toolchain of this repository (`rust-toolchain.toml` pins it), installed by `rustup`.

**Nothing here touches a disk the person did not choose.** Every write step names
the card by its path, and every step is run on the spare card only. A step that
shows a disk other than the card, or the system disk, is refused by the tool; stop
and send the output.

## What the steps prove, and what each unlocks

The Windows and macOS paths are guarded by one constant in
`crates/flash/disk/src/platform.rs`: `WINDOWS_PROOF` and `MACOS_PROOF`. While it
is `Unproven`, a write to a real disk is refused before anything is listed or
touched, and the message says so. Listing and the refusals work without it.

| Step | Proves | Unlocks |
|---|---|---|
| 1 Build | the helper builds on this system | nothing |
| 2 List | disks are found, with removable, bus and size; system disks are marked | the list column of the proof |
| 3 Refusal without proof | the write is refused until the proof is set | nothing (it confirms the guard) |
| 4 Proof set locally | (build only) | the steps below |
| 5 System disk refused | the system disk is never chosen | safety rule 1 |
| 6 Elevation | a write without administrator rights is refused (Windows) | the elevation proof |
| 7 Wrong card refused | a changed size or serial stops the write | the identity proof |
| 8 Busy volume refused | a volume in use blocks the lock (Windows) | the lock proof |
| 9 Write and verify | the image lands and reads back equal | the write proof |
| 10 Finish and eject | the card is flushed, read again and ejected | the finish proof |

When steps 1 to 10 pass on a system, the person sends the outputs, and the proof
constant for that system is set to `Proven` in one commit, with the output in the
PR. Until then the constant stays `Unproven`. Never set it on a machine's
evidence alone: one system at a time, each with its own run.

## Setup

From the repository root (the folder with `Cargo.toml`), in a terminal opened as
a normal user:

```
cargo build -p atlas-helper
```

On Windows the helper is `target\debug\atlas-helper.exe`; on macOS `target/debug/atlas-helper`.
Below, `HELPER` stands for that path. Three values come from step 2's output:
`CARD` is the spare card's `path`, `SIZE` is its `size_bytes`, and `SYSTEM` is the
`path` of the disk marked `"system":true`.

## Step 2: list the disks (no administrator, read only)

```
HELPER list
```

Expected: one `host` line, then one `disk` line per disk. Example shape:

```
{"type":"host","platform":"Windows","proof":"Unproven","acknowledged":false}
{"type":"disk","path":"\\\\.\\PhysicalDrive0","name":"PhysicalDrive0","removable":false,"system":true,...,"verdict":"refused: ... running system ..."}
{"type":"disk","path":"\\\\.\\PhysicalDrive2","name":"PhysicalDrive2","removable":true,"usb":true,"system":false,...,"verdict":"would be written"}
```

On macOS the paths are `/dev/diskN`. Check: the internal system disk is `refused`
(it holds `/`); the card reader is `would be written`; the card's size matches the
card. Send the whole output. If a card reader's disk shows `"removable":false`,
say so: the reader reports a fixed disk and the write will be refused. That is the
intended rule, and the output is the evidence.

Also run `Get-Disk | Format-List` (Windows, PowerShell) or
`diskutil info /dev/diskN` (macOS) for the card and send that too, so the list
can be compared with the OS's own view.

## Step 3: the write is refused until the proof is set

With the card inserted and its path from step 2 (`CARD`):

```
HELPER write --device CARD --image test.img --expected-size SIZE
```

Expected: exit code 1 and one line on stderr:
`atlas-helper: writing a disk on Windows has not been proven on this system yet; ...`
(macOS says `macOS`). The card is not touched. Send the output.

## Step 4: set the proof locally (build only, not committed)

Edit `crates/flash/disk/src/platform.rs`: change `WINDOWS_PROOF` (or `MACOS_PROOF`)
from `Proof::Unproven` to `Proof::Proven`, then `cargo build -p atlas-helper` again.
Do not commit this edit. Steps 5 to 10 run on this build.

## Step 5: the system disk is refused

Use the system disk's path from step 2 (`SYSTEM`), with the card's size as
`--expected-size`:

```
HELPER write --device SYSTEM --image test.img --expected-size SIZE
```

Expected: exit code 1, `... holds the running system or a boot partition and will not be written`.
Send the output. Repeat with the `--device` of an internal non-removable disk if
there is a second one.

## Step 6: elevation (Windows)

Run step 9's command from a normal (not administrator) terminal first:

```
HELPER write --device CARD --image test.img --expected-size SIZE
```

Expected: exit code 1, `writing a disk on Windows needs administrator rights: ...`.
Then open PowerShell **as administrator** (Start, type `powershell`, Run as administrator,
approve the User Account Control prompt) and run the same command there. The
administrator run goes on to step 9. Send both outputs.

On macOS there is no elevation check in the helper: the system's authorisation
prompt (from `authopen`) appears when the raw card is opened in step 9. Note
whether the prompt appeared, what it said, and whether approving it let the write
go on. Send a note of it.

## Step 7: a card that changed is refused

From the administrator terminal, with the right path:

```
HELPER write --device CARD --image test.img --expected-size 1
HELPER write --device CARD --image test.img --expected-size SIZE --expected-serial WRONG
```

Expected: both exit 1; the first says `changed size since it was chosen`, the
second `is not the card that was chosen (its serial is not WRONG)`. Send both.
Nothing is written in either run.

## Step 8: a card in use is refused (Windows)

Open a file on one of the card's volumes in Explorer (for example a text file
you create on the card's drive letter, and keep it open in an editor). Then run
step 9's command. Expected: exit 1 with `... is in use and could not be unmounted`
(the lock fails). Close the file, and continue. Send the output. This shows the
volume lock refuses a busy volume rather than writing under it.

macOS: with a file open on a volume of the card (`touch /Volumes/NAME/x` then
`tail -f` it in another terminal), the unmount in step 9 must refuse with the
`diskutil` message. Send the message.

## Step 9: the write and the read-back

With the card idle:

```
HELPER write --device CARD --image test.img --expected-size SIZE --progress progress.jsonl
```

Expected: the progress file gets `writing` lines counting up to the image size,
and the last line is `{"type":"done","report":{...,"verified":true,"sha256":"..."}}`.
Exit 0. Send the last line and the final progress line.

Cross-check the card itself. On macOS:
`sudo dd if=/dev/rdiskN bs=1m count=64 | shasum -a 256` and `shasum -a 256 test.img`
(compare the first 64 MiB, or use the image's own size with `count=` set to it in MiB).
On Windows the helper's read-back is the proof; compare the `sha256` in `done`
with `certutil -hashfile test.img SHA256`.

## Step 10: finish and eject

The `done` line means the write was flushed, read back and the disk finished. For
the eject: the card should show as ejected (Windows: the drive letter is gone
from Explorer; macOS: the disk is gone from Finder). If the line instead says
`the image is written and verified; finishing the disk failed: ...`, the image is
on the card and verified, and the eject failed: send that message.

## What to send back

For each step: the exact command, the exit code, and the output (the JSON lines
or the stderr line), plus the OS version (`winver` or `sw_vers`) and the card
reader's model. Note any prompt that appeared and what it said. Send to the PR
for D18; do not send card contents.
