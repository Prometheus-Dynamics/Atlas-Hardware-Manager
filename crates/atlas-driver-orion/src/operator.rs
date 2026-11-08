//! Atlas's Orion operator identity: one key per data folder, shared by the
//! app and the CLI (both keep it in Atlas's data folder). The default folder
//! keeps the name `atlas-<host>` that boards were enrolled under. Any other
//! folder (a test instance's scratch data) gets its own name, so it doesn't
//! replace the enrolled key of the real app on the same computer.

use std::path::Path;

use orion_client::remote::OperatorIdentity;

/// The key's file in Atlas's data folder.
pub const KEY_FILE: &str = "orion-operator.key";
/// Names the operator (`operator:<name>`), overriding the derived name.
pub const NAME_ENV: &str = "ATLAS_OPERATOR_NAME";

/// Loads Atlas's operator key, creating it (0600) the first time, and names
/// the operator for `dir`: see [`name_for`].
pub fn operator_identity(dir: &Path) -> Result<OperatorIdentity, String> {
    let file = dir.join(KEY_FILE);
    let secret = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(_) => {
            let fresh = OperatorIdentity::generate("atlas").map_err(|error| error.to_string())?;
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
            let bytes = fresh.secret_key_bytes();
            write_private(&file, &bytes)?;
            bytes.to_vec()
        }
    };
    // The fingerprint comes from the key alone, so the name is chosen after it.
    let fingerprint = OperatorIdentity::from_secret_key_bytes("atlas", &secret)
        .map_err(|error| format!("{}: {error}", file.display()))?
        .fingerprint();
    let explicit = std::env::var(NAME_ENV).ok();
    let separate = !is_default_folder(dir) && !hostname_overridden();
    let name = name_for(explicit.as_deref(), &host_name(), separate, &fingerprint);
    OperatorIdentity::from_secret_key_bytes(&name, &secret)
        .map_err(|error| format!("{}: {error}", file.display()))
}

/// The operator name for a data folder.
///
/// An explicit name wins. Otherwise `atlas-<host>` (lowercase, safe for an
/// operator name; `atlas` when there is no host). When `separate` (a data
/// folder other than the default, on this computer's own hostname), the
/// first 8 hex digits of the key's fingerprint follow, so two folders on one
/// computer never share a name.
pub(crate) fn name_for(
    explicit: Option<&str>,
    host: &str,
    separate: bool,
    fingerprint: &str,
) -> String {
    if let Some(name) = explicit.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    let host: String = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let host = host.trim_matches('-');
    let base = if host.is_empty() {
        "atlas".to_owned()
    } else {
        format!("atlas-{host}")
    };
    if separate {
        let tag: String = fingerprint
            .trim_start_matches("sha256:")
            .chars()
            .take(8)
            .collect();
        format!("{base}-{tag}")
    } else {
        base
    }
}

/// Whether `dir` is the default data folder, where the app and the CLI keep
/// Atlas's files unless `ATLAS_DATA_DIR` or `XDG_DATA_HOME` moves them.
fn is_default_folder(dir: &Path) -> bool {
    dirs::data_dir().is_some_and(|data| data.join("atlas") == dir)
}

/// The machine's host name: `/etc/hostname`, else the kernel's, else the
/// environment's (Windows, then a shell's).
fn host_name() -> String {
    read_line("/etc/hostname")
        .or_else(|| read_line("/proc/sys/kernel/hostname"))
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default()
}

/// Whether this process has a hostname of its own (as under `bwrap
/// --hostname`): the kernel's differs from the machine's `/etc/hostname`.
/// Such a run keeps the plain name, so the names it was enrolled under still
/// work.
fn hostname_overridden() -> bool {
    match (
        read_line("/etc/hostname"),
        read_line("/proc/sys/kernel/hostname"),
    ) {
        (Some(file), Some(kernel)) => file != kernel,
        _ => false,
    }
}

fn read_line(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// Creates `file` readable only by this user (0600 on Unix; on Windows the
/// per-user data directory's ACL already restricts it).
fn write_private(file: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options
        .open(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    handle
        .write_all(bytes)
        .map_err(|error| format!("{}: {error}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINGERPRINT: &str = "sha256:0123456789abcdef0123456789abcdef";

    #[test]
    fn the_operator_key_is_created_once_and_kept_private() {
        let dir = std::env::temp_dir().join(format!("atlas-orion-{}", std::process::id()));
        let first = operator_identity(&dir).unwrap();
        let again = operator_identity(&dir).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        assert_eq!(first.operator_id(), again.operator_id());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_default_folder_keeps_the_plain_name() {
        assert_eq!(
            name_for(None, "Mathias-PC\n", false, FINGERPRINT),
            "atlas-mathias-pc"
        );
    }

    #[test]
    fn another_folder_on_this_computer_adds_its_key_tag() {
        assert_eq!(
            name_for(None, "mathias-pc", true, FINGERPRINT),
            "atlas-mathias-pc-01234567"
        );
    }

    #[test]
    fn an_overridden_hostname_keeps_the_plain_name() {
        // The bench's bwrap runs (their own hostname, scratch data): the
        // name they were enrolled under stays as it was.
        assert_eq!(name_for(None, "bench", false, FINGERPRINT), "atlas-bench");
    }

    #[test]
    fn an_explicit_name_wins_and_blank_is_ignored() {
        assert_eq!(
            name_for(Some(" atlas-bench "), "mathias-pc", true, FINGERPRINT),
            "atlas-bench"
        );
        assert_eq!(
            name_for(Some("  "), "mathias-pc", false, FINGERPRINT),
            "atlas-mathias-pc"
        );
    }

    #[test]
    fn without_a_host_the_name_is_atlas() {
        assert_eq!(name_for(None, " ", false, FINGERPRINT), "atlas");
        assert_eq!(name_for(None, " ", true, FINGERPRINT), "atlas-01234567");
    }
}
