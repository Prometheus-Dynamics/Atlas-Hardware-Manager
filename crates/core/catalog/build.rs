//! Reads the repository's `devices/` folder and writes the catalog the app
//! embeds. The reader is the same code the library and its tests use (see
//! `src/load.rs`), so a file that fails here fails the build.

#[allow(dead_code)]
#[path = "src/load.rs"]
mod load;
#[allow(dead_code)]
#[path = "src/model.rs"]
mod model;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let devices = manifest.join("../../../devices");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("catalog.json");

    // Both may be absent (no boards yet): cargo then reruns this script each
    // build, and the reader returns an empty catalog. Never an error.
    println!(
        "cargo:rerun-if-changed={}",
        devices.join("usb-ids.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        devices.join("boards").display()
    );
    println!("cargo:rerun-if-changed=src/model.rs");
    println!("cargo:rerun-if-changed=src/load.rs");

    let catalog = load::from_dir(&devices).unwrap_or_else(|e| panic!("device catalog: {e}"));
    let json = serde_json::to_string(&catalog).expect("the catalog serializes");
    std::fs::write(&out, &json).expect("write the embedded catalog");
    println!(
        "cargo:warning=embedded device catalog: {} bytes, {} boards",
        json.len(),
        catalog.boards.len()
    );
}
