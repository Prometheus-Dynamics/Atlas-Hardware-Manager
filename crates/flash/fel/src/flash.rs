//! The two FEL flows: the SPL into SRAM through the thunk, then an image
//! into DRAM. Each step is a FEL command from [`crate::client`], so the mock
//! device in [`crate::sim`] and a real board take the same path.

use atlas_usb::{UsbError, UsbIo};

use crate::client::Fel;
use crate::layout::{Layout, LayoutError};
use crate::protocol::MODE_FEL;
use crate::thunk::{self, TableError};

/// Why an FEL flash stopped. Nothing after the failing step is written.
#[derive(Debug)]
pub enum FlashError {
    Usb(UsbError),
    Layout(LayoutError),
    Table(TableError),
    /// The device was not in FEL mode when a step needed it.
    NotFel {
        mode: u16,
    },
    /// The device reports a different SoC than the caller expects. Checked
    /// before any write.
    WrongSoc {
        expected: u32,
        found: u32,
    },
    /// The thunk ran but FEL did not answer afterwards.
    ReturnToFel(String),
    /// A read-back didn't match what was written.
    Readback(String),
}

impl std::fmt::Display for FlashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usb(error) => write!(f, "{error}"),
            Self::Layout(error) => write!(f, "{error}"),
            Self::Table(error) => write!(f, "{error}"),
            Self::NotFel { mode } => {
                write!(f, "the device reports mode {mode}, not FEL ({MODE_FEL})")
            }
            Self::WrongSoc { expected, found } => write!(
                f,
                "the device reports SoC id {found:#010x}, the description expects {expected:#010x}"
            ),
            Self::ReturnToFel(what) => {
                write!(f, "the SPL did not return the device to FEL: {what}")
            }
            Self::Readback(what) => write!(f, "read-back mismatch: {what}"),
        }
    }
}

impl std::error::Error for FlashError {}

impl From<UsbError> for FlashError {
    fn from(error: UsbError) -> Self {
        Self::Usb(error)
    }
}

impl From<LayoutError> for FlashError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<TableError> for FlashError {
    fn from(error: TableError) -> Self {
        Self::Table(error)
    }
}

/// What the SPL stage did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spl {
    /// The SoC id the device reported (verify-device's board field).
    pub soc_id: u32,
    /// How many SRAM writes the SPL took (more than one when it crosses a
    /// backup redirect).
    pub pieces: usize,
}

/// What a whole flash did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub soc_id: u32,
    pub spl_pieces: usize,
    pub image_bytes: usize,
}

/// Writes the SPL through the thunk, reads it back, runs the thunk, and
/// checks that FEL is answering again. `expected_soc`, when given, must match
/// the device's SoC id, or nothing is written.
pub async fn load_spl(
    io: &mut dyn UsbIo,
    layout: &Layout,
    spl: &[u8],
    expected_soc: Option<u32>,
) -> Result<Spl, FlashError> {
    layout.check()?;
    // The SRAM plan is worked out before the device is touched.
    let pieces = layout.spl_pieces(spl.len())?;
    let mut fel = Fel::new(io);
    let verify = fel.verify().await?;
    if verify.mode != MODE_FEL {
        return Err(FlashError::NotFel { mode: verify.mode });
    }
    if let Some(expected) = expected_soc
        && verify.board != expected
    {
        return Err(FlashError::WrongSoc {
            expected,
            found: verify.board,
        });
    }
    let blob = thunk::blob(&layout.swaps, layout.load_address)?;
    fel.write(layout.thunk_address, &blob).await?;
    for piece in &pieces {
        let data = &spl[piece.offset..piece.offset + piece.len];
        fel.write(piece.address, data).await?;
    }
    for piece in &pieces {
        let back = fel.read(piece.address, piece.len).await?;
        if back != spl[piece.offset..piece.offset + piece.len] {
            return Err(FlashError::Readback(format!(
                "the SPL at {:#010x}, {} bytes",
                piece.address, piece.len
            )));
        }
    }
    fel.run(layout.thunk_address).await?;
    let after = fel
        .verify()
        .await
        .map_err(|error| FlashError::ReturnToFel(error.to_string()))?;
    if after.mode != MODE_FEL {
        return Err(FlashError::NotFel { mode: after.mode });
    }
    Ok(Spl {
        soc_id: verify.board,
        pieces: pieces.len(),
    })
}

/// Writes `image` to DRAM at the layout's image address, reads it back, and
/// runs it from the entry address. The device leaves FEL when the image
/// starts, so a disconnect after the run is the expected end. A disconnect
/// during the write is not: it is returned as an error.
pub async fn load_image(
    io: &mut dyn UsbIo,
    layout: &Layout,
    image: &[u8],
) -> Result<(), FlashError> {
    let mut fel = Fel::new(io);
    fel.write(layout.image_address, image).await?;
    let back = fel.read(layout.image_address, image.len()).await?;
    if back != image {
        return Err(FlashError::Readback(format!(
            "the image at {:#010x}, {} bytes",
            layout.image_address,
            image.len()
        )));
    }
    match fel.run(layout.image_entry).await {
        Ok(()) | Err(UsbError::Disconnected) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Both stages in order: the SPL through the thunk, then the image.
pub async fn flash(
    io: &mut dyn UsbIo,
    layout: &Layout,
    spl: &[u8],
    image: &[u8],
    expected_soc: Option<u32>,
) -> Result<Report, FlashError> {
    let stage = load_spl(io, layout, spl, expected_soc).await?;
    load_image(io, layout, image).await?;
    Ok(Report {
        soc_id: stage.soc_id,
        spl_pieces: stage.pieces,
        image_bytes: image.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Swap, ThunkIsa};
    use crate::protocol::cmd;
    use crate::sim::{FelDevice, MOCK_SOC_ID, STACK_FILL};

    /// A made-up layout (the A733's is not sourced): the SPL loads on the
    /// 0x800-byte stack buffer at 0x1800, backed up at 0x44000.
    fn layout() -> Layout {
        Layout {
            thunk_isa: ThunkIsa::Armv7,
            load_address: 0x1800,
            thunk_address: 0x20000,
            swaps: vec![Swap {
                buf1: 0x1800,
                buf2: 0x44000,
                size: 0x800,
            }],
            image_address: 0x4100_0000,
            image_entry: 0x4100_0000,
        }
    }

    #[tokio::test]
    async fn the_spl_runs_from_its_load_address_and_the_stack_comes_back() {
        let spl: Vec<u8> = (0..0x800u32).map(|n| (n % 251) as u8 | 1).collect();
        let layout = layout();
        let mut board = FelDevice::new(layout.clone(), spl.len(), 0x1_0000);
        let stage = load_spl(&mut board, &layout, &spl, None).await.unwrap();
        assert_eq!(stage.pieces, 1);
        // The SPL saw its bytes at its load address during the call ...
        assert_eq!(board.spl_seen, spl);
        // ... the live stack is back in place after the thunk ...
        assert!(board.sram(0x1800, 0x800).iter().all(|b| *b == STACK_FILL));
        // ... and the SPL's bytes sit in the backup, where the swap left them.
        assert_eq!(board.sram(0x44000, 0x800), &spl[..]);
    }

    #[tokio::test]
    async fn the_whole_flash_writes_the_spl_then_the_image() {
        let spl = vec![0xabu8; 0x400];
        let image: Vec<u8> = (0..0x1800u32).map(|n| (n % 253) as u8).collect();
        let layout = layout();
        let mut board = FelDevice::new(layout.clone(), spl.len(), image.len());
        let report = flash(&mut board, &layout, &spl, &image, Some(MOCK_SOC_ID))
            .await
            .unwrap();
        assert_eq!(report.spl_pieces, 1);
        assert_eq!(report.image_bytes, image.len());
        assert_eq!(report.soc_id, MOCK_SOC_ID);
        assert_eq!(board.dram(layout.image_address, image.len()), &image[..]);
        assert_eq!(board.commands.last(), Some(&cmd::RUN));
    }

    #[tokio::test]
    async fn a_device_that_is_not_in_fel_is_refused_before_any_write() {
        let layout = layout();
        let mut board = FelDevice::new(layout.clone(), 0x400, 0x1000).in_mode(2);
        let error = load_spl(&mut board, &layout, &[0u8; 0x400], None)
            .await
            .unwrap_err();
        assert!(matches!(error, FlashError::NotFel { mode: 2 }));
        assert_eq!(board.commands, vec![cmd::VERIFY_DEVICE]);
    }

    #[tokio::test]
    async fn an_unexpected_soc_is_refused_before_any_write() {
        let layout = layout();
        let mut board = FelDevice::new(layout.clone(), 0x400, 0x1000);
        let error = load_spl(&mut board, &layout, &[0u8; 0x400], Some(0x1234))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FlashError::WrongSoc { expected: 0x1234, found } if found == MOCK_SOC_ID
        ));
        assert_eq!(board.commands, vec![cmd::VERIFY_DEVICE]);
    }

    #[tokio::test]
    async fn an_spl_on_the_thunk_is_refused_before_any_write() {
        let mut layout = layout();
        layout.load_address = 0x1f000;
        let mut board = FelDevice::new(layout.clone(), 0x2000, 0x1000);
        let error = load_spl(&mut board, &layout, &[0u8; 0x2000], None)
            .await
            .unwrap_err();
        assert!(matches!(error, FlashError::Layout(LayoutError::Overlap(_))));
        assert!(board.spl_seen.is_empty());
        assert!(board.commands.is_empty());
    }
}
