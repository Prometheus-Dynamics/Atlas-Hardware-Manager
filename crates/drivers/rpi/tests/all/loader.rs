//! The Pi's loader in the flash flow, against the simulated bus: the boot ROM takes the second stage, resets, and
//! the second stage asks for the flasher's boot set as Raze #2's did on 2026-11-22 (bench log a.log: config.txt,
//! then boot.img, then Done).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_flash_flow::{Loader, StepError, Target};
use atlas_rpi::{BootRom, RpiConfig, RpiDriver, RpiLoader};
use atlas_rpiboot::{BootFiles, BootSettings, Chip};
use atlas_usb::sim::{Answer, SimBus};
use atlas_usb::{DeviceSummary, UsbId};

const ROM_USB: UsbId = UsbId {
    vendor: 0x0a5c,
    product: 0x2712,
};
const PORT: &str = "3-4.2.1.4.4.2";

fn loader(bus: Arc<SimBus>) -> RpiLoader {
    let flasher = BootFiles::in_memory([
        ("2712/bootcode5.bin", vec![5u8; 77_236]),
        ("config.txt", b"boot_ramdisk=1\n".to_vec()),
        ("boot.img", vec![0u8; 40_000]),
    ]);
    RpiLoader::new(Arc::new(RpiDriver::new(
        bus,
        RpiConfig {
            boot_roms: vec![BootRom {
                usb: ROM_USB,
                chip: Chip::Bcm2712,
                model: "sample-cm5".into(),
            }],
            settings: BootSettings::default(),
            flasher: Some(Arc::new(flasher)),
        },
    )))
}

fn rom(serial: &str) -> DeviceSummary {
    DeviceSummary::new(PORT, ROM_USB.vendor, ROM_USB.product).with_serial(serial)
}

fn target() -> Target {
    Target {
        key: "rpi:a317bcbee5226d57".into(),
        board: "sample".into(),
        family: "rpi".into(),
        port: PORT.into(),
    }
}

fn request(command: i32, name: &str) -> Answer {
    let mut message = command.to_le_bytes().to_vec();
    message.extend_from_slice(name.as_bytes());
    message.resize(260, 0);
    Answer::Data(message)
}

#[tokio::test(start_paused = true)]
async fn the_loader_serves_the_flasher_boot_set_and_says_what_it_sent() {
    let bus = Arc::new(SimBus::new());
    bus.attach(rom("a317bcbee5226d57"));
    // Round one: length, header, length, the bootcode in 16 KiB chunks (5), then the ROM resets.
    bus.script(PORT, vec![Answer::Ok; 8]);
    bus.script(PORT, [Answer::Disconnect]);
    let loader = loader(bus.clone());
    let lines = Mutex::new(Vec::new());
    let say = |line: String| lines.lock().unwrap().push(line);

    let target = target();
    let (loaded, ()) = tokio::join!(loader.load(&target, &say), async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        bus.attach(rom("a317bcbee5226d57"));
        bus.script(
            PORT,
            [
                request(0, "config.txt"),
                Answer::Ok,
                request(1, "config.txt"),
                Answer::Ok,
                Answer::Ok,
                request(0, "boot.img"),
                Answer::Ok,
                request(1, "boot.img"),
                Answer::Ok,
                Answer::Ok,
                Answer::Ok,
                Answer::Ok,
                request(2, "boot.img"),
            ],
        );
    });

    let loaded = loaded.unwrap();
    assert!(loaded.words.contains("2 files"), "{}", loaded.words);
    assert_eq!(loaded.facts[0], ("SoC".to_string(), "BCM2712".to_string()));
    let lines = lines.into_inner().unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line == "BCM2712 took the second stage (77236 bytes)"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "sent boot.img (40000 bytes)"),
        "{lines:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn another_board_on_the_port_is_refused_and_nothing_is_sent() {
    let bus = Arc::new(SimBus::new());
    bus.attach(rom("0000000000000001"));
    let refused = loader(bus.clone()).load(&target(), &|_| {}).await;
    assert!(
        matches!(&refused, Err(StepError::Refused(words)) if words.contains("rpi:0000000000000001")),
        "{refused:?}"
    );
    assert!(
        bus.records(PORT).is_empty(),
        "no transfer reached the other board"
    );
}

#[tokio::test(start_paused = true)]
async fn an_empty_port_is_tried_again() {
    let bus = Arc::new(SimBus::new());
    let again = loader(bus).load(&target(), &|_| {}).await;
    assert!(matches!(again, Err(StepError::Again(_))), "{again:?}");
}
