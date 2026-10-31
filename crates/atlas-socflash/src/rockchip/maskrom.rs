//! Maskrom download (rkdeveloptool `db`, RKComm.cpp `RKU_DeviceRequest`,
//! RKDevice.cpp `DownloadBoot`): each 471 entry, then each 472 entry, sent
//! as vendor control requests of at most 4096 bytes with a big-endian
//! CRC-16 after the payload. The entries go as the loader file holds them
//! (boot_merger already scrambled them when the loader uses RC4).

use std::time::Duration;

use crate::rockchip::crc::crc16_ccitt;
use crate::rockchip::loader::{EntryKind, Loader};
use crate::transport::{UsbError, UsbIo};

/// `bRequest` of every maskrom download request.
pub const REQUEST: u8 = 0x0c;
pub const STAGE_DDR: u16 = 0x0471;
pub const STAGE_USBPLUG: u16 = 0x0472;
const CHUNK: usize = 4096;
/// The pad byte rkdeveloptool appends (`ucFillByte`); its value isn't
/// confirmed from source, 0 assumed.
const FILL: u8 = 0;

/// The bytes of one stage on the wire: the payload, a pad byte when it's
/// one short of a 4096 boundary, then the CRC-16, big-endian; and whether
/// a lone pad byte must follow (when the total is a whole number of
/// 4096-byte chunks, which the boot ROM can't tell from the last one).
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

/// Sends one stage (`STAGE_DDR` or `STAGE_USBPLUG`).
pub async fn send_stage(io: &mut dyn UsbIo, stage: u16, payload: &[u8]) -> Result<(), UsbError> {
    let (wire, trailing_fill) = stage_bytes(payload);
    for chunk in wire.chunks(CHUNK) {
        io.control_out(REQUEST, 0, stage, chunk).await?;
    }
    if trailing_fill {
        io.control_out(REQUEST, 0, stage, &[FILL]).await?;
    }
    Ok(())
}

/// Runs a loader's 471 and 472 entries, waiting each entry's delay, then a
/// second, as rkdeveloptool does; the board then re-enumerates in loader
/// mode (its usbplug).
pub async fn download_boot(
    io: &mut dyn UsbIo,
    loader: &Loader,
    file: &[u8],
) -> Result<(), UsbError> {
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
    use crate::rockchip::crc::LOADER_POLYNOMIALS;
    use crate::rockchip::loader::build_for_test;
    use crate::transport::{Script, Step};

    fn out(index: u16, data: &[u8]) -> Step {
        Step::ControlOut {
            request: REQUEST,
            value: 0,
            index,
            data: data.to_vec(),
        }
    }

    #[test]
    fn the_crc_and_the_4096_boundary_rules() {
        let (wire, fill) = stage_bytes(b"123456789");
        assert_eq!(&wire[9..], &[0x29, 0xb1]);
        assert!(!fill);
        let (wire, fill) = stage_bytes(&[0u8; 4095]);
        assert_eq!((wire.len(), wire[4095], fill), (4098, FILL, false));
        assert_eq!(&wire[4096..], &crc16_ccitt(&wire[..4096]).to_be_bytes());
        let (wire, fill) = stage_bytes(&[0u8; 4094]);
        assert_eq!((wire.len(), fill), (4096, true));
    }

    #[tokio::test(start_paused = true)]
    async fn both_stages_in_order_in_4096_byte_requests() {
        let ddr = vec![0x11u8; 5000];
        let usbplug = vec![0x22u8; 4094];
        let file = build_for_test(
            &[
                (EntryKind::Ddr, "ddr", &ddr, 1),
                (EntryKind::Usbplug, "usbplug", &usbplug, 0),
            ],
            LOADER_POLYNOMIALS[0],
        );
        let loader = Loader::parse(&file).unwrap();
        let (ddr_wire, _) = stage_bytes(&ddr);
        let (plug_wire, _) = stage_bytes(&usbplug);
        let mut script = Script::new(vec![
            out(STAGE_DDR, &ddr_wire[..4096]),
            out(STAGE_DDR, &ddr_wire[4096..]),
            out(STAGE_USBPLUG, &plug_wire),
            out(STAGE_USBPLUG, &[FILL]),
        ]);
        download_boot(&mut script, &loader, &file).await.unwrap();
        assert!(script.finished());
    }

    #[tokio::test(start_paused = true)]
    async fn a_board_that_drops_off_says_so() {
        let file = build_for_test(
            &[
                (EntryKind::Ddr, "ddr", &[1; 10], 0),
                (EntryKind::Usbplug, "usbplug", &[2; 10], 0),
            ],
            LOADER_POLYNOMIALS[0],
        );
        let loader = Loader::parse(&file).unwrap();
        let mut script = Script::new(vec![Step::Disconnect]);
        assert_eq!(
            download_boot(&mut script, &loader, &file).await,
            Err(UsbError::Disconnected)
        );
    }
}
