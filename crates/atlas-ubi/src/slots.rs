//! A/B slots as UBI volumes (Gaia G8's layout): `rootfs_a` and `rootfs_b`
//! (any `<name>_a`/`<name>_b` pair), the running one named on the kernel
//! command line by `ubi.block=<dev>,<volume>`, the other one written.

/// The volume names `ubi.block=` maps on this boot, from a kernel command
/// line (`ubi.block=0,rootfs_a`; the device part may be a number or a path).
pub fn mapped_volumes(cmdline: &str) -> Vec<String> {
    cmdline
        .split_ascii_whitespace()
        .filter_map(|word| word.strip_prefix("ubi.block="))
        .filter_map(|value| value.split_once(',').map(|(_, volume)| volume.to_string()))
        .collect()
}

/// The slot to write: the other half of the running slot's pair, which must
/// exist. `running` is the running slot's volume name.
pub fn inactive_slot<'a>(volumes: &'a [String], running: &str) -> Result<&'a str, String> {
    let (stem, other) = if let Some(stem) = running.strip_suffix("_a") {
        (stem, "_b")
    } else if let Some(stem) = running.strip_suffix("_b") {
        (stem, "_a")
    } else {
        return Err(format!(
            "the running volume {running} isn't one of an _a/_b pair"
        ));
    };
    let wanted = format!("{stem}{other}");
    volumes
        .iter()
        .find(|volume| **volume == wanted)
        .map(String::as_str)
        .ok_or_else(|| format!("no volume {wanted} to pair with the running {running}"))
}

/// The running slot among the mapped volumes: the one that is half of an
/// _a/_b pair.
pub fn running_slot(cmdline: &str) -> Option<String> {
    mapped_volumes(cmdline)
        .into_iter()
        .find(|volume| volume.ends_with("_a") || volume.ends_with("_b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_other_half_of_the_running_pair() {
        let cmdline = "console=ttyS0 ubi.mtd=ubi ubi.block=0,rootfs_b root=/dev/dm-0";
        assert_eq!(mapped_volumes(cmdline), ["rootfs_b"]);
        let running = running_slot(cmdline).unwrap();
        let volumes: Vec<String> = ["rootfs_a", "rootfs_b", "data"].map(String::from).to_vec();
        assert_eq!(inactive_slot(&volumes, &running), Ok("rootfs_a"));
        assert_eq!(inactive_slot(&volumes, "rootfs_a"), Ok("rootfs_b"));
        assert!(
            inactive_slot(&volumes, "data")
                .unwrap_err()
                .contains("pair")
        );
        assert!(inactive_slot(&volumes[..1], "rootfs_a").is_err());
        assert_eq!(running_slot("quiet"), None);
    }
}
