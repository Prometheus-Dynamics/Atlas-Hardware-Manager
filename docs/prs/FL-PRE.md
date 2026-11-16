# FL-PRE: the non-wire prerequisites of the flasher

Status: sim-only. Nothing here has been run against a board or a real USB device. Every
claim about hardware is marked UNVERIFIED.

Coordinator ruling: FL-A waits for Lemnos's wire crate. This PR does only what does not
depend on it: the common outcome type, Deaf versus Absent in the usb layer and its sim,
port-path selection with an identity-checked open, and the multi-block xz reader tests
that Gaia's threaded encoder needs.

## Out of scope (FL-A proper, after Lemnos merges its wire crate)

- The Lemnos wire crate, and any frame, timing, hash or compare rule.
- The single data-driven boot-ROM driver that replaces the four chip driver contracts.
- Blobs named by chip, stage and version, and refusal of an unknown wire version.
- The FEL thunk move to a Lemnos blob.
- Lemnos host logic merged into `crates/flash` (K230 first, then MR1).
- The fastboot `open` (`crates/flash/fastboot`): U-Boot re-enumerates with its own id, so
  it selects by port only, through `await_device`. Not converted here.
- `identify` in the allwinner and k230 drivers: read-only, opens by location, not checked
  against an expected identity. Left as it was.

## The outcome type (`crates/flash/usb/src/outcome.rs`)

`Outcome<T>` is `Done(T)`, `Refused(Refused)`, `Deaf(Deaf)`, `Absent(Absent)` or
`TimedOut(TimedOut)`. `Stop = Outcome<Infallible>` is the carried form. Each non-Done case
holds what was observed, as typed fields.

- Refused { by: Party (Device | Host), reason: Reason (Code(u32) | WrongIdentity { location,
  expected, found } | Message(String)) }. A device answer that has no typed form (a
  protocol message, a stall, fastboot's text) is a `Message`. Only `WrongIdentity` is
  produced by the host, before anything is sent.
- Deaf { port, expected: Option<UsbId>, sent, waited, may_have_applied }.
- Absent { port, looked_for: Option<UsbId>, during, waited, may_have_applied }.
- TimedOut { port, phase, bound, may_have_applied }.

Definitions (also in the module doc):

- **Deaf**: a device IS enumerated at the port with the expected identity, and it does not
  answer the protocol within the bound. "Present but silent."
- **Absent**: no device with the expected identity is on the port. Nothing is enumerated
  there, or it left the bus during the exchange. A device that vanished is Absent, however
  long the transfer had been pending.
- **TimedOut**: a bounded phase ran out of time while the expected device was on the port.
  Produced when the device is there but cannot be opened within its wait.

The bus listing taken at the time of the error decides Deaf versus Absent: the usb layer
relists the port after a timeout (`observe_stop`, `crates/flash/usb/src/select.rs`).

`may_have_applied` is true when a write or a start was sent and its completion was not
seen. `Refused` never claims it.

Display gives one plain line per case, for the CLI and jobs.

### The carrier in `atlas_driver::DriverError`

One new variant: `DriverError::Flash(atlas_usb::Stop)` (`crates/core/driver/src/error.rs`).
`atlas-driver` now depends on `atlas-usb`, which has no Atlas dependencies, so no cycle.
Jobs and the CLI read the case from the value, never from the text.

### What `Unknown` maps from

`DriverError::Unknown` ("not tried again") is unchanged. The job engine's rule is now (in
`crates/core/jobs/src/call.rs`):

- `Unknown`, or a flash stop with `may_have_applied`, ends the job as Unknown and is not
  tried again.
- Incompatible, Unsupported, or a flash refusal, is settled: the job fails with its words.
- Everything else (Deaf, Absent or TimedOut that did not apply) is retried on backoff, as
  any transient failure is.

Before this PR a flash stop had no classification; it would have been retried, which for
an applied write would send it twice. Covered by three job tests.

## Port paths and the identity-checked open

- `PortPath` (`crates/flash/usb/src/port.rs`): `bus-p1.p2...`, one to seven ports, each
  1 to 255, bus 1 to 65535. Parse, Display and validation, with a typed error for each part.
- `UsbBus::open_at(location, expected, options)`: opens only if the device's ids are
  `expected`. The default checks the listing; the nusb backend reads the ids from the open
  device (`device_descriptor()`) before the interface is claimed
  (`crates/flash/usb/src/nusb_backend.rs`). A different identity is
  `UsbError::WrongIdentity { location, expected, found }`.
  UNVERIFIED on hardware: the nusb path has only been compiled; no test ran against a real
  device, as the brief requires.
- `open_expected(bus, port, Expect::{Exactly(id) | AnyIdentity}, options, wait)`
  (`select.rs`): selects by port. It waits up to `wait` for the expected device. A different
  identity on the port is refused when the wait ends (wait zero: at once). A re-enumeration on
  the same port follows the port, so a boot ROM handing over to its stage is selected by
  the port and the stage's new id.
- Every job now opens by port with an identity check, through the four paths the brief
  named:
  - `allwinner::FelDriver::flash(candidate, expected)`
  - `k230::K230Driver::start_image(location, expected, image, address)` and `await_restart`
    (AnyIdentity: the image decides which stage answers)
  - `atlas_rockchip::flow::run_usbplug(bus, port, expected, ...)` (the loader-mode device
    after the download is AnyIdentity)
  - `atlas_rpiboot::boot_device(bus, location, expected, ...)` (round two is AnyIdentity)
  The driver rejects an expected id that is not one of its own ids, and a candidate with
  no valid port, before any device is touched.

## Mapping table: old → new, per driver

| Driver / flow | Old | New |
|---|---|---|
| allwinner `flash`, open | `usb_error`: Access→Unsupported, Other→Other, else Unreachable | `open_expected`: Refused(Host, Message) for access; Refused(Host, WrongIdentity); Absent; TimedOut |
| allwinner `flash`, FEL transfer | `usb_error` (`FlashError::Usb`) | `Flash(observe_stop)`: Deaf / Absent / Refused(Device, Message) for a stall or protocol message |
| allwinner `flash`, non-USB errors (`Layout`, `Table`, `NotFel`, `WrongSoc`, `ReturnToFel`, `Readback`) | Incompatible / Other | unchanged |
| allwinner `identify` | `usb_error` | unchanged (read-only, not in scope) |
| k230 `start_image`, open | `unreachable`: Access→Unreachable, Protocol→Incompatible, else Unreachable | `open_expected`: as above |
| k230 `start_image`, stage query and image load | `unreachable` | `Flash(observe_stop)`; `may_have_applied` true for the load |
| k230 `start_image`, burner / unknown stage, bad address | Incompatible | unchanged |
| k230 `await_restart` | `unreachable` on `await_device`'s Timeout | Absent (nothing returned) or TimedOut (present, not openable) |
| rockchip `run_usbplug` (no driver caller) | `UsbResult<Box<dyn UsbIo>>` | `Outcome<Opened>`; download errors observed (Deaf / Absent / Refused) |
| rpiboot `boot_device`, open | `await_device` → `RpiBootError::Usb` | `open_expected` → `RpiBootError::Stop` |
| rpiboot `boot_device`, rounds | `RpiBootError::Usb` | `RpiBootError::Stop(observe_stop)` |
| rpiboot `RomRefused(code)` | `driver_error` → Other | `Flash(Refused(Device, Code(code)))` |
| rpi driver `driver_error` | Cancelled, Incompatible (access), Unreachable, Other | `Stop` → `Flash(stop)`; the rest as before |
| `UsbError` | no identity case | + `WrongIdentity { location, expected, found }` |

## Multi-block xz readers (Gaia's threaded encoder)

Answer to the coordinator's Part 1 questions, from the code.

Readers (all through `atlas-xzdec`, `xz2` 0.1 with bundled liblzma 5.2.5, linked statically):

1. `crates/flash/xzdec/src/lib.rs`: `scan` (:391), `decode_image` (:453), `read_record` (:489),
   `decode_stream` (:475), `memlimit` (:469). Decoder created with
   `Stream::new_stream_decoder(memlimit(dict), 0)` (:480): flags 0, so no concatenated
   decoding. Memory limit = max(dictionary, 8 MiB) + 256 KiB.
2. `crates/flash/disk/src/image.rs:84-89` (`scan`, `decode_image`) and `:107`
   (`decoded_at_least`, for progress). Used by `write_image`
   (`crates/flash/disk/src/writer.rs:429`), reached from `atlas-disk` in `crates/tools/helper`
   and `crates/drivers/disk`.
3. `crates/flash/image/src/lib.rs:227-239` (`open`, the slot reader), reached from
   `crates/core/bundle/src/from_image.rs:103` and `crates/drivers/board-slots/src/install.rs:1007`.
4. `crates/flash/image/src/flash.rs:340-347` (`read_record`, `file_layout`) and `:617-628`
   (`flash_manifest`, the probe).
5. `crates/core/release`: no decode. It hashes and sizes the file's bytes
   (`catalog/download.rs`, `HASH_READ`, Sha256). Works unchanged for any block layout.

Does anything assume one block?

- `MAX_BLOCKS = 8192` (`xzdec/src/lib.rs:48`, checked at :234). **This is a real bound.** A
  stream of more blocks is refused by name ("declares N blocks"). For a fixed block size B
  the largest image accepted is 8192 * B. A block size below image size / 8192 breaks.
  Choose B so that the count stays at or below 8192 for the largest image.
- `record_candidate` (`scan`, :405): a stream is the signature record only if it is the last
  stream, has exactly one block and at most 1024 decoded bytes. The image stream is never
  tested for block count. Works for a multi-block image followed by a record.
- Dictionary: `check_stream` checks each block header and takes the max over blocks, with
  the per-block 8 MiB ceiling. No single-block assumption.
- Uncompressed size: summed from the index over blocks.
- Decoder: one liblzma stream decoder decodes all blocks in sequence; memory is one
  dictionary. No single-block configuration anywhere.
- `MAX_INDEX` = 1 MiB: about 8 bytes per block, so 8192 blocks need about 64 KiB. Fine.

Boundary and padding (unchanged by this PR): the image stream is the one that ends at
`last.start` minus up to 4096 bytes of zero padding (`image_end_before`, :375), and must
start at byte 0. `decode_image` limits input to the image span (:459), so the record is
never read into the image. Bytes after the last stream are refused.

Tests that existed (`crates/flash/image/tests/flash.rs`):

- `a_multi_block_stream_is_read_through_its_index` (:322): `-T1 --block-size=65536` on a
  1.5 MiB image (about 24 blocks). Asserts the manifest's sha and size, through
  `flash_manifest` only.
- `a_dictionary_over_the_limit_is_refused_before_anything_is_decoded` (:336): a single
  block at `-7` (16 MiB dictionary), refused with "dictionary" and "8 MiB".
- `six` (:369): `-6`, accepted.
- `crates/flash/disk/tests/xz_stream.rs` `big-single-block` (:87): one block, bounded RSS
  while writing, 96 MiB bound. It asserts no block count.
- `crates/flash/image/tests/memory.rs:53`: one block (`-T1`), bounded RSS.

Gaps found, and tests added in this PR:

- `crates/flash/image/tests/blocks.rs` (new):
  - `a_multi_block_stream_reads_through_the_manifest_the_probe_and_the_slot_reader`: block
    count checked first with `xz --robot --list`, then manifest, probe and slot read.
  - `a_threaded_multi_block_stream_with_a_signature_record_reads_its_image_and_its_record`:
    `-T4`, with a 108-byte record stream after it.
  - `a_stream_with_more_blocks_than_the_reader_takes_is_refused_by_count_before_decoding`:
    8193 blocks of 4 KiB, refused with "8193 blocks".
- `crates/flash/disk/tests/multi_block.rs` (new): `write_image` on a multi-block image
  (`-T4 --block-size=262144`), with and without a record: the card holds the image and the
  record never reaches storage.

Result: multi-block works unchanged in every reader, with the 8192-block bound as the only
limit. Gaia must keep `block size >= image size / 8192`.

## Sim scripts (`crates/flash/usb/src/sim.rs`)

- Present but silent: `Answer::Timeout` on a device that stays attached (existing), tested
  by `a_device_present_but_silent_is_deaf`.
- Wrong identity: `attach` with other ids at the port (existing re-enumeration); tested by
  `a_wrong_identity_at_the_port_is_refused_and_nothing_is_claimed`.
- Vanishes after N transfers: NEW `SimBus::vanish_after(location, n)`; tested by
  `a_device_that_vanishes_mid_exchange_is_absent_and_the_write_may_have_applied`.
- Reappears on another port: NEW `SimBus::move_device(from, to)` (keeps the interfaces);
  tested by `a_device_that_reappears_on_another_port_is_absent_here_and_found_there`.
- Re-enumeration on the same port with a new identity (existing `attach`): tested by
  `a_re_enumeration_on_the_same_port_follows_the_port_and_the_old_handle_dies` and
  `a_re_enumeration_is_waited_for_within_the_given_wait`.
- Open refused a number of times: NEW `SimBus::fail_opens(location, n, error)`; tested by
  `a_refused_access_is_refused_by_this_host_with_its_fix` and
  `an_expected_device_that_cannot_be_opened_in_time_is_timed_out`.

## Tests added (every outcome case has a producer and a test)

- `crates/flash/usb/src/port.rs`: parse, Display, malformed paths, the constructor.
- `crates/flash/usb/src/outcome.rs`, `tests/ports.rs`: one test per case:
  - Done: two identical boards on 1-2 and 1-3, only 1-3 claimed.
  - Refused (Host, WrongIdentity), nothing claimed, nothing sent.
  - Absent, nothing on the port, and absent after vanishing.
  - Deaf, present but silent.
  - Refused (Device, Message), stall.
  - Refused (Host, Message), access with its fix.
  - TimedOut, device there but not openable.
  - Refused (Device, Code): `crates/drivers/rpi/src/lib.rs` `a_rom_refusal_is_a_device_refusal_with_its_code`.
- Driver tests, each refusing before any transfer where the brief requires:
  - `crates/drivers/allwinner/tests/driver.rs`: a drop during the write is Absent with
    `may_have_applied`; another identity on the port is refused and never claimed; a port
    with no valid path is refused.
  - `crates/drivers/k230/tests/driver.rs`: another identity refused; a silent stage query is
    Deaf; nothing on the port after the wait is Absent (was Unreachable).
  - `crates/flash/rockchip/tests/flow.rs`: wrong identity refused before any stage; stall
    is a device refusal; a bad CRC is a device refusal; a drop mid-download is Absent.
  - `crates/flash/rpiboot/tests/boot.rs`: an empty port is Absent; another identity gets no
    second stage.
- Job engine, `crates/core/jobs/tests/jobs.rs`: a write that may have applied is Unknown and
  sent once; a settled identity refusal fails the job with its words; a deaf device before
  any write is retried.
- Part 1: see the multi-block section.

## Not proven on hardware (UNVERIFIED)

Everything in this PR. In particular:

- nusb `open_at` identity read from the open device (`device_descriptor()`) before claim:
  compiled, never run against a device. The K230D at USB 29f1:0230 was not opened, and no
  test enumerates or opens real devices.
- The wait and backoff values (`open_expected`, `wait`) for a real re-enumeration.
- Whether a real boot ROM's stall and timeout behave as the sim scripts them.
- The Deaf and Absent split after a real timeout: depends on the bus listing right after a
  timeout, which the sim gives but a real bus may lag.

## Open questions for the coordinator

1. `DriverError` now depends on `atlas-usb`. Accept, or should the carried type move to a
   shared crate?
2. `may_have_applied` drives Unknown in the job engine. Confirm the mapping in
   `crates/core/jobs/src/call.rs`.
3. Fastboot's open and the read-only `identify` paths are not identity-checked. Include them
   in FL-A, or here?

## Gaia's 64 MiB-block sample (addendum)

Gaia's encoder writes one stream of fixed 64 MiB blocks (`xz -6 --check=crc64
--block-size=64MiB -T<N>`, LZMA2, 8 MiB dictionary, sizes in headers). Its sample is the
fixture `crates/flash/xzdec/tests/fixtures/gaia-64mib-3blocks.img.xz` (131,020 bytes,
sha256 87de9f1d…1deff0, checked by the test). It decodes to 167,772,160 bytes, sha256
326c6782…a16f.

- `crates/flash/xzdec/tests/gaia_sample.rs`:
  - `scan` reports one image stream, 3 blocks, 8 MiB dictionary, no record. `Layout` gained a
    `blocks` field for this.
  - `decode_image` into a hashing sink (never buffering the image) yields exactly the stated
    length and digest.
  - `gaia_block_count_arithmetic_matches_the_block_limit`: blocks = ceil(size / 64 MiB),
    and `MAX_BLOCKS` (now `pub`, 8192) is reached at 512 GiB. A change on either side fails it.
  - `a_signed_gaia_sample_has_its_image_then_its_record`: `#[ignore]`, reason "awaiting
    Gaia's signed sample (image + signature stream)".
- `crates/flash/xzdec/src/lib.rs` unit test: the decoder memory limit is the dictionary
  (at least 8 MiB) plus 256 KiB.
- `crates/flash/disk/tests/multi_block.rs`: the card writer onto a sparse 160 MiB target:
  length and digest match.
- Slot reader: not run on the sample. The sample is not known to be a disk image with our
  slot layout, so `open` was not attempted.
- The decode test takes about 5 s in a debug build.
