//! Atlas's Orion operator identity: one key per user, shared by the app and
//! the CLI (both keep it in Atlas's data folder).

use std::path::Path;

use orion_client::remote::OperatorIdentity;

/// The key's file in Atlas's data folder.
pub const KEY_FILE: &str = "orion-operator.key";

/// `atlas-<hostname>`, lowercase, safe for an operator name.
fn operator_name() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .or_else(|_| std::fs::read_to_string("/proc/sys/kernel/hostname"))
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    let host: String = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let host = host.trim_matches('-');
    if host.is_empty() {
        "atlas".into()
    } else {
        format!("atlas-{host}")
    }
}

/// Loads Atlas's operator key, creating it (0600) the first time.
pub fn operator_identity(dir: &Path) -> Result<OperatorIdentity, String> {
    let file = dir.join(KEY_FILE);
    let name = operator_name();
    if let Ok(bytes) = std::fs::read(&file) {
        return OperatorIdentity::from_secret_key_bytes(&name, &bytes)
            .map_err(|error| format!("{}: {error}", file.display()));
    }
    let identity = OperatorIdentity::generate(&name).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    write_private(&file, &identity.secret_key_bytes())?;
    Ok(identity)
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

    #[test]
    fn the_operator_key_is_created_once_and_kept_private() {
        let dir = std::env::temp_dir().join(format!("atlas-orion-{}", std::process::id()));
        let first = operator_identity(&dir).unwrap();
        let again = operator_identity(&dir).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(operator_name().starts_with("atlas"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
