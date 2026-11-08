//! The two FEL flows: the SPL into SRAM through the thunk, and the legacy
//! U-Boot image into DRAM. Each step is a FEL command from fel/client.rs,
//! so a dry run through [`super::sim`] and a real board take the same path.

use super::client::Fel;
use super::layout::{Layout, LayoutError};
use super::protocol::MODE_FEL;
use super::thunk::{self, TableError};
use crate::transport::{UsbError, UsbIo};

/// Why an FEL flash stopped.
#[derive(Debug)]
pub enum FlashError {
    Usb(UsbError),
    Layout(LayoutError),
    Table(TableError),
    /// The device was not in FEL mode when a step needed it.
    NotFel {
        mode: u16,
    },
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

/// What a flash did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The SoC id the device reported (verify-device's board field).
    pub soc_id: u32,
    /// How many SRAM writes the SPL took (more than one when it crosses a
    /// backup redirect).
    pub spl_pieces: usize,
    pub uboot_bytes: usize,
}

/// Writes the SPL through the thunk, reads it back, runs the thunk, and
/// checks that FEL is answering again. Returns the SPL's piece count.
pub async fn load_spl(
    io: &mut dyn UsbIo,
    layout: &Layout,
    spl: &[u8],
) -> Result<usize, FlashError> {
    let mut fel = Fel::new(io);
    let verify = fel.verify().await?;
    if verify.mode != MODE_FEL {
        return Err(FlashError::NotFel { mode: verify.mode });
    }
    let pieces = layout.spl_pieces(spl.len())?;
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
    let after = fel.verify().await.map_err(|error| {
        FlashError::Usb(UsbError::Protocol(format!(
            "the thunk did not return to FEL: {error}"
        )))
    })?;
    if after.mode != MODE_FEL {
        return Err(FlashError::NotFel { mode: after.mode });
    }
    Ok(pieces.len())
}

/// Writes the legacy U-Boot image to DRAM at the layout's address, reads
/// it back, and runs it. The device leaves FEL when U-Boot starts, so a
/// disconnect after the run is the expected end.
pub async fn load_uboot(
    io: &mut dyn UsbIo,
    layout: &Layout,
    image: &[u8],
) -> Result<(), FlashError> {
    let mut fel = Fel::new(io);
    fel.write(layout.uboot_address, image).await?;
    let back = fel.read(layout.uboot_address, image.len()).await?;
    if back != image {
        return Err(FlashError::Readback(format!(
            "the U-Boot image at {:#010x}, {} bytes",
            layout.uboot_address,
            image.len()
        )));
    }
    match fel.run(layout.uboot_address).await {
        Ok(()) | Err(UsbError::Disconnected) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Both flows, in order: the SPL, then the U-Boot image.
pub async fn flash(
    io: &mut dyn UsbIo,
    layout: &Layout,
    spl: &[u8],
    uboot: &[u8],
) -> Result<Report, FlashError> {
    let pieces = load_spl(io, layout, spl).await?;
    let soc_id = Fel::new(io).verify().await?.board;
    load_uboot(io, layout, uboot).await?;
    Ok(Report {
        soc_id,
        spl_pieces: pieces,
        uboot_bytes: uboot.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fel::layout::Swap;
    use crate::fel::sim::{Board, STACK_FILL};

    /// A made-up layout (the A733's is not sourced): the SPL loads on the
    /// 0x800-byte stack buffer at 0x1800, backed up at 0x44000.
    fn layout() -> Layout {
        Layout {
            load_address: 0x1800,
            thunk_address: 0x20000,
            swaps: vec![Swap {
                buf1: 0x1800,
                buf2: 0x44000,
                size: 0x800,
            }],
            uboot_address: 0x4100_0000,
        }
    }

    #[tokio::test]
    async fn the_spl_runs_from_its_load_address_and_the_stack_comes_back() {
        let spl: Vec<u8> = (0..0x800u32).map(|n| (n % 251) as u8 | 1).collect();
        let layout = layout();
        let mut board = Board::new(layout.clone(), spl.len(), 0x1_0000);
        let pieces = load_spl(&mut board, &layout, &spl).await.unwrap();
        assert_eq!(pieces, 1);
        // The SPL saw its bytes at its load address during the call ...
        assert_eq!(board.spl_seen, spl);
        // ... the live stack is back in place after the thunk ...
        assert!(
            board
                .sram(0x1800, 0x800)
                .iter()
                .all(|byte| *byte == STACK_FILL)
        );
        // ... and the SPL's bytes sit in the backup, where the swap left them.
        assert_eq!(board.sram(0x44000, 0x800), &spl[..]);
    }

    #[tokio::test]
    async fn the_whole_flash_writes_the_spl_then_the_uboot_image() {
        let spl = vec![0xabu8; 0x400];
        let uboot: Vec<u8> = (0..0x1800u32).map(|n| (n % 253) as u8).collect();
        let layout = layout();
        let mut board = Board::new(layout.clone(), spl.len(), uboot.len());
        let report = flash(&mut board, &layout, &spl, &uboot).await.unwrap();
        assert_eq!(report.spl_pieces, 1);
        assert_eq!(report.uboot_bytes, uboot.len());
        assert_eq!(report.soc_id, crate::fel::sim::MOCK_SOC_ID);
        assert_eq!(board.spl_seen[..spl.len()], spl[..]);
        assert_eq!(board.commands.last(), Some(&crate::fel::protocol::cmd::RUN));
    }

    #[tokio::test]
    async fn an_spl_on_the_thunk_is_refused_before_any_write() {
        let mut layout = layout();
        layout.load_address = 0x1f000;
        let mut board = Board::new(layout.clone(), 0x2000, 0x1000);
        let error = load_spl(&mut board, &layout, &[0u8; 0x2000])
            .await
            .unwrap_err();
        assert!(matches!(error, FlashError::Layout(LayoutError::Overlap(_))));
        assert!(board.spl_seen.is_empty());
    }
}
