include!("../../../scripts/build-version.rs");

fn main() {
    // Release builds carry the tag's version (scripts/bundle.sh); others
    // the release being worked toward, -dev (scripts/build-version.rs).
    atlas_build_version();
    tauri_build::build();
}
