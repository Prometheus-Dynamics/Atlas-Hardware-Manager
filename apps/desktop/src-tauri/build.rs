fn main() {
    // Release builds carry the tag's version (scripts/bundle.sh).
    println!("cargo:rerun-if-env-changed=ATLAS_VERSION");
    tauri_build::build();
}
