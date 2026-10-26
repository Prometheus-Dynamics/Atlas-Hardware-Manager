// The version an Atlas binary reports, for its build.rs (`include!`d):
// ATLAS_VERSION when set (a tagged release), else
// `<release>-dev.<commit count>+<commit>` from the workspace's
// `[workspace.metadata.atlas] release` and git (`<release>-dev` without
// git). Exported as ATLAS_BUILD_VERSION.

fn atlas_build_version() {
    use std::path::Path;
    use std::process::Command;

    println!("cargo:rerun-if-env-changed=ATLAS_VERSION");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join("scripts/build-version.rs").is_file())
        .expect("the Atlas workspace root")
        .to_path_buf();
    println!("cargo:rerun-if-changed={}", root.join("Cargo.toml").display());
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|out| !out.is_empty())
    };
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        // HEAD's log changes on every commit, checkout and reset.
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/logs/HEAD");
    }
    let version = match std::env::var("ATLAS_VERSION").ok().filter(|v| !v.is_empty()) {
        Some(version) => version,
        None => {
            let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
            let release = manifest
                .split("[workspace.metadata.atlas]")
                .nth(1)
                .and_then(|section| {
                    section.lines().find_map(|line| {
                        let value = line.trim().strip_prefix("release")?.trim_start();
                        Some(value.strip_prefix('=')?.trim().trim_matches('"').to_string())
                    })
                })
                .expect("[workspace.metadata.atlas] release in Cargo.toml");
            match (
                git(&["rev-list", "--count", "HEAD"]),
                git(&["rev-parse", "--short=8", "HEAD"]),
            ) {
                (Some(count), Some(commit)) => format!("{release}-dev.{count}+{commit}"),
                _ => format!("{release}-dev"),
            }
        }
    };
    println!("cargo:rustc-env=ATLAS_BUILD_VERSION={version}");
}
