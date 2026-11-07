//! Rockusb vectors derived by hand from rkdeveloptool 304f073 (the source the
//! blob fetch recorded in /mnt/nvme-wd/atlas-blobs/radxa-rock-5c/
//! rkdeveloptool-304f073/). Each CBW is built from the C layout: the struct
//! `CBW` (RKComm.h:60-77), `InitializeCBW` (RKComm.cpp, flags and CDB length
//! per opcode), and the transfer length each `RKU_*` sets in its own body.
//! A reply's shape is from the same functions.

use atlas_socflash::rockchip::rockusb::{Cb, Reset, Rockusb, Storage, cbw, op};
use atlas_socflash::transport::{Script, Step};

/// The 31-byte CBW of an opcode, written field by field:
/// `dwCBWSignature` "USBC", `dwCBWTag` (LE), `dwCBWTransferLength` (LE),
/// `ucCBWFlags` (0x80 IN, 0x00 OUT), `ucCBWLUN`, `ucCBWCBLength` (6 or 10),
/// then the CBWCB: opcode, `ucReserved` (the subcode), LBA and count.
fn expected(tag: u32, transfer: u32, input: bool, cdb_len: u8, opcode: u8, sub: u8) -> Vec<u8> {
    let mut want = vec![0u8; 31];
    want[0..4].copy_from_slice(b"USBC");
    want[4..8].copy_from_slice(&tag.to_le_bytes());
    want[8..12].copy_from_slice(&transfer.to_le_bytes());
    want[12] = if input { 0x80 } else { 0x00 };
    want[14] = cdb_len;
    want[15] = opcode;
    want[16] = sub;
    want
}

fn csw(tag: u32, status: u8) -> Vec<u8> {
    let mut csw = b"USBS".to_vec();
    csw.extend_from_slice(&tag.to_le_bytes());
    csw.extend_from_slice(&[0; 4]);
    csw.push(status);
    csw
}

#[test]
fn read_flash_info_asks_for_11_bytes() {
    // RKComm.cpp:391-400: InitializeCBW(READ_FLASH_INFO) (IN, CDB 6), then
    // dwCBWTransferLength = 11.
    let want = expected(1, 11, true, 6, op::READ_FLASH_INFO, 0);
    assert_eq!(
        cbw(1, 11, true, Cb::short(op::READ_FLASH_INFO, 0)).to_vec(),
        want
    );
}

#[test]
fn chip_info_asks_for_16_and_flash_id_for_5() {
    // RKComm.cpp:331 (READ_CHIP_INFO, 16), RKComm.cpp:366 (READ_FLASH_ID, 5).
    assert_eq!(
        cbw(2, 16, true, Cb::short(op::READ_CHIP_INFO, 0)).to_vec(),
        expected(2, 16, true, 6, op::READ_CHIP_INFO, 0)
    );
    assert_eq!(
        cbw(3, 5, true, Cb::short(op::READ_FLASH_ID, 0)).to_vec(),
        expected(3, 5, true, 6, op::READ_FLASH_ID, 0)
    );
}

#[test]
fn read_storage_asks_for_a_4_byte_mask() {
    // RKComm.cpp:613: dwCBWTransferLength = 4, after InitializeCBW(READ_STORAGE).
    assert_eq!(
        cbw(4, 4, true, Cb::short(op::READ_STORAGE, 0)).to_vec(),
        expected(4, 4, true, 6, op::READ_STORAGE, 0)
    );
}

#[test]
fn change_storage_and_reset_carry_their_subcode_in_ucreserved() {
    // RKComm.cpp:559-597 (CHANGE_STORAGE: OUT, CDB 6, storage in ucReserved
    // at 572); RKComm.cpp:520-557 (DEVICE_RESET: OUT, CDB 6, subcode).
    assert_eq!(
        cbw(5, 0, false, Cb::short(op::CHANGE_STORAGE, 2)).to_vec(),
        expected(5, 0, false, 6, op::CHANGE_STORAGE, 2)
    );
    assert_eq!(
        cbw(
            6,
            0,
            false,
            Cb::short(op::DEVICE_RESET, Reset::Maskrom as u8)
        )
        .to_vec(),
        expected(6, 0, false, 6, op::DEVICE_RESET, 3)
    );
}

#[test]
fn flash_info_reply_is_rkdeveloptools_11_byte_struct() {
    // RKDevice.h:34-43 (STRUCT_FLASHINFO_CMD, packed): uiFlashSize,
    // usBlockSize, bPageSize, bECCBits, bAccessTime, bManufCode, bFlashCS.
    // The sizes are sectors (RKDevice.cpp:308-318 halves them to KiB).
    let reply = vec![
        0x00, 0x00, 0x20, 0x00, // uiFlashSize: 2 Mi sectors (1 GiB)
        0x00, 0x01, // usBlockSize: 256 sectors (128 KiB)
        0x04, // bPageSize: 4 sectors (2 KiB)
        0x00, 0x00, 0x01, 0x03, // ECC, access time, manufacturer, CS
    ];
    let mut script = Script::new(vec![
        Step::BulkOut(expected(1, 11, true, 6, op::READ_FLASH_INFO, 0)),
        Step::BulkIn {
            max: 512,
            reply: reply.clone(),
        },
        Step::BulkIn {
            max: 13,
            reply: csw(1, 0),
        },
    ]);
    let mut device = Rockusb::new(&mut script);
    let info = futures_block_on(device.read_flash_info()).unwrap();
    assert_eq!(info.sectors, 0x0020_0000);
    assert_eq!(info.block_sectors, 256);
    assert_eq!(info.page_sectors, 4);
    assert_eq!(info.ecc_bits, 0);
    assert_eq!(info.manufacturer, 1);
    assert_eq!(info.chip_select, 3);
    assert_eq!(info.bytes(), 0x0020_0000 * 512);
    assert!(script.finished());
}

#[test]
fn a_flash_info_reply_shorter_than_its_fields_is_refused() {
    let mut script = Script::new(vec![
        Step::BulkOut(expected(1, 11, true, 6, op::READ_FLASH_INFO, 0)),
        Step::BulkIn {
            max: 512,
            reply: vec![0; 8],
        },
        Step::BulkIn {
            max: 13,
            reply: csw(1, 0),
        },
    ]);
    let mut device = Rockusb::new(&mut script);
    assert!(futures_block_on(device.read_flash_info()).is_err());
}

#[test]
fn storage_mask_bits_are_the_storage_ids() {
    // main.cpp:59 gives 1 eMMC, 2 SD, 9 SPI-NOR; RKComm.cpp:599-647 takes the
    // index of the first set bit of the 4-byte mask.
    for (mask, id) in [(0x2u32, 1u8), (0x4, 2), (0x200, 9)] {
        let mut script = Script::new(vec![
            Step::BulkOut(expected(1, 4, true, 6, op::READ_STORAGE, 0)),
            Step::BulkIn {
                max: 4,
                reply: mask.to_le_bytes().to_vec(),
            },
            Step::BulkIn {
                max: 13,
                reply: csw(1, 0),
            },
        ]);
        let mut device = Rockusb::new(&mut script);
        assert_eq!(futures_block_on(device.read_storage()).unwrap(), Some(id));
    }
}

#[test]
fn a_storage_the_board_does_not_switch_to_is_refused() {
    // rkdeveloptool's cs reads the storage back (main.cpp:2283-2298): the
    // board answers CHANGE_STORAGE with success whether or not it has SPI-NOR.
    let mut script = Script::new(vec![
        Step::BulkOut(expected(1, 0, false, 6, op::CHANGE_STORAGE, 9)),
        Step::BulkIn {
            max: 13,
            reply: csw(1, 0),
        },
        Step::BulkOut(expected(2, 4, true, 6, op::READ_STORAGE, 0)),
        Step::BulkIn {
            max: 4,
            reply: 0x2u32.to_le_bytes().to_vec(),
        },
        Step::BulkIn {
            max: 13,
            reply: csw(2, 0),
        },
    ]);
    let mut device = Rockusb::new(&mut script);
    let error = futures_block_on(device.change_storage(Storage::SpiNor)).unwrap_err();
    assert!(error.to_string().contains("isn't available"), "{error}");
}

/// The tests run the device calls on a small current-thread runtime.
fn futures_block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}
