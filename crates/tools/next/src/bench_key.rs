//! `atlas-next bench-key <directory>`: makes the THROWAWAY bench signing key, for bench boards only.
//!
//! It writes three new files into a directory and prints the key id and their paths:
//!
//! - `bench.seed`: the seed (64 hex and a newline, mode 0600). A test credential: never committed,
//!   never stored elsewhere, never printed.
//! - `bench.pub`: the public key (64 lowercase hex and a newline, mode 0644), the form
//!   `lemnos-ctl bundle --pubkey` takes.
//! - `bench.keys`: the board's key-list line, mode 0644, for a board that trusts bench keys.
//!
//! Nothing is overwritten, and a directory inside a git work tree, or one that group or others can
//! write, is refused before anything is made. Its output holds no part of the seed.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use atlas_bundle::Seed;

use crate::report::Report;

/// The seed file's name in the directory.
pub const SEED_FILE: &str = "bench.seed";
/// The public key file's name.
pub const PUBLIC_FILE: &str = "bench.pub";
/// The key-list file's name.
pub const KEYS_FILE: &str = "bench.keys";
/// The owner text of the key-list line: this key is for the bench, never a released image.
pub const OWNER: &str = "bench key, not for any released image";
/// The prefix of a public key's display form (`atlas-release` writes it).
const PUBLIC_PREFIX: &str = "ed25519:";

/// What one run made: the key id and the three paths.
#[derive(Debug)]
struct Made {
    key_id: String,
    seed: PathBuf,
    public: PathBuf,
    keys: PathBuf,
}

/// Runs the command: the output goes to `out` (text, or one JSON document with `json`). Returns the
/// exit status: 0 done, 1 refused (nothing was left behind).
pub fn run<W: Write>(directory: &Path, json: bool, out: &mut W) -> i32 {
    let mut r = Report::new(out, json);
    match make(directory) {
        Ok(made) => {
            r.progress(format!("key id: {}", made.key_id));
            r.progress(format!("seed file: {}", made.seed.display()));
            r.progress(format!("public key file: {}", made.public.display()));
            r.progress(format!("key list file: {}", made.keys.display()));
            r.field("key_id", made.key_id);
            r.field("seed_file", made.seed.display().to_string());
            r.field("public_key_file", made.public.display().to_string());
            r.field("key_list_file", made.keys.display().to_string());
            r.finish(0)
        }
        Err(words) => {
            r.say(words);
            r.finish(1)
        }
    }
}

fn make(directory: &Path) -> Result<Made, String> {
    let shown = directory.display();
    let canonical = fs::canonicalize(directory).map_err(|error| {
        format!("the directory {shown} cannot be used: {error}. Create it first, then run the command again")
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "{shown} is not a directory: give a directory that exists, outside any repository"
        ));
    }
    if let Some(top) = git_work_tree(&canonical) {
        return Err(format!(
            "{shown} is inside the git work tree {}: nothing was written. Use a directory outside any repository, for example a new one under your home directory or /tmp",
            top.display()
        ));
    }
    check_mode(&canonical, &shown.to_string())?;
    let paths = [
        canonical.join(SEED_FILE),
        canonical.join(PUBLIC_FILE),
        canonical.join(KEYS_FILE),
    ];
    for path in &paths {
        check_absent(path)?;
    }
    let seed = Seed::generate().map_err(|error| format!("{error}. Nothing was written"))?;
    let public = public_hex(&seed)?;
    let key_id = seed.key_id_hex();
    write_three(&seed, &public, &paths)?;
    Ok(Made {
        key_id,
        seed: paths[0].clone(),
        public: paths[1].clone(),
        keys: paths[2].clone(),
    })
}

/// The top of the git work tree that holds `path`, when one does: a `.git` entry (a file or a
/// directory) in `path` or in any parent. `path` is canonical, so a symbolic link cannot hide one.
fn git_work_tree(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| fs::symlink_metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// Refuses a directory that group or others can write (on Unix; elsewhere nothing is checked).
fn check_mode(directory: &Path, shown: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(directory)
            .map_err(|error| format!("{shown} cannot be read: {error}. Nothing was written"))?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o022 != 0 {
            return Err(format!(
                "{shown} has mode {mode:o}, so group or others can write it: nothing was written. Run chmod go-w on it, or use another directory"
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (directory, shown);
    Ok(())
}

/// Refuses when a file of the run's three names already exists (a dangling link counts).
fn check_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "{} already exists: nothing was written. Move it away, or use another directory",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "{} cannot be checked: {error}. Nothing was written",
            path.display()
        )),
    }
}

/// The public key of `seed` as 64 lowercase hex. The seed type has no hex form of its public key,
/// so this takes the public key's display form (`ed25519:<hex>`, the one public text form) and
/// checks its shape.
fn public_hex(seed: &Seed) -> Result<String, String> {
    let display = seed.public_key().to_string();
    let hex = display.strip_prefix(PUBLIC_PREFIX).unwrap_or_default();
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("the public key has an unexpected form. Nothing was written".into());
    }
    Ok(hex.to_owned())
}

/// Writes the three files in order. When one fails, the files this run already wrote are removed,
/// and the refusal says so. A file that existed before is never touched.
fn write_three(seed: &Seed, public: &str, paths: &[PathBuf; 3]) -> Result<(), String> {
    let public_line = format!("{public}\n");
    let key_line = format!("{public} {OWNER}\n");
    let mut written: Vec<&Path> = Vec::new();
    for (step, path) in paths.iter().enumerate() {
        let made = match step {
            0 => seed.write_new(path).map_err(|error| error.to_string()),
            1 => write_new_file(path, public_line.as_bytes()),
            _ => write_new_file(path, key_line.as_bytes()),
        };
        match made {
            Ok(()) => written.push(path),
            Err(error) => {
                for done in &written {
                    let _ = fs::remove_file(done);
                }
                let removed = if written.is_empty() {
                    "nothing was written".to_owned()
                } else {
                    format!(
                        "the {} file(s) this run had written were removed",
                        written.len()
                    )
                };
                return Err(format!("{error}. {removed}"));
            }
        }
    }
    Ok(())
}

/// Writes `bytes` to a new file with mode 0644 (set after creation, so the umask cannot narrow it).
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let shown = path.display();
    let mut file = options
        .open(path)
        .map_err(|error| format!("writing {shown}: {error}"))?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(path);
        return Err(format!("writing {shown}: {error}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o644)) {
            let _ = fs::remove_file(path);
            return Err(format!("setting the mode of {shown}: {error}"));
        }
    }
    Ok(())
}
