//! A bench runner for D19's board step: atlas-rpiboot boots one device on this
//! computer's own USB, from a folder, and prints a log in `rpiboot -v`'s words
//! so the two can be compared line by line. Not shipped; the app boots through
//! the Pi driver.
//!
//! ```text
//! cargo run -p atlas-rpiboot --example serve -- <port> <vendor:product> <chip> <folder>
//! ```
//!
//! `<port>` is the bus and port path (`3-4.2.1.4.4.2`), `<vendor:product>` the
//! boot ROM's id in hex, `<chip>` one of 2835, 2837, 2711, 2712. The device must
//! already be in USB boot. Nothing is written to the board's storage: the
//! files are served to its RAM.

use std::time::Duration;

use atlas_rpiboot::{BootEvent, BootFiles, BootSettings, Chip, boot_device};
use atlas_usb::{NusbBus, OpenOptions, Timeouts, UsbId};

fn chip(name: &str) -> Option<Chip> {
    Some(match name {
        "2835" => Chip::Bcm2835,
        "2837" => Chip::Bcm2837,
        "2711" => Chip::Bcm2711,
        "2712" => Chip::Bcm2712,
        _ => return None,
    })
}

fn id(text: &str) -> Option<UsbId> {
    let (vendor, product) = text.split_once(':')?;
    Some(UsbId {
        vendor: u16::from_str_radix(vendor, 16).ok()?,
        product: u16::from_str_radix(product, 16).ok()?,
    })
}

/// One event in the words `rpiboot -v` prints for the same step.
fn line(event: &BootEvent) -> String {
    match event {
        BootEvent::WaitingForDevice => "Waiting for BCM2835/6/7/2711/2712...".into(),
        BootEvent::SecondStage { bytes, .. } => format!("Sent the second stage, {bytes} bytes"),
        BootEvent::FileRequested { name, bytes } => {
            format!("Received message GetFileSize: {name}\nFile size = {bytes} bytes")
        }
        BootEvent::FileSent { name, bytes } => {
            format!(
                "Received message ReadFile: {name}\nFile read: {name}\nlibusb_bulk_transfer sent {bytes} bytes; returned 0"
            )
        }
        BootEvent::FileMissing { name } => format!("Cannot open file {name}"),
        BootEvent::Retried { what } => format!("Asked again after: {what}"),
        BootEvent::Metadata { name, value } => format!("Metadata {name} = {value}"),
        BootEvent::FileServerDone => "Second stage boot server done".into(),
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--list") {
        // What the bus lists, as `boot_device` sees it.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(runtime) = runtime else {
            return std::process::ExitCode::FAILURE;
        };
        match runtime.block_on(atlas_usb::UsbBus::devices(&NusbBus::new())) {
            Ok(devices) => {
                for device in devices {
                    println!("{device:?}");
                }
                return std::process::ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("{error}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    let (Some(port), Some(rom), Some(chip), Some(folder)) = (
        args.first(),
        args.get(1).and_then(|text| id(text)),
        args.get(2).and_then(|text| chip(text)),
        args.get(3),
    ) else {
        eprintln!("usage: serve <port> <vendor:product> <2835|2837|2711|2712> <folder>");
        return std::process::ExitCode::from(2);
    };
    let files = match BootFiles::open(folder) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let settings = BootSettings {
        open: OpenOptions {
            interface: None,
            timeouts: Timeouts::default(),
        },
        appear_timeout: Duration::from_secs(30),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let bus = NusbBus::new();
    let outcome = runtime.block_on(boot_device(
        &bus,
        port,
        rom,
        chip,
        &files,
        &settings,
        &|event| println!("{}", line(&event)),
        &|| false,
    ));
    match outcome {
        Ok(end) => {
            println!("End: {end:?}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Stopped: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
