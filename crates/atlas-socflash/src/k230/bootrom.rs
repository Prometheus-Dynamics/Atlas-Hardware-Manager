//! The K230 boot ROM's USB download mode (stage 1): EP0 vendor requests to
//! read the CPU info and set the load address, the loader over bulk in
//! 1000-byte pages, then a request to run it.

use crate::transport::{UsbError, UsbIo};

const GET_CPU_INFO: u8 = 0;
const SET_DATA_ADDRESS: u8 = 1;
const PROG_START: u8 = 4;
const CPU_INFO_LEN: u16 = 32;
/// The boot ROM's bulk page.
pub const PAGE: usize = 1000;
/// Where k230_flash loads and starts the burner. U-Boot's burntool
/// defconfig links at 0x80300000 instead; which the boot ROM wants isn't
/// settled (UNCONFIRMED), so a device record can give its own.
pub const DEFAULT_LOAD_ADDRESS: u32 = 0x8036_0000;

/// Which stage answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The boot ROM ("K230").
    BootRom,
    /// The burner U-Boot ("Uboot Stage for K230").
    Burner,
    Unknown(String),
}

pub async fn stage(io: &mut dyn UsbIo) -> Result<Stage, UsbError> {
    let reply = io.control_in(GET_CPU_INFO, 0, 0, CPU_INFO_LEN).await?;
    let text = String::from_utf8_lossy(&reply)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    Ok(if text.starts_with("Uboot Stage") {
        Stage::Burner
    } else if text.starts_with("K230") {
        Stage::BootRom
    } else {
        Stage::Unknown(text)
    })
}

fn split(address: u32) -> (u16, u16) {
    ((address >> 16) as u16, (address & 0xffff) as u16)
}

/// Loads `loader` at `address` and runs it. The board then re-enumerates as
/// the burner on the same port.
pub async fn run_loader(io: &mut dyn UsbIo, loader: &[u8], address: u32) -> Result<(), UsbError> {
    let (high, low) = split(address);
    io.control_out(SET_DATA_ADDRESS, high, low, &[]).await?;
    for page in loader.chunks(PAGE) {
        io.bulk_out(page).await?;
    }
    match io.control_out(PROG_START, high, low, &[]).await {
        // The ROM may jump before it acknowledges.
        Err(UsbError::Disconnected) => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Script, Step};

    #[tokio::test]
    async fn the_stage_is_told_by_its_cpu_info() {
        let info = |reply: &[u8]| Step::ControlIn {
            request: 0,
            value: 0,
            index: 0,
            length: 32,
            reply: reply.to_vec(),
        };
        let mut script = Script::new(vec![
            info(b"K230\0\0\0\0"),
            info(b"Uboot Stage for K230\0"),
            info(b"something"),
        ]);
        assert_eq!(stage(&mut script).await.unwrap(), Stage::BootRom);
        assert_eq!(stage(&mut script).await.unwrap(), Stage::Burner);
        assert_eq!(
            stage(&mut script).await.unwrap(),
            Stage::Unknown("something".into())
        );
    }

    #[tokio::test]
    async fn the_loader_goes_in_1000_byte_pages_between_address_and_start() {
        let loader: Vec<u8> = (0..2500u32).map(|i| i as u8).collect();
        let request = |request: u8| Step::ControlOut {
            request,
            value: 0x8036,
            index: 0x0000,
            data: Vec::new(),
        };
        let mut script = Script::new(vec![
            request(1),
            Step::BulkOut(loader[..1000].to_vec()),
            Step::BulkOut(loader[1000..2000].to_vec()),
            Step::BulkOut(loader[2000..].to_vec()),
            request(4),
        ]);
        run_loader(&mut script, &loader, DEFAULT_LOAD_ADDRESS)
            .await
            .unwrap();
        assert!(script.finished());
    }
}
