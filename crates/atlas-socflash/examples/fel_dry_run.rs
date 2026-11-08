//! A dry run of an Allwinner FEL flash against the mock device in
//! `fel::sim`: no USB. It runs the version request, the SPL through the
//! thunk (written, read back, run, checked for the return to FEL), then the
//! legacy U-Boot image into DRAM and its run. Every transfer is logged at
//! info, in hex.
//!
//! cargo run -p atlas-socflash --example fel_dry_run -- SPL UBOOT LAYOUT.json
//!
//! LAYOUT.json has the shape of a family's `flash.sram` (see
//! devices/schema/family.schema.json). The A733's layout is not sourced, so
//! there is no shipped one; pass a test layout.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use atlas_socflash::Traced;
use atlas_socflash::fel::flash::flash;
use atlas_socflash::fel::layout::Layout;
use atlas_socflash::fel::sim::Board;

fn usage() -> ! {
    eprintln!("usage: fel_dry_run SPL UBOOT LAYOUT.json");
    std::process::exit(2)
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| {
        eprintln!("{path}: {error}");
        std::process::exit(1)
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(spl_path), Some(uboot_path), Some(layout_path)) =
        (args.first(), args.get(1), args.get(2))
    else {
        usage()
    };
    let spl = read(spl_path);
    let uboot = read(uboot_path);
    let layout_text = read(layout_path);
    let sram: serde_json::Value = serde_json::from_slice(&layout_text).unwrap_or_else(|error| {
        eprintln!("{layout_path}: {error}");
        std::process::exit(1)
    });
    let layout = Layout::from_value(Some(&sram)).unwrap_or_else(|error| {
        eprintln!("{layout_path}: {error}");
        std::process::exit(1)
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    let board = Board::new(layout.clone(), spl.len(), uboot.len().max(1));
    let mut traced = Traced::new(Box::new(board), "dry-run");
    match runtime.block_on(flash(&mut traced, &layout, &spl, &uboot)) {
        Ok(report) => println!(
            "dry run ok: SoC id {:#010x} (mock), SPL in {} write(s), U-Boot {} bytes read back and run",
            report.soc_id, report.spl_pieces, report.uboot_bytes
        ),
        Err(error) => {
            eprintln!("dry run failed: {error}");
            std::process::exit(1)
        }
    }
}
