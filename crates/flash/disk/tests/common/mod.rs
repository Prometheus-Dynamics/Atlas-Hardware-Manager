//! Shared helpers for the write tests. Everything here is a regular file in
//! the temporary directory: no test opens a block device.

#![allow(dead_code)]

use std::fs::{self, File};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use atlas_disk::Target;
use sha2::{Digest, Sha256};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh path in the temporary directory, unique to this process and call.
pub fn temp(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "atlas-disk-test-{label}-{}-{n}-{nanos}",
        std::process::id()
    ))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A regular file of `size` bytes, filled with `fill`, standing in for a disk.
pub struct FileTarget {
    file: File,
    size: u64,
    path: PathBuf,
}

impl FileTarget {
    pub fn new(size: u64, fill: u8) -> Self {
        let path = temp("target");
        fs::write(&path, vec![fill; size as usize]).unwrap();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        Self { file, size, path }
    }

    /// A sparse file of `size` bytes: nothing is stored until written, so a
    /// large target costs no memory in the test process.
    pub fn sparse(size: u64) -> Self {
        let path = temp("sparse-target");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(size).unwrap();
        Self { file, size, path }
    }

    pub fn contents(&self) -> Vec<u8> {
        fs::read(&self.path).unwrap()
    }
}

impl Drop for FileTarget {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Target for FileTarget {
    fn capacity(&self) -> u64 {
        self.size
    }

    fn identity(&self) -> String {
        self.path.display().to_string()
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.file.write_all_at(data, offset)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.read_exact_at(buf, offset)
    }

    fn reattach(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn pause(&mut self, _how_long: std::time::Duration) {}
}

/// The xz file of `raw`, made by the `xz` program with its CRC-64 check.
pub fn xz_bytes(raw: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("xz")
        .args(["--check=crc64", "--compress", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the xz program is needed to make the test images");
    let mut stdin = child.stdin.take().expect("stdin");
    let input = raw.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().expect("xz");
    writer.join().expect("writer").expect("write to xz");
    assert!(output.status.success(), "xz failed");
    output.stdout
}
