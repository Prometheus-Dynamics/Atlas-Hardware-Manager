//! Atlas's pinned SSH host keys for a board, forgotten when it's reflashed.

/// Drops the known_hosts lines Atlas pinned for the board with `serial`
/// (its `HostKeyAlias`, `board-<family>-<serial>`, holds the serial).
/// Returns how many went. A missing file has nothing to forget.
pub(crate) fn forget_host_keys(path: &std::path::Path, serial: &str) -> std::io::Result<usize> {
    let serial = serial.trim().to_ascii_lowercase();
    // Too short a serial could match another board's.
    if serial.len() < 8 || !serial.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(0);
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let ours = |line: &str| {
        line.split_whitespace().next().is_some_and(|hosts| {
            hosts.split(',').any(|host| {
                host.starts_with("board-") && host.to_ascii_lowercase().contains(&serial)
            })
        })
    };
    let kept: Vec<&str> = text.lines().filter(|line| !ours(line)).collect();
    let gone = text.lines().count() - kept.len();
    if gone > 0 {
        let tmp = path.with_extension("tmp");
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(gone)
}
