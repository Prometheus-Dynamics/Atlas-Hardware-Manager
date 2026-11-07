//! Shared builders for the K230 vector tests (see k230_vectors.rs for the
//! C source line key).
#![allow(dead_code)]

use atlas_socflash::k230::kburn::{Burner, Medium};
use atlas_socflash::{Script, Step};

pub fn h(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// A 60-byte packet written out by hand: `head` hex, zero padded.
pub fn pad(head: &str) -> Vec<u8> {
    let mut bytes = h(head);
    bytes.resize(60, 0);
    bytes
}

/// U:50-59: cmd, result, data_size (u16 LE each), 54 data bytes.
pub fn pkt(cmd: u16, result: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in [cmd, result, data.len() as u16] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(data);
    if bytes.len() < 60 {
        bytes.resize(60, 0);
    }
    bytes
}

pub fn out(cmd: u16, data: &[u8]) -> Step {
    Step::BulkOut(pkt(cmd, 0, data))
}

/// A reply: U:13 sets 0x8000 on the command.
pub fn rep(cmd: u16, result: u16, data: &[u8]) -> Step {
    Step::BulkIn {
        max: 60,
        reply: pkt(cmd | 0x8000, result, data),
    }
}

pub fn words(values: &[u64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn ok(cmd: u16, data: &[u8]) -> [Step; 2] {
    [out(cmd, data), rep(cmd, 1, data)]
}

/// U:872-897: the loader version (U:139-158: IN, request 0, wValue 1, four
/// bytes), then a NOP (U:434-446).
pub fn sync(version: u32) -> Vec<Step> {
    vec![
        Step::ControlIn {
            request: 0,
            value: 1,
            index: 0,
            length: 4,
            reply: version.to_le_bytes().to_vec(),
        },
        out(0, &[]),
        rep(0, 1, &[]),
    ]
}

/// U:503-543: the probe's two u64 chunk sizes.
pub fn probe(medium: u8, out_chunk: u64, in_chunk: u64) -> Vec<Step> {
    vec![
        out(0x10, &[medium, 0xff]),
        rep(0x10, 1, &words(&[out_chunk, in_chunk])),
    ]
}

/// kburn_k230.h:46-56 / U:545-568: capacity, block, erase size, then the
/// bit-field word (timeout_ms 32 bits, wp 8, type 7, valid 1).
pub fn info(capacity: u64, block: u64, erase: u64, flags: u64) -> Vec<Step> {
    vec![
        out(0x11, &[]),
        rep(0x11, 1, &words(&[capacity, block, erase, flags])),
    ]
}

pub const VALID: u64 = 1 << 47;

pub fn ready(steps: &mut Vec<Step>, version: u32, out_chunk: u64, block: u64) {
    steps.extend(sync(version));
    steps.extend(probe(2, out_chunk, 512));
    steps.extend(info(1 << 30, block, 4096, 30000 | (2 << 40) | VALID));
}

/// Runs probe and info on a fresh burner over `script`.
pub async fn start(script: &mut Script) -> Burner<'_> {
    let mut burner = Burner::new(script);
    burner.probe(Medium::Sd).await.unwrap();
    burner.info().await.unwrap();
    burner
}
