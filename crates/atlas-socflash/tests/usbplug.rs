//! Loader mode end to end against a simulated usbplug: a device that reads
//! rockusb CBWs and serves RAM storage. A disk image goes on and reads
//! back, and a UBI image goes onto "SPI-NAND" through LBAs with atlas-ubi's
//! ubiformat-style writer driven through the runtime handle, as Atlas's
//! driver does it.

use std::collections::VecDeque;

use async_trait::async_trait;
use atlas_socflash::rockchip::rockusb::Rockusb;
use atlas_socflash::rockchip::write::{LbaNand, write_image};
use atlas_socflash::{UsbError, UsbIo};
use atlas_ubi::headers::{
    EcHeader, LAYOUT_VOLUME_ID, Read as Header, VidHeader, VolumeKind, VtblRecord,
};

const SECTOR: usize = 512;

/// A usbplug over `storage`: WRITE_LBA, READ_LBA, ERASE_LBA (to 0xff, as
/// NAND erases), TEST_UNIT_READY.
struct Usbplug {
    storage: Vec<u8>,
    /// A command waiting for its OUT data.
    pending_write: Option<(u32, usize, u32)>,
    replies: VecDeque<Vec<u8>>,
    commands: usize,
}

impl Usbplug {
    fn new(bytes: usize) -> Self {
        Self {
            storage: vec![0xff; bytes],
            pending_write: None,
            replies: VecDeque::new(),
            commands: 0,
        }
    }

    fn csw(&mut self, tag: u32, status: u8) {
        let mut csw = b"USBS".to_vec();
        csw.extend_from_slice(&tag.to_le_bytes());
        csw.extend_from_slice(&[0, 0, 0, 0, status]);
        self.replies.push_back(csw);
    }
}

#[async_trait]
impl UsbIo for Usbplug {
    async fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
        Err(UsbError::Protocol(
            "a usbplug takes no control requests".into(),
        ))
    }

    async fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, UsbError> {
        Err(UsbError::Protocol(
            "a usbplug takes no control requests".into(),
        ))
    }

    async fn bulk_out(&mut self, data: &[u8]) -> Result<(), UsbError> {
        if let Some((lba, length, tag)) = self.pending_write.take() {
            assert_eq!(data.len(), length, "the OUT data matches the CBW");
            let at = lba as usize * SECTOR;
            self.storage[at..at + length].copy_from_slice(data);
            self.csw(tag, 0);
            return Ok(());
        }
        assert_eq!(data.len(), 31, "a CBW");
        assert_eq!(&data[..4], b"USBC");
        self.commands += 1;
        let tag = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let transfer = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        let cb = &data[15..];
        let lba = u32::from_be_bytes(cb[2..6].try_into().unwrap());
        let count = u16::from_be_bytes([cb[7], cb[8]]) as usize;
        match cb[0] {
            0x00 => self.csw(tag, 0),
            0x15 => {
                assert_eq!(transfer, count * SECTOR);
                self.pending_write = Some((lba, transfer, tag));
            }
            0x14 => {
                let at = lba as usize * SECTOR;
                self.replies
                    .push_back(self.storage[at..at + count * SECTOR].to_vec());
                self.csw(tag, 0);
            }
            0x25 => {
                let at = lba as usize * SECTOR;
                self.storage[at..at + count * SECTOR].fill(0xff);
                self.csw(tag, 0);
            }
            other => panic!("opcode {other:#04x}"),
        }
        Ok(())
    }

    async fn bulk_in(&mut self, max: usize) -> Result<Vec<u8>, UsbError> {
        let reply = self.replies.pop_front().expect("a reply is due");
        assert!(reply.len() <= max);
        Ok(reply)
    }
}

const PEB: usize = 16 * 1024;
const PAGE: usize = 512;

/// A UBI image as Gaia writes one: layout volume, then rootfs_a's data.
fn ubi_image(rootfs: &[u8]) -> Vec<u8> {
    let records = ((PEB - 1024) / 172).min(128);
    let mut table = Vec::new();
    for id in 0..records {
        let record = match id {
            0 => Some(("rootfs_a", 4)),
            1 => Some(("rootfs_b", 4)),
            _ => None,
        }
        .map(|(name, reserved)| VtblRecord {
            reserved_pebs: reserved,
            alignment: 1,
            data_pad: 0,
            kind: VolumeKind::Dynamic,
            upd_marker: false,
            name: name.into(),
            autoresize: false,
        });
        table.extend_from_slice(&VtblRecord::encode(record.as_ref()));
    }
    let block = |volume_id: u32, lnum: u32, compat: u8, data: &[u8]| {
        let mut block = vec![0xffu8; PEB];
        block[..64].copy_from_slice(
            &EcHeader {
                erase_counter: 0,
                vid_header_offset: 512,
                data_offset: 1024,
                image_seq: 7,
            }
            .encode(),
        );
        block[512..576].copy_from_slice(
            &VidHeader {
                kind: VolumeKind::Dynamic,
                copy_flag: 0,
                compat,
                volume_id,
                lnum,
                data_size: 0,
                used_ebs: 0,
                data_pad: 0,
                data_crc: 0,
                sqnum: 0,
            }
            .encode(),
        );
        block[1024..1024 + data.len()].copy_from_slice(data);
        block
    };
    let mut image = block(LAYOUT_VOLUME_ID, 0, 5, &table);
    image.extend(block(LAYOUT_VOLUME_ID, 1, 5, &table));
    for (lnum, chunk) in rootfs.chunks(PEB - 1024).enumerate() {
        image.extend(block(0, lnum as u32, 0, chunk));
    }
    image
}

#[test]
fn a_disk_image_and_a_ubi_region_through_the_usbplug() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let mut plug = Usbplug::new(64 * PEB);

    // The loaders before the UBI region: 2 eraseblocks, written as is.
    let head: Vec<u8> = (0..2 * PEB).map(|i| (i % 249) as u8).collect();
    let written = {
        let mut device = Rockusb::new(&mut plug);
        handle
            .block_on(write_image(
                &mut device,
                &mut &head[..],
                head.len() as u64,
                0,
                true,
                &mut |_, _, _| {},
                &|| false,
            ))
            .unwrap()
    };
    assert_eq!(written.bytes, head.len() as u64);
    assert_eq!(plug.storage[..head.len()], head[..]);

    // The UBI region, from eraseblock 2 to the end of the flash.
    let rootfs: Vec<u8> = (0..20_000u32).map(|i| (i % 241) as u8).collect();
    let image = ubi_image(&rootfs);
    let info = atlas_ubi::read_image(&mut std::io::Cursor::new(&image), PEB as u32).unwrap();
    let report = {
        let mut device = Rockusb::new(&mut plug);
        let mut nand = LbaNand {
            device: &mut device,
            runtime: handle.clone(),
            base_lba: (2 * PEB / SECTOR) as u32,
            pebs: 62,
            peb_size: PEB as u32,
            page_size: PAGE as u32,
        };
        atlas_ubi::format(
            &mut nand,
            &mut std::io::Cursor::new(&image),
            &info,
            true,
            &mut |_, _| {},
        )
        .unwrap()
    };
    assert_eq!(report.placed, [0, 1, 2, 3]);
    assert_eq!(report.empty_pebs, 58);
    assert!(report.marked_bad.is_empty());

    // The flash now holds the image's PEBs (counters as written: 0, as the
    // flash had none) and an EC header on every other eraseblock.
    let flash = &plug.storage[2 * PEB..];
    for (index, block) in flash.chunks(PEB).enumerate() {
        match EcHeader::read(block) {
            Header::Header(ec) => {
                assert_eq!((ec.erase_counter, ec.image_seq), (0, 7), "PEB {index}")
            }
            other => panic!("PEB {index}: {other:?}"),
        }
    }
    let carried = &flash[..4 * PEB];
    assert_eq!(
        atlas_ubi::read_image(&mut std::io::Cursor::new(carried), PEB as u32)
            .unwrap()
            .volumes,
        info.volumes
    );
    // An empty PEB past the image holds one programmed page, then erased.
    assert!(
        flash[5 * PEB + PAGE..6 * PEB]
            .iter()
            .all(|byte| *byte == 0xff)
    );
    assert!(plug.commands > 0);
}
