//! Writing an image keeps memory flat whatever its size: decoded data
//! streams through fixed buffers into the disk, and the read-back hashes as
//! it goes. A 512 MiB image must not take more than a few dozen MiB. (A
//! 1.4 GB HeliOS .img.xz written and verified this way peaked at 82 MB in
//! all, decoder included.)

use std::sync::atomic::AtomicBool;

fn peak_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

#[test]
#[cfg(target_os = "linux")]
fn a_large_image_is_written_in_bounded_memory() {
    const SIZE: u64 = 512 << 20;
    let dir = std::env::temp_dir().join(format!("atlas-bounded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Sparse: nothing is stored until written, so the test needs little disk.
    let image = dir.join("big.img");
    std::fs::File::create(&image)
        .unwrap()
        .set_len(SIZE)
        .unwrap();
    let target = dir.join("disk");
    std::fs::File::create(&target)
        .unwrap()
        .set_len(SIZE)
        .unwrap();
    let disk = atlas_blockdev::Disk {
        path: target.clone(),
        name: "disk".into(),
        size_bytes: SIZE,
        vendor: None,
        model: None,
        serial: None,
        removable: true,
        usb: true,
        usb_port: None,
        system: false,
        mount_points: Vec::new(),
    };
    let report = atlas_blockdev::write_image(
        &image,
        &disk,
        &atlas_blockdev::WriteOptions {
            verify: true,
            image_sha256: None,
        },
        &mut |_| {},
        &AtomicBool::new(false),
    );
    let peak = peak_kib();
    let _ = std::fs::remove_dir_all(&dir);
    let report = report.unwrap();
    assert_eq!(report.bytes_written, SIZE);
    assert!(report.verified);
    assert!(peak < 96 * 1024, "peak resident memory {} MiB", peak / 1024);
}
