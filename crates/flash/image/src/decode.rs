//! The xz decoder: liblzma, pushed bytes in and decoded bytes out.
//!
//! The decoder is `xz2`'s streaming decoder: each block is decoded as its input
//! arrives, so memory is bounded by the dictionary the stream declares (checked by
//! [`crate::xzscan`] before any decode, at most 8 MiB) and by liblzma's own memory
//! limit, [`MEMLIMIT`]. The pure-Rust `lzma-rs` decoder was used before; it decodes
//! a whole xz block into memory before it writes a byte, so an image built as one
//! block (`xz -T1`, the builder's command) would take the image's size in RAM.
//! That is measured in the tests and is why this decoder is here.

use std::io::{self, BufRead, Write};

use xz2::stream::Stream;
use xz2::write::XzDecoder;

/// liblzma's memory limit for one decode: the dictionary (at most 8 MiB, checked
/// earlier) and the decoder's own state.
pub(crate) const MEMLIMIT: u64 = 32 << 20;

/// Decodes the xz stream at `input` into `sink`. The stream must be one complete
/// stream: a cut stream fails at [`XzDecoder::finish`].
pub(crate) fn decode<R: BufRead, W: Write>(input: &mut R, sink: &mut W) -> io::Result<()> {
    let stream = Stream::new_stream_decoder(MEMLIMIT, 0).map_err(io::Error::other)?;
    let mut decoder = XzDecoder::new_stream(sink, stream);
    io::copy(input, &mut decoder)?;
    decoder.finish()?;
    Ok(())
}
