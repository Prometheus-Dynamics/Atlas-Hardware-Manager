//! The SoC's SRAM and DRAM layout, as data: where the SPL loads, where the
//! thunk goes, which SRAM buffers the boot ROM's FEL stacks occupy and which
//! free buffers back them up while the SPL runs, the thunk's instruction set,
//! and where the image goes in DRAM. The caller reads it from the family or
//! board description (JSON, the shape of [`Layout::from_value`]). Null means
//! the SoC's layout is not sourced, and nothing runs.
//!
//! The planning rules come from the linux-sunxi FEL/USBBoot page's
//! description of the thunk: the SPL's bytes that would land on a live stack
//! buffer are written to its backup buffer instead, and the thunk swaps the
//! two before the SPL runs.

use serde_json::Value;

use crate::thunk::{TABLE_ENTRIES, shape};

/// The thunk a layout uses, which is also the FEL execution state it
/// assumes. Three modes; each has its own thunk source (see [`crate::thunk`]).
///
/// - [`ThunkIsa::Armv7`]: FEL runs AArch32, the SPL is AArch32. The thunk
///   calls the SPL and returns into the boot ROM's FEL loop.
/// - [`ThunkIsa::Aarch64`]: FEL runs native AArch64. The same contract with
///   AArch64 registers: the thunk calls the SPL and returns to FEL.
/// - [`ThunkIsa::Aarch32ToAarch64`]: FEL runs AArch32 and the SPL is entered
///   in AArch64 through a warm reset. The thunk writes the SPL's entry to
///   the SoC's reset vector register (`reset_vector`, the RVBAR backing
///   store on Allwinner cores) and requests the warm reset with RMR. The
///   core resets into the SPL, so there is no return to FEL: the flow is
///   [`ReturnMode::NoReturnExpected`].
///
/// Which mode the A733 uses is UNCONFIRMED. The records name the choice and
/// the flow refuses an unconfirmed record unless the caller acknowledges it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThunkIsa {
    Armv7,
    Aarch64,
    Aarch32ToAarch64 {
        /// The SoC's writable reset vector register, written with the SPL's
        /// entry address before the RMR request (a physical address).
        reset_vector: u32,
    },
}

/// Whether the SPL's run ends back in the boot ROM's FEL loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnMode {
    /// The thunk returns into FEL; FEL can be used again (an image can follow).
    ReturnsToFel,
    /// The core warm-resets into the SPL. FEL is gone and no return is
    /// expected, so nothing can follow over FEL.
    NoReturnExpected,
}

impl ThunkIsa {
    /// The return mode this thunk gives.
    pub fn return_mode(self) -> ReturnMode {
        match self {
            Self::Armv7 | Self::Aarch64 => ReturnMode::ReturnsToFel,
            Self::Aarch32ToAarch64 { .. } => ReturnMode::NoReturnExpected,
        }
    }

    /// The thunk blob's length in bytes.
    pub fn blob_len(self) -> usize {
        shape(self).len()
    }

    /// The name a layout record gives this mode.
    pub fn name(self) -> &'static str {
        match self {
            Self::Armv7 => "armv7",
            Self::Aarch64 => "aarch64",
            Self::Aarch32ToAarch64 { .. } => "aarch32-rmr",
        }
    }
}

/// One swap: `size` bytes at `buf1` (a live FEL stack region) exchange with
/// `size` bytes at `buf2` (a free backup area).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swap {
    pub buf1: u32,
    pub buf2: u32,
    pub size: u32,
}

/// The fields of a [`Layout`], as a caller gives them to [`Layout::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutParts {
    pub thunk_isa: ThunkIsa,
    /// Where the SPL's first byte is loaded and entered (SRAM).
    pub load_address: u32,
    /// Where the thunk blob is written and entered (SRAM, outside every
    /// swap buffer and the SPL's own range).
    pub thunk_address: u32,
    pub swaps: Vec<Swap>,
    /// Where the image is written to DRAM (after the SPL has initialised it).
    pub image_address: u32,
    /// Where the image is entered. Usually its address; a format with a
    /// header may enter elsewhere.
    pub image_entry: u32,
}

/// Whether the caller accepts an UNCONFIRMED layout for use on a device.
/// Given when the layout is made; the device-touching entry points check it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acknowledgement {
    /// Not accepted: the layout may be read with, but no device write or run
    /// is made with it.
    Withheld,
    /// Accepted on purpose: a guessed layout may be used on a device.
    Given,
}

/// Whether a layout is proven on hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirmation {
    Confirmed,
    Unconfirmed(Acknowledgement),
}

/// The SRAM and DRAM facts an FEL flash needs, with their confirmation.
///
/// A `Layout` is made only by the checked constructors ([`Layout::new`],
/// [`Layout::from_value`], [`Layout::from_record`]), which run the layout's
/// own rules. Its fields are crate-private, so a struct literal does not
/// build outside this crate:
///
/// ```compile_fail
/// let _ = atlas_fel::layout::Layout {
///     thunk_isa: atlas_fel::layout::ThunkIsa::Armv7,
///     load_address: 0x2000,
///     thunk_address: 0x20000,
///     swaps: Vec::new(),
///     image_address: 0x4100_0000,
///     image_entry: 0x4100_0000,
///     confirmation: atlas_fel::layout::Confirmation::Confirmed,
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub(crate) thunk_isa: ThunkIsa,
    pub(crate) load_address: u32,
    pub(crate) thunk_address: u32,
    pub(crate) swaps: Vec<Swap>,
    pub(crate) image_address: u32,
    pub(crate) image_entry: u32,
    pub(crate) confirmation: Confirmation,
}

/// Why a layout can't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The layout is null or missing: the SoC's layout is not sourced.
    Unsourced,
    /// The JSON doesn't match the shape the layout takes.
    Malformed(String),
    /// Two regions that must not overlap do.
    Overlap(String),
    /// A count, size or alignment the thunk can't use.
    Invalid(String),
    /// The layout asks for something no thunk is written for, or for a flow
    /// that can't follow it (an image after an SPL that does not return).
    Unsupported(String),
    /// The record is UNCONFIRMED and the caller did not acknowledge it.
    Unconfirmed(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsourced => f.write_str(
                "the SRAM layout is not sourced for this SoC (UNCONFIRMED: null); \
                 FEL cannot run the SPL until a description gives it",
            ),
            Self::Malformed(message) => write!(f, "the SRAM layout is malformed: {message}"),
            Self::Overlap(message) => write!(f, "the SRAM layout overlaps itself: {message}"),
            Self::Invalid(message) => write!(f, "the SRAM layout is unusable: {message}"),
            Self::Unsupported(message) => write!(f, "the SRAM layout is unsupported: {message}"),
            Self::Unconfirmed(message) => write!(f, "the SRAM layout is UNCONFIRMED: {message}"),
        }
    }
}

impl std::error::Error for LayoutError {}

/// One piece of the SPL to write: `len` bytes of the image starting at
/// `offset`, to SRAM address `address`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub offset: usize,
    pub address: u32,
    pub len: usize,
}

fn hex_u32(value: &Value, field: &str) -> Result<u32, LayoutError> {
    let text = value
        .as_str()
        .ok_or_else(|| LayoutError::Malformed(format!("{field} is not a hex string")))?;
    let digits = text
        .strip_prefix("0x")
        .ok_or_else(|| LayoutError::Malformed(format!("{field} is not 0x-prefixed")))?;
    u32::from_str_radix(digits, 16)
        .map_err(|_| LayoutError::Malformed(format!("{field} is not a 32-bit hex number")))
}

impl Layout {
    /// Makes a layout from its parts, running the layout's own rules. The
    /// confirmation says whether it is proven and, when it is not, whether
    /// the caller acknowledges it.
    pub fn new(parts: LayoutParts, confirmation: Confirmation) -> Result<Self, LayoutError> {
        let layout = Self {
            thunk_isa: parts.thunk_isa,
            load_address: parts.load_address,
            thunk_address: parts.thunk_address,
            swaps: parts.swaps,
            image_address: parts.image_address,
            image_entry: parts.image_entry,
            confirmation,
        };
        layout.check()?;
        Ok(layout)
    }

    /// The layout's confirmation.
    pub fn confirmation(&self) -> Confirmation {
        self.confirmation
    }

    /// Reads a layout record. Null or missing is [`LayoutError::Unsourced`].
    /// An UNCONFIRMED record is read with no acknowledgement given: it can be
    /// read with, but no device write or run accepts it. This is the default
    /// entry point; it is [`Layout::from_record`] with
    /// [`Acknowledgement::Withheld`].
    pub fn from_value(sram: Option<&Value>) -> Result<Self, LayoutError> {
        Self::from_record(sram, Acknowledgement::Withheld)
    }

    /// Reads a layout record. Fields: `thunk_isa` ("armv7", "aarch64" or
    /// "aarch32-rmr"), `confirmed` (true or false: a record whose layout is
    /// not yet proven on hardware is false), `reset_vector` (a hex string,
    /// required by "aarch32-rmr"), `load_address`, `thunk_address`,
    /// `image_address`, `image_entry` (hex strings), and `swap_buffers` (a
    /// list of `{buf1, buf2, size}` hex strings). Any other field, such as a
    /// `note`, is data for a reader and is ignored.
    ///
    /// `acknowledge` matters only when the record is UNCONFIRMED: it is the
    /// caller's acknowledgement that a guessed layout may be used on a device.
    pub fn from_record(
        sram: Option<&Value>,
        acknowledge: Acknowledgement,
    ) -> Result<Self, LayoutError> {
        let sram = match sram {
            None | Some(Value::Null) => return Err(LayoutError::Unsourced),
            Some(value) => value,
        };
        let field = |name: &str| {
            sram.get(name)
                .ok_or_else(|| LayoutError::Malformed(format!("no {name}")))
        };
        let confirmed = field("confirmed")?
            .as_bool()
            .ok_or_else(|| LayoutError::Malformed("confirmed is not true or false".into()))?;
        let confirmation = if confirmed {
            Confirmation::Confirmed
        } else {
            Confirmation::Unconfirmed(acknowledge)
        };
        let isa = match field("thunk_isa")?.as_str() {
            Some("armv7") => ThunkIsa::Armv7,
            Some("aarch64") => ThunkIsa::Aarch64,
            Some("aarch32-rmr") => ThunkIsa::Aarch32ToAarch64 {
                reset_vector: hex_u32(field("reset_vector")?, "reset_vector")?,
            },
            _ => {
                return Err(LayoutError::Malformed(
                    "thunk_isa is not \"armv7\", \"aarch64\" or \"aarch32-rmr\"".into(),
                ));
            }
        };
        let swaps_value = field("swap_buffers")?
            .as_array()
            .ok_or_else(|| LayoutError::Malformed("swap_buffers is not a list".into()))?;
        let mut swaps = Vec::with_capacity(swaps_value.len());
        for swap in swaps_value {
            let part = |name: &str| {
                swap.get(name)
                    .ok_or_else(|| LayoutError::Malformed(format!("a swap has no {name}")))
                    .and_then(|value| hex_u32(value, name))
            };
            swaps.push(Swap {
                buf1: part("buf1")?,
                buf2: part("buf2")?,
                size: part("size")?,
            });
        }
        Self::new(
            LayoutParts {
                thunk_isa: isa,
                load_address: hex_u32(field("load_address")?, "load_address")?,
                thunk_address: hex_u32(field("thunk_address")?, "thunk_address")?,
                swaps,
                image_address: hex_u32(field("image_address")?, "image_address")?,
                image_entry: hex_u32(field("image_entry")?, "image_entry")?,
            },
            confirmation,
        )
    }

    /// Refuses a device write or run with an UNCONFIRMED layout whose
    /// acknowledgement was withheld. Every device-touching entry point calls
    /// this before its first write.
    pub(crate) fn may_write(&self) -> Result<(), LayoutError> {
        match self.confirmation {
            Confirmation::Unconfirmed(Acknowledgement::Withheld) => Err(LayoutError::Unconfirmed(
                "the layout is not proven on hardware and its acknowledgement was withheld; \
                 no write or run is made with it"
                    .into(),
            )),
            Confirmation::Confirmed | Confirmation::Unconfirmed(Acknowledgement::Given) => Ok(()),
        }
    }

    /// The layout's own rules: alignment, sizes, a table the thunk holds,
    /// and no overlap between the thunk and the swap buffers.
    pub fn check(&self) -> Result<(), LayoutError> {
        if let ThunkIsa::Aarch32ToAarch64 { reset_vector } = self.thunk_isa
            && (reset_vector == 0 || reset_vector % 4 != 0)
        {
            return Err(LayoutError::Invalid(format!(
                "the reset vector register {reset_vector:#010x} is not a 4-byte address"
            )));
        }
        if self.swaps.len() > TABLE_ENTRIES {
            return Err(LayoutError::Invalid(format!(
                "{} swap buffers; the thunk's table holds {TABLE_ENTRIES}",
                self.swaps.len()
            )));
        }
        for swap in &self.swaps {
            if swap.size == 0 || swap.size % 4 != 0 {
                return Err(LayoutError::Invalid(format!(
                    "a swap of {} bytes (must be a non-zero multiple of 4)",
                    swap.size
                )));
            }
            if swap.buf1 % 4 != 0 || swap.buf2 % 4 != 0 {
                return Err(LayoutError::Invalid(
                    "a swap buffer is not 4-byte aligned".into(),
                ));
            }
        }
        for address in [
            self.load_address,
            self.thunk_address,
            self.image_address,
            self.image_entry,
        ] {
            if address % 4 != 0 {
                return Err(LayoutError::Invalid(format!(
                    "{address:#010x} is not 4-byte aligned"
                )));
            }
        }
        let thunk = (self.thunk_address, self.thunk_isa.blob_len() as u32);
        for (index, swap) in self.swaps.iter().enumerate() {
            let buf1 = (swap.buf1, swap.size);
            let buf2 = (swap.buf2, swap.size);
            if overlaps(buf1, buf2) {
                return Err(LayoutError::Overlap(format!(
                    "swap {index}: buf1 and buf2 share SRAM"
                )));
            }
            if overlaps(thunk, buf1) || overlaps(thunk, buf2) {
                return Err(LayoutError::Overlap(format!(
                    "the thunk at {:#010x} sits in swap {index}",
                    self.thunk_address
                )));
            }
            for (other, later) in self.swaps.iter().enumerate().skip(index + 1) {
                if overlaps(buf1, (later.buf1, later.size))
                    || overlaps(buf1, (later.buf2, later.size))
                    || overlaps(buf2, (later.buf1, later.size))
                    || overlaps(buf2, (later.buf2, later.size))
                {
                    return Err(LayoutError::Overlap(format!("swaps {index} and {other}")));
                }
            }
        }
        Ok(())
    }

    /// The writes that put an SPL of `len` bytes in SRAM: the parts that
    /// land on a swap's buf1 go to its buf2 instead, so the thunk's swap
    /// moves them into place. Fails when the SPL would overlap the thunk
    /// or a backup buffer that the SPL does not itself redirect to.
    pub fn spl_pieces(&self, len: usize) -> Result<Vec<Piece>, LayoutError> {
        let start = self.load_address as u64;
        let end = start + len as u64;
        if end > u64::from(u32::MAX) {
            return Err(LayoutError::Invalid(
                "the SPL runs past the address space".into(),
            ));
        }
        let thunk = (self.thunk_address as u64, self.thunk_isa.blob_len() as u64);
        let spl = (start, len as u64);
        if overlaps64(spl, thunk) {
            return Err(LayoutError::Overlap("the SPL overlaps the thunk".into()));
        }
        // Cut the SPL at every swap edge inside it.
        let mut cuts = vec![start, end];
        for swap in &self.swaps {
            for edge in [swap.buf1 as u64, swap.buf1 as u64 + u64::from(swap.size)] {
                if edge > start && edge < end {
                    cuts.push(edge);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut pieces = Vec::new();
        for window in cuts.windows(2) {
            let (from, to) = (window[0], window[1]);
            let swap = self.swaps.iter().find(|swap| {
                let buf1 = swap.buf1 as u64;
                from >= buf1 && to <= buf1 + u64::from(swap.size)
            });
            let address = match swap {
                // Written to the backup; the thunk's swap moves it to buf1.
                Some(swap) => u64::from(swap.buf2) + (from - swap.buf1 as u64),
                None => {
                    if self.swaps.iter().any(|swap| {
                        overlaps64((from, to - from), (swap.buf2 as u64, u64::from(swap.size)))
                    }) {
                        return Err(LayoutError::Overlap(format!(
                            "the SPL at {from:#010x} lies in a backup buffer"
                        )));
                    }
                    from
                }
            };
            if overlaps64((address, to - from), thunk) {
                return Err(LayoutError::Overlap(
                    "a redirected SPL piece lands in the thunk".into(),
                ));
            }
            pieces.push(Piece {
                offset: (from - start) as usize,
                address: address as u32,
                len: (to - from) as usize,
            });
        }
        Ok(pieces)
    }
}

fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
    overlaps64((a.0 as u64, a.1 as u64), (b.0 as u64, b.1 as u64))
}

fn overlaps64(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.0 + b.1 && b.0 < a.0 + a.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A made-up layout (not the A733's): one 0x800-byte stack region at
    /// 0x1800 backed up at 0x44000, the thunk at 0x20000, the SPL at 0x2000.
    fn sample() -> Layout {
        Layout {
            thunk_isa: ThunkIsa::Armv7,
            load_address: 0x2000,
            thunk_address: 0x20000,
            swaps: vec![Swap {
                buf1: 0x1800,
                buf2: 0x44000,
                size: 0x800,
            }],
            image_address: 0x4a00_0000,
            image_entry: 0x4a00_0000,
            confirmation: Confirmation::Confirmed,
        }
    }

    fn value() -> Value {
        json!({
            "thunk_isa": "armv7",
            "confirmed": true,
            "load_address": "0x00002000",
            "thunk_address": "0x00020000",
            "image_address": "0x4a000000",
            "image_entry": "0x4a000000",
            "swap_buffers": [{"buf1": "0x00001800", "buf2": "0x00044000", "size": "0x00000800"}]
        })
    }

    #[test]
    fn null_or_missing_sram_is_unsourced() {
        assert_eq!(Layout::from_value(None), Err(LayoutError::Unsourced));
        assert_eq!(
            Layout::from_value(Some(&Value::Null)),
            Err(LayoutError::Unsourced)
        );
    }

    #[test]
    fn a_layout_reads_from_its_hex_fields() {
        assert_eq!(Layout::from_value(Some(&value())).unwrap(), sample());
        let mut bad = value();
        bad["load_address"] = json!("2000");
        assert!(matches!(
            Layout::from_value(Some(&bad)),
            Err(LayoutError::Malformed(_))
        ));
    }

    #[test]
    fn an_unknown_thunk_isa_is_refused_and_the_known_ones_read() {
        let mut unknown = value();
        unknown["thunk_isa"] = json!("riscv");
        assert!(matches!(
            Layout::from_value(Some(&unknown)),
            Err(LayoutError::Malformed(_))
        ));
        let mut a64 = value();
        a64["thunk_isa"] = json!("aarch64");
        assert_eq!(
            Layout::from_value(Some(&a64)).unwrap().thunk_isa,
            ThunkIsa::Aarch64
        );
    }

    #[test]
    fn the_rmr_mode_needs_a_nonzero_reset_vector_register() {
        let mut rmr = value();
        rmr["thunk_isa"] = json!("aarch32-rmr");
        assert!(matches!(
            Layout::from_value(Some(&rmr)),
            Err(LayoutError::Malformed(_))
        ));
        rmr["reset_vector"] = json!("0x08001004");
        assert_eq!(
            Layout::from_value(Some(&rmr)).unwrap().thunk_isa,
            ThunkIsa::Aarch32ToAarch64 {
                reset_vector: 0x0800_1004
            }
        );
        rmr["reset_vector"] = json!("0x00000000");
        assert!(matches!(
            Layout::from_value(Some(&rmr)),
            Err(LayoutError::Invalid(_))
        ));
    }

    #[test]
    fn an_unconfirmed_record_reads_and_carries_its_acknowledgement() {
        let mut guessed = value();
        guessed["confirmed"] = json!(false);
        assert_eq!(
            Layout::from_value(Some(&guessed)).unwrap().confirmation(),
            Confirmation::Unconfirmed(Acknowledgement::Withheld)
        );
        assert_eq!(
            Layout::from_record(Some(&guessed), Acknowledgement::Given)
                .unwrap()
                .confirmation(),
            Confirmation::Unconfirmed(Acknowledgement::Given)
        );
        let mut missing = value();
        missing.as_object_mut().unwrap().remove("confirmed");
        assert!(matches!(
            Layout::from_value(Some(&missing)),
            Err(LayoutError::Malformed(_))
        ));
    }

    #[test]
    fn a_note_beside_the_fields_is_data_and_is_ignored() {
        let mut noted = value();
        noted["note"] = json!("a reader's note");
        assert_eq!(Layout::from_value(Some(&noted)).unwrap(), sample());
    }

    #[test]
    fn each_mode_says_whether_its_spl_returns_to_fel() {
        assert_eq!(ThunkIsa::Armv7.return_mode(), ReturnMode::ReturnsToFel);
        assert_eq!(ThunkIsa::Aarch64.return_mode(), ReturnMode::ReturnsToFel);
        assert_eq!(
            ThunkIsa::Aarch32ToAarch64 {
                reset_vector: 0x0800_1004
            }
            .return_mode(),
            ReturnMode::NoReturnExpected
        );
    }

    #[test]
    fn a_thunk_inside_a_swap_is_refused() {
        let mut layout = sample();
        layout.thunk_address = 0x1900;
        assert!(matches!(layout.check(), Err(LayoutError::Overlap(_))));
    }

    #[test]
    fn a_spl_inside_the_stack_is_redirected_to_its_backup() {
        // An SPL of 0x800 bytes loaded at the stack buffer's own address
        // (0x1800..0x2000) lands wholly on it, so it all goes to the backup.
        let mut layout = sample();
        layout.load_address = 0x1800;
        let pieces = layout.spl_pieces(0x800).unwrap();
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].address, 0x44000);
        assert_eq!(pieces[0].len, 0x800);
        assert_eq!(pieces[0].offset, 0);
    }

    #[test]
    fn a_spl_crossing_a_stack_edge_is_cut_there() {
        // 0x1000..0x2000 from 0x1000, then the 0x1800 edge splits it.
        let mut layout = sample();
        layout.load_address = 0x1000;
        let pieces = layout.spl_pieces(0x1000).unwrap();
        assert_eq!(
            pieces,
            vec![
                Piece {
                    offset: 0,
                    address: 0x1000,
                    len: 0x800
                },
                Piece {
                    offset: 0x800,
                    address: 0x44000,
                    len: 0x800
                },
            ]
        );
    }

    #[test]
    fn an_spl_on_the_thunk_is_refused() {
        let mut layout = sample();
        layout.load_address = 0x1f000;
        assert!(matches!(
            layout.spl_pieces(0x2000),
            Err(LayoutError::Overlap(_))
        ));
    }

    /// The A733's record, as shipped: UNCONFIRMED. It reads, and nothing
    /// writes with it until the caller acknowledges it.
    const A733: &str = include_str!("../records/sun60i-a733.json");

    #[test]
    fn the_a733_record_reads_unconfirmed_and_its_addresses_are_the_pinned_ones() {
        let record: Value = serde_json::from_str(A733).unwrap();
        let layout = Layout::from_value(Some(&record)).unwrap();
        assert_eq!(
            layout.confirmation(),
            Confirmation::Unconfirmed(Acknowledgement::Withheld)
        );
        assert_eq!(
            layout.thunk_isa,
            ThunkIsa::Aarch32ToAarch64 {
                reset_vector: 0x0800_1004
            }
        );
        assert_eq!(layout.load_address, 0x0004_7000);
        assert_eq!(layout.thunk_address, 0x0009_0000);
        assert!(layout.swaps.is_empty());
    }
}
