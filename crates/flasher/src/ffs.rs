//! The gadget's vendor interface, through FunctionFS (`usb_f_fs`).
//!
//! /init makes the gadget in configfs with an `ffs.flasher` function and
//! mounts it (`mount -t functionfs flasher <dir>`). Writing the interface's
//! descriptors and strings to `<dir>/ep0` brings up `ep1` (bulk IN, to
//! Atlas) and `ep2` (bulk OUT, from Atlas); only then can /init bind the
//! gadget to the USB controller. ep0 stays open for as long as the
//! interface lives, and its events say when Atlas has it configured.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use atlas_flasher_proto::{INTERFACE, INTERFACE_CLASS};

const DESCRIPTORS_MAGIC_V2: u32 = 3;
const STRINGS_MAGIC: u32 = 2;
const HAS_FS_DESC: u32 = 1;
const HAS_HS_DESC: u32 = 2;

const EVENT_LEN: usize = 12;
const EVENT_ENABLE: u8 = 2;
const EVENT_DISABLE: u8 = 3;
const EVENT_UNBIND: u8 = 1;
const EVENT_SETUP: u8 = 4;

/// One interface, a bulk IN then a bulk OUT endpoint, at `packet` bytes.
fn interface(packet: u16) -> Vec<u8> {
    let [lo, hi] = packet.to_le_bytes();
    vec![
        // Interface: 2 endpoints, vendor class, string 1.
        9,
        4,
        0,
        0,
        2,
        INTERFACE_CLASS,
        0,
        0,
        1,
        // Bulk IN (ep1).
        7,
        5,
        0x81,
        2,
        lo,
        hi,
        0,
        // Bulk OUT (ep2).
        7,
        5,
        0x02,
        2,
        lo,
        hi,
        0,
    ]
}

pub fn descriptors() -> Vec<u8> {
    let full = interface(64);
    let high = interface(512);
    let mut out = Vec::new();
    out.extend(DESCRIPTORS_MAGIC_V2.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // length, below
    out.extend((HAS_FS_DESC | HAS_HS_DESC).to_le_bytes());
    out.extend(3u32.to_le_bytes());
    out.extend(3u32.to_le_bytes());
    out.extend(full);
    out.extend(high);
    let len = out.len() as u32;
    out[4..8].copy_from_slice(&len.to_le_bytes());
    out
}

pub fn strings() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(STRINGS_MAGIC.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(1u32.to_le_bytes()); // strings per language
    out.extend(1u32.to_le_bytes()); // languages
    out.extend(0x0409u16.to_le_bytes());
    out.extend(INTERFACE.as_bytes());
    out.push(0);
    let len = out.len() as u32;
    out[4..8].copy_from_slice(&len.to_le_bytes());
    out
}

/// The live interface: ep0 held open, and whether Atlas has enabled it.
pub struct Function {
    dir: PathBuf,
    enabled: Arc<AtomicBool>,
}

impl Function {
    /// Writes the descriptors, which brings up ep1 and ep2, and follows
    /// ep0's events from then on.
    pub fn start(dir: &Path) -> io::Result<Self> {
        let mut ep0 = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join("ep0"))?;
        ep0.write_all(&descriptors())?;
        ep0.write_all(&strings())?;
        let enabled = Arc::new(AtomicBool::new(false));
        let flag = enabled.clone();
        thread::spawn(move || follow_events(ep0, &flag));
        Ok(Self {
            dir: dir.to_path_buf(),
            enabled,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The endpoints for one session: (IN, OUT).
    pub fn endpoints(&self) -> io::Result<(File, File)> {
        let to_host = OpenOptions::new().write(true).open(self.dir.join("ep1"))?;
        let from_host = OpenOptions::new().read(true).open(self.dir.join("ep2"))?;
        Ok((to_host, from_host))
    }
}

fn follow_events(mut ep0: File, enabled: &AtomicBool) {
    let mut event = [0u8; EVENT_LEN * 4];
    loop {
        let n = match ep0.read(&mut event) {
            Ok(n) if n >= EVENT_LEN => n,
            _ => {
                thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
        };
        for one in event[..n].as_chunks::<EVENT_LEN>().0 {
            match one[8] {
                EVENT_ENABLE => enabled.store(true, Ordering::Relaxed),
                EVENT_DISABLE | EVENT_UNBIND => enabled.store(false, Ordering::Relaxed),
                // No control requests are ours to answer: stall them (a
                // zero-length read for IN, a write for OUT).
                EVENT_SETUP => {
                    let device_to_host = one[0] & 0x80 != 0;
                    let _ = if device_to_host {
                        ep0.read(&mut [])
                    } else {
                        ep0.write(&[])
                    };
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_blobs_say_their_own_length() {
        let blob = descriptors();
        assert_eq!(
            u32::from_le_bytes(blob[4..8].try_into().unwrap()) as usize,
            blob.len()
        );
        assert_eq!(blob.len(), 20 + 2 * 23);
        let strings = strings();
        assert_eq!(
            u32::from_le_bytes(strings[4..8].try_into().unwrap()) as usize,
            strings.len()
        );
        assert!(strings.ends_with(b"atlas-flasher\0"));
    }
}
