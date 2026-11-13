//! Maskrom download (rkdeveloptool `db`, RKComm.cpp `RKU_DeviceRequest`,
//! RKDevice.cpp `DownloadBoot`): each 471 entry, then each 472 entry, sent
//! as vendor control requests of at most 4096 bytes. Each stage ends with a
//! big-endian CRC-16 over its payload. The entries go as the loader file
//! holds them: boot_merger has already scrambled them when the loader uses
//! RC4, and no RC4 is applied on this wire.

use std::time::Duration;

use atlas_usb::UsbIo;

use crate::UsbResult;
use crate::crc::crc16_ccitt;
use crate::loader::{EntryKind, Loader};

/// `bRequest` of every maskrom download request.
pub const REQUEST: u8 = 0x0c;
/// `wIndex` of the first stage (DDR init).
pub const STAGE_DDR: u16 = 0x0471;
/// `wIndex` of the second stage (usbplug).
pub const STAGE_USBPLUG: u16 = 0x0472;
/// The largest request, and the size a request is cut to.
pub const CHUNK: usize = 4096;
/// The pad byte rkdeveloptool appends: zero, both in the buffer it zeroes
/// (RKComm.cpp:829, `memset`) and in its lone trailing byte (`ucFillByte`,
/// RKComm.cpp:867-868).
pub const FILL: u8 = 0;

/// The bytes of one stage on the wire: the payload, a pad byte when it's
/// one short of a 4096 boundary, then the CRC-16, big-endian; and whether a
/// lone pad byte must follow (when the payload is 4094 bytes into its last
/// chunk, the boot ROM can't tell that from a short last chunk).
pub fn stage_bytes(payload: &[u8]) -> (Vec<u8>, bool) {
    let mut wire = payload.to_vec();
    let mut trailing_fill = false;
    match payload.len() % CHUNK {
        4095 => wire.push(FILL),
        4094 => trailing_fill = true,
        _ => {}
    }
    let crc = crc16_ccitt(&wire);
    wire.extend_from_slice(&crc.to_be_bytes());
    (wire, trailing_fill)
}

/// Whether the bytes of one whole stage, as received, carry a valid framing:
/// the CRC-16 of everything before the final two bytes, or, for a stage that
/// ends in a lone pad byte, the CRC-16 before that pad. This is the check the
/// simulated boot ROM applies; it is dev's framing, not a measured ROM.
pub fn stage_is_intact(wire: &[u8]) -> bool {
    let n = wire.len();
    if n >= 2 && crc16_ccitt(&wire[..n - 2]).to_be_bytes() == wire[n - 2..] {
        return true;
    }
    n >= 3 && wire[n - 1] == FILL && crc16_ccitt(&wire[..n - 3]).to_be_bytes() == wire[n - 3..n - 1]
}

/// Sends one stage (`STAGE_DDR` or `STAGE_USBPLUG`) on `io`.
pub async fn send_stage(io: &mut dyn UsbIo, stage: u16, payload: &[u8]) -> UsbResult<()> {
    let (wire, trailing_fill) = stage_bytes(payload);
    for chunk in wire.chunks(CHUNK) {
        io.control_out(REQUEST, 0, stage, chunk).await?;
    }
    if trailing_fill {
        io.control_out(REQUEST, 0, stage, &[FILL]).await?;
    }
    Ok(())
}

/// Runs a loader's 471 and 472 entries, waiting each entry's delay and then
/// a second, as rkdeveloptool does. The board then resets and re-enumerates
/// in loader mode (its usbplug); the caller waits for it.
pub async fn download_boot(io: &mut dyn UsbIo, loader: &Loader, file: &[u8]) -> UsbResult<()> {
    for (kind, stage) in [
        (EntryKind::Ddr, STAGE_DDR),
        (EntryKind::Usbplug, STAGE_USBPLUG),
    ] {
        for entry in loader.entries(kind) {
            send_stage(io, stage, entry.data(file)).await?;
            if entry.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(u64::from(entry.delay_ms))).await;
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::LOADER_POLYNOMIALS;
    use crate::loader::build_loader;

    #[test]
    fn the_crc_and_the_4096_boundary_rules() {
        let (wire, fill) = stage_bytes(b"123456789");
        assert_eq!(&wire[9..], &[0x29, 0xb1]);
        assert!(!fill);
        let (wire, fill) = stage_bytes(&[0u8; 4095]);
        assert_eq!((wire.len(), wire[4095], fill), (4098, FILL, false));
        let (wire, fill) = stage_bytes(&[0u8; 4094]);
        assert_eq!((wire.len(), fill), (4096, true));
    }

    #[test]
    fn every_boundary_length_is_intact_as_sent() {
        for len in [
            0, 1, 2, 4093, 4094, 4095, 4096, 4097, 8190, 8191, 8192, 5000,
        ] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7 + 1) as u8).collect();
            let (mut wire, fill) = stage_bytes(&payload);
            if fill {
                wire.push(FILL);
            }
            assert!(stage_is_intact(&wire), "length {len}");
            if let Some(first) = wire.first_mut() {
                *first ^= 0x40;
                assert!(!stage_is_intact(&wire), "corrupt length {len}");
            }
        }
    }

    #[test]
    fn the_check_value_is_intact_on_its_own() {
        // The payload "123456789" with its CRC-16 is intact on its own.
        assert!(stage_is_intact(&[
            b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', 0x29, 0xb1
        ]));
    }

    #[test]
    fn the_usbplug_stage_is_checked_as_its_own_payload() {
        // Two stages back to back do not frame as one: the CRC is per stage.
        let (a, _) = stage_bytes(b"abc");
        let (b, _) = stage_bytes(b"def");
        let both = [a.as_slice(), b.as_slice()].concat();
        assert!(!stage_is_intact(&both));
    }

    #[test]
    fn loader_entries_are_sent_as_stored() {
        // Bytes the loader stores are the bytes sent: nothing is re-scrambled.
        let file = build_loader(
            b"8853",
            &[
                (EntryKind::Ddr, "ddr", &[9; 3], 0),
                (EntryKind::Usbplug, "plug", &[7; 3], 0),
            ],
            LOADER_POLYNOMIALS[0],
        );
        let loader = Loader::parse(&file).unwrap();
        let ddr = loader.entries(EntryKind::Ddr).next().unwrap();
        assert_eq!(ddr.data(&file), &[9, 9, 9]);
    }
}
