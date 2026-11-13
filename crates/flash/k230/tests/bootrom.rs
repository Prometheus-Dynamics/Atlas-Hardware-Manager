//! The boot ROM stage, against the simulated device: its stage, loading an
//! image and starting it, and the device resetting into it. The address and
//! the image are the caller's; the two addresses below are the ones dev's
//! notes record, passed in as data.

mod common;

use std::time::Duration;

use atlas_k230::bootrom::{self, Stage};
use atlas_usb::sim::Answer;
use atlas_usb::{Transfer, UsbError, await_device};
use common::*;

/// The two load addresses dev's notes record (0x80360000 and 0x80300000).
const ADDRESSES: [u32; 2] = [0x8036_0000, 0x8030_0000];

#[tokio::test]
async fn the_stage_is_told_by_its_cpu_info() {
    let bus = bus();
    bus.script(
        LOCATION,
        [
            cpu_info("K230"),
            cpu_info("Uboot Stage for K230"),
            cpu_info("something else"),
        ],
    );
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    assert_eq!(bootrom::stage(&mut *io).await.unwrap(), Stage::BootRom);
    assert_eq!(bootrom::stage(&mut *io).await.unwrap(), Stage::Burner);
    assert_eq!(
        bootrom::stage(&mut *io).await.unwrap(),
        Stage::Unknown("something else".into())
    );
}

#[tokio::test]
async fn an_image_goes_in_pages_between_the_address_and_the_start() {
    for address in ADDRESSES {
        let bus = bus();
        let image: Vec<u8> = (0..2500u32).map(|i| i as u8).collect();
        bus.script(LOCATION, vec![Answer::Ok; 5]);
        let mut io = bus.open(LOCATION, &options()).await.unwrap();
        bootrom::load_and_start(&mut *io, &image, address)
            .await
            .unwrap();

        let (high, low) = ((address >> 16) as u16, (address & 0xffff) as u16);
        let sent: Vec<Transfer> = bus
            .records(LOCATION)
            .into_iter()
            .map(|record| record.transfer)
            .collect();
        assert_eq!(
            sent,
            vec![
                Transfer::ControlOut {
                    request: 1,
                    value: high,
                    index: low,
                    data: Vec::new()
                },
                Transfer::BulkOut(image[..1000].to_vec()),
                Transfer::BulkOut(image[1000..2000].to_vec()),
                Transfer::BulkOut(image[2000..].to_vec()),
                Transfer::ControlOut {
                    request: 4,
                    value: high,
                    index: low,
                    data: Vec::new()
                },
            ],
            "{address:#010x}"
        );
    }
}

#[tokio::test]
async fn the_reset_into_the_image_is_success_and_the_device_comes_back() {
    let bus = bus();
    let image = vec![7u8; 10];
    // The boot ROM resets as the start request lands.
    bus.script(LOCATION, [Answer::Ok, Answer::Ok, Answer::Disconnect]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    bootrom::load_and_start(&mut *io, &image, ADDRESSES[0])
        .await
        .unwrap();

    // The old handle is dead: the device left the bus.
    assert!(bus.devices().await.unwrap().is_empty());
    assert_eq!(
        bootrom::stage(&mut *io).await.err(),
        Some(UsbError::Disconnected)
    );

    // It re-enumerates at the same port, now as the image it was given.
    bus.attach(device(LOCATION));
    bus.script(LOCATION, [cpu_info("Uboot Stage for K230")]);
    let mut back = await_device(&bus, LOCATION, &options(), Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(bootrom::stage(&mut *back).await.unwrap(), Stage::Burner);
}

#[tokio::test]
async fn a_reset_mid_upload_is_an_error_and_the_device_is_gone() {
    let bus = bus();
    let image = vec![1u8; 2500];
    bus.script(LOCATION, [Answer::Ok, Answer::Ok, Answer::Disconnect]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let error = bootrom::load_and_start(&mut *io, &image, ADDRESSES[1])
        .await
        .unwrap_err();
    assert_eq!(error, UsbError::Disconnected);
    // A reset is not a start: the error is returned, not taken as success.
    // The device has left the bus, and its log with it.
    assert!(bus.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_stalled_page_stops_the_load_with_the_stall() {
    let bus = bus();
    let image = vec![2u8; 1500];
    bus.script(LOCATION, [Answer::Ok, Answer::Stall]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let error = bootrom::load_and_start(&mut *io, &image, ADDRESSES[0])
        .await
        .unwrap_err();
    assert_eq!(error, UsbError::Stall);
    assert_eq!(
        bus.records(LOCATION).len(),
        2,
        "no page after the stall, no start"
    );
}

#[tokio::test(start_paused = true)]
async fn a_start_that_times_out_is_reported_after_the_control_timeout() {
    let bus = bus();
    bus.script(LOCATION, [Answer::Ok, Answer::Ok, Answer::Timeout]);
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let started = tokio::time::Instant::now();
    let error = bootrom::load_and_start(&mut *io, &[9u8; 4], ADDRESSES[0])
        .await
        .unwrap_err();
    assert!(matches!(error, UsbError::Timeout(_)), "{error:?}");
    assert_eq!(started.elapsed(), Duration::from_secs(5));
}

#[tokio::test]
async fn an_address_outside_the_range_or_an_empty_image_is_refused_before_any_transfer() {
    let bus = bus();
    let mut io = bus.open(LOCATION, &options()).await.unwrap();
    let outside = bootrom::load_and_start(&mut *io, &[1u8; 4], 0x8000_0000)
        .await
        .unwrap_err();
    assert!(outside.to_string().contains("outside"), "{outside}");
    let empty = bootrom::load_and_start(&mut *io, &[], ADDRESSES[0])
        .await
        .unwrap_err();
    assert!(empty.to_string().contains("no image"), "{empty}");
    assert!(bus.records(LOCATION).is_empty());
}
