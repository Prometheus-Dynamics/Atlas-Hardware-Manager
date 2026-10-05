//! Boots one Pi in USB boot mode with a boot files directory and prints
//! every step, like `rpiboot -d <dir>`. For debugging the protocol on real
//! hardware: `cargo run -p atlas-usbboot --example usbboot -- <dir> [location]`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::Instant;

use atlas_usbboot::{BootFiles, BootOptions, boot_device, list_boot_devices};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: usbboot <boot files dir> [location]");
        std::process::exit(2);
    };
    let location = args.next();
    let started = Instant::now();
    let at = move || format!("{:>6.2}s", started.elapsed().as_secs_f32());

    println!("{} devices: {:?}", at(), list_boot_devices().await);
    let files = BootFiles::open(&dir).unwrap_or_else(|error| {
        eprintln!("boot files: {error}");
        std::process::exit(1);
    });
    let options = BootOptions {
        location,
        ..BootOptions::default()
    };
    let result = boot_device(
        &files,
        &options,
        &|event| println!("{} {event:?}", at()),
        &|| false,
    )
    .await;
    println!("{} result: {result:?}", at());
}
