//! A dry run of a Rockchip flash against a mock board: no USB device. It
//! walks the maskrom download, the chip check, the storage (read back), the
//! flash info, the image written from LBA 0 and read back, and a reset. Every
//! transfer is logged at info, in hex.
//!
//! cargo run -p atlas-socflash --example dry_run -- LOADER IMAGE CHIP [emmc|sd|spi-nor] [SECTORS]
//!
//! CHIP is the chip number the loader is for ("3588"). SECTORS is the mock's
//! storage (default 65536, 32 MiB, held in RAM).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use atlas_socflash::rockchip::rockusb::Storage;
use atlas_socflash::rockchip::sim::{Board, walk};

fn usage() -> ! {
    eprintln!("usage: dry_run LOADER IMAGE CHIP [emmc|sd|spi-nor] [SECTORS]");
    std::process::exit(2)
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(loader_path), Some(image_path), Some(chip)) =
        (args.first(), args.get(1), args.get(2))
    else {
        usage()
    };
    let storage = match args.get(3).map(String::as_str) {
        None | Some("emmc") => Storage::Emmc,
        Some("sd") => Storage::Sd,
        Some("spi-nor") => Storage::SpiNor,
        Some(_) => usage(),
    };
    let sectors: u32 = match args.get(4) {
        Some(text) => text.parse().unwrap_or_else(|_| usage()),
        None => 65_536,
    };
    let loader = std::fs::read(loader_path).unwrap_or_else(|error| {
        eprintln!("{loader_path}: {error}");
        std::process::exit(1)
    });
    let image = std::fs::read(image_path).unwrap_or_else(|error| {
        eprintln!("{image_path}: {error}");
        std::process::exit(1)
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    // The mock has every storage id, so the storage switch is the one asked for.
    let board = Board::new(
        chip,
        sectors,
        &[Storage::Emmc, Storage::Sd, Storage::SpiNor],
    );
    match runtime.block_on(walk(&loader, &image, 0, Some(storage), board)) {
        Ok(written) => println!(
            "dry run ok: wrote and read back {} bytes, sha256 {}",
            written.bytes, written.sha256
        ),
        Err(error) => {
            eprintln!("dry run failed: {error}");
            std::process::exit(1)
        }
    }
}
