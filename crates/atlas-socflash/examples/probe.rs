//! A read-only probe of one Rockchip board in maskrom or loader mode: the
//! loader's DDR init and usbplug if the board is in maskrom, then the chip,
//! the flash ID, the flash info, the storage and the capability. It writes
//! nothing, changes no storage and resets nothing: the board stays in loader
//! mode. Every transfer is printed in hex.
//!
//! cargo run -p atlas-socflash --example probe -- LOADER
//!
//! Run it on one board; with two Rockchip devices it refuses to guess.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use atlas_socflash::rockchip::loader::Loader;
use atlas_socflash::rockchip::rockusb::{Rockusb, chip_number};
use atlas_socflash::rockchip::{Mode, list, maskrom};
use atlas_socflash::{NusbIo, Traced};

const BULK_TIMEOUT: Duration = Duration::from_secs(10);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn probe(path: &str) -> Result<(), String> {
    let file = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    let loader = Loader::parse(&file).map_err(|error| error.to_string())?;
    println!(
        "loader {path}: chip {}, {} bytes",
        loader.chip_number().unwrap_or_else(|| "unknown".into()),
        file.len()
    );
    let found = list().await.map_err(|error| error.to_string())?;
    let (device, info) = match <[_; 1]>::try_from(found) {
        Ok([one]) => one,
        Err(many) => {
            return Err(format!(
                "{} Rockchip devices in a download mode; unplug all but the one to probe",
                many.len()
            ));
        }
    };
    println!(
        "device {}: product {:#06x}, {:?}",
        device.location, device.product_id, device.mode
    );
    if device.mode == Mode::Maskrom {
        println!("maskrom: sending the loader's DDR init and usbplug");
        let io = NusbIo::open(&info, "Rockchip maskrom device", BULK_TIMEOUT)
            .await
            .map_err(|error| error.to_string())?;
        let mut io = Traced::new(Box::new(io), "maskrom");
        maskrom::download_boot(&mut io, &loader, &file)
            .await
            .map_err(|error| error.to_string())?;
    } else {
        println!("already in loader mode; no maskrom download");
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let info = loop {
        let found = list().await.map_err(|error| error.to_string())?;
        if let Some((_, info)) = found.into_iter().find(|(d, _)| d.mode == Mode::Loader) {
            break info;
        }
        if Instant::now() > deadline {
            return Err("no loader-mode board 30 s after the usbplug".into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let io = NusbIo::open(&info, "Rockchip loader", BULK_TIMEOUT)
        .await
        .map_err(|error| error.to_string())?;
    let mut io = Traced::new(Box::new(io), "loader");
    let mut device = Rockusb::new(&mut io);
    for attempt in 0..10 {
        match device.test_unit_ready().await {
            Ok(()) => break,
            Err(_) if attempt < 9 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(error) => return Err(error.to_string()),
        }
    }
    let chip = device
        .read_chip_info()
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "chip: reply {} reads as {}",
        hex(&chip),
        chip_number(&chip).unwrap_or_else(|| "unreadable".into())
    );
    let id = device
        .read_flash_id()
        .await
        .map_err(|error| error.to_string())?;
    println!("flash id: {}", hex(&id));
    let flash = device
        .read_flash_info()
        .await
        .map_err(|error| error.to_string())?;
    println!(
        "flash info: {} sectors ({} bytes), block {} sectors, page {} sectors, ecc {}, access {}, manufacturer {}, cs {}",
        flash.sectors,
        flash.bytes(),
        flash.block_sectors,
        flash.page_sectors,
        flash.ecc_bits,
        flash.access_time,
        flash.manufacturer,
        flash.chip_select
    );
    println!("flash info raw: {}", hex(&flash.raw));
    let storage = device
        .read_storage()
        .await
        .map_err(|error| error.to_string())?;
    println!("storage: now id {storage:?}");
    let capability = device
        .read_capability()
        .await
        .map_err(|error| error.to_string())?;
    println!("capability: {}", hex(&capability));
    println!("probe done: nothing written; the board stays in loader mode");
    Ok(())
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: probe LOADER");
        std::process::exit(2)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    if let Err(error) = runtime.block_on(probe(&path)) {
        eprintln!("probe failed: {error}");
        std::process::exit(1)
    }
}
