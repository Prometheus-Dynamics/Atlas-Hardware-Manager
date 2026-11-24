//! The app's build: the version it reports, and Tauri's own build step.
//!
//! The version is the workspace rule (scripts/version-rule.sh, `atlas_version`): a tagged release
//! sets `ATLAS_VERSION`; a build from a checkout is `<release>-dev.<commit count>+<commit>`. A
//! tree that is not a git checkout reports the release with `-dev`.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ATLAS_VERSION");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for file in commit_files(&root) {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let version = std::env::var("ATLAS_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| checkout_version(&root));
    println!("cargo:rustc-env=ATLAS_APP_VERSION={version}");
    // Tauri reads the built UI (`../dist`) when this crate compiles. A checkout where the UI has
    // not been built yet (clippy and the tests run before scripts/ui-gate.sh builds it) gets a
    // page that says so, so the crate always compiles; `bun run build` replaces it.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if !dist.join("index.html").exists() {
        let _ = std::fs::create_dir_all(&dist);
        let _ = std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>Atlas</title><p>The UI is not built. Run <code>bun run build</code> in apps/desktop.</p>\n",
        );
    }
    tauri_build::build();
}

/// The files whose change means a new commit: this checkout's own HEAD, the branch file it names
/// and the packed refs. Only files that exist are named: cargo reruns a build script that names a
/// missing file on every build, and in a worktree `<root>/.git` is a file, not a directory, so
/// the old `.git/HEAD` rebuilt this crate in every build of every lane.
fn commit_files(root: &Path) -> Vec<std::path::PathBuf> {
    let git = |arg: &str| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "--path-format=absolute", arg])
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
    };
    let (Some(dir), Some(common)) = (git("--git-dir"), git("--git-common-dir")) else {
        return Vec::new();
    };
    let head = dir.join("HEAD");
    let mut files = vec![head.clone(), common.join("packed-refs")];
    if let Some(branch) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|text| text.strip_prefix("ref: ").map(|r| r.trim().to_string()))
    {
        files.push(common.join(branch));
    }
    files.retain(|file| file.is_file());
    files
}

fn checkout_version(root: &Path) -> String {
    let release = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| {
            let section = text.split("[workspace.metadata.atlas]").nth(1)?;
            let line = section
                .lines()
                .find(|l| l.trim_start().starts_with("release"))?;
            Some(line.split('"').nth(1)?.to_string())
        })
        .unwrap_or_else(|| {
            env!("CARGO_PKG_VERSION")
                .trim_end_matches("-dev")
                .to_string()
        });
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|text| !text.is_empty())
    };
    match (
        git(&["rev-list", "--count", "HEAD"]),
        git(&["rev-parse", "--short=8", "HEAD"]),
    ) {
        (Some(count), Some(commit)) => format!("{release}-dev.{count}+{commit}"),
        _ => format!("{release}-dev"),
    }
}
