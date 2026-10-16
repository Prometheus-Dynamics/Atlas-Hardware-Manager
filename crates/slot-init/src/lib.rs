//! slot-init: the Raze slot initramfs's helper (the dynamic slots design:
//! slot images as files on /data, each with its dm-verity hash tree
//! appended). The initramfs's `/init` runs it to open the chosen image:
//!
//! - a read-only loop device on `/data/slots/<id>.erofs`;
//! - a dm-verity device over it, from the slot's manifest (block sizes,
//!   data blocks, where the hash tree starts, the salt) and the root hash
//!   the boot side passed on the kernel command line. The manifest lives on
//!   /data and isn't trusted: wrong values only make verity refuse.
//!
//! Done by ioctl (loop-control, device-mapper), so the initramfs needs no
//! cryptsetup or lvm2. Linux only.

use serde::Deserialize;

/// The parts of `slot.json` the initramfs needs.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub kernel: String,
    /// The OS family (`photonvision`, `helios`): whose data dirs it gets.
    #[serde(default)]
    pub family: String,
    /// Paths under /data this OS keeps per family (`photonvision_config`):
    /// each is bound from /data/os/<family>/<path>. None: /data as it is.
    #[serde(default)]
    pub data_dirs: Vec<String>,
    pub verity: Verity,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Verity {
    pub salt: String,
    /// Bytes from the start of the file to the hash tree.
    pub hash_offset: u64,
    pub data_blocks: u64,
    #[serde(default = "block")]
    pub data_block_size: u32,
    #[serde(default = "block")]
    pub hash_block_size: u32,
}

const fn block() -> u32 {
    4096
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(text).map_err(|e| format!("slot.json: {e}"))?;
        let v = &manifest.verity;
        for size in [v.data_block_size, v.hash_block_size] {
            if !size.is_power_of_two() || !(512..=65536).contains(&size) {
                return Err(format!(
                    "slot.json: block size {size} isn't a power of two in 512..65536"
                ));
            }
        }
        if !v.hash_offset.is_multiple_of(u64::from(v.hash_block_size)) {
            return Err("slot.json: hash_offset isn't a whole hash block".into());
        }
        if v.data_blocks == 0 || v.data_blocks * u64::from(v.data_block_size) > v.hash_offset {
            return Err("slot.json: the data runs into the hash tree".into());
        }
        if !hex(&v.salt) {
            return Err("slot.json: the salt isn't hex".into());
        }
        if !valid_id(&manifest.id) {
            return Err(format!("slot.json: `{}` isn't a slot id", manifest.id));
        }
        if let Some(bad) = manifest.data_dirs.iter().find(|d| !valid_data_dir(d)) {
            return Err(format!("slot.json: data dir `{bad}` isn't a relative path"));
        }
        if !manifest.family.is_empty() && !valid_id(&manifest.family) {
            return Err(format!(
                "slot.json: `{}` isn't a family name",
                manifest.family
            ));
        }
        Ok(manifest)
    }
}

/// The board's slot state (`/data/board/slots/state.json`), the parts the
/// initramfs reads: which slot to fall back to.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct State {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub previous: Option<String>,
}

impl State {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("state.json: {e}"))
    }
}

/// A slot id as the board names files after it: `[a-z0-9._-]`, at most 64,
/// not starting with a dot.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// A data dir name from a manifest: a relative path without `..`.
pub fn valid_data_dir(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && path.bytes().all(|b| b.is_ascii_graphic())
}

/// A dotted field of a JSON document (`verity.root_hash`) as text: strings
/// as they are, numbers and booleans as JSON; None when missing or not a
/// scalar.
pub fn field(text: &str, path: &str) -> Result<Option<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let mut at = &value;
    for key in path.split('.') {
        match at.get(key) {
            Some(next) => at = next,
            None => return Ok(None),
        }
    }
    Ok(match at {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

/// `boot.files` of a slot.json as "<sha256> <name>" lines (sha256sum's
/// format), names checked to stay inside the boot directory.
pub fn boot_files(text: &str) -> Result<Vec<(String, String)>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let files = value
        .pointer("/boot/files")
        .and_then(|f| f.as_object())
        .ok_or("slot.json: no boot.files")?;
    files
        .iter()
        .map(|(name, sha)| {
            let sha = sha.as_str().unwrap_or_default();
            if !valid_data_dir(name) {
                return Err(format!(
                    "slot.json: boot file `{name}` isn't a relative path"
                ));
            }
            if sha.len() != 64 || !hex(sha) {
                return Err(format!("slot.json: boot file `{name}` has no sha256"));
            }
            Ok((sha.to_ascii_lowercase(), name.clone()))
        })
        .collect()
}

fn hex(s: &str) -> bool {
    s.len().is_multiple_of(2) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A root hash as dm-verity takes it: sha256, 64 hex digits.
pub fn root_hash(s: &str) -> Result<String, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.len() != 64 || !hex(&s) {
        return Err("the root hash isn't 64 hex digits (sha256)".into());
    }
    Ok(s)
}

/// The dm table line for the slot: one verity target over the loop device
/// (`major:minor`), data and hash tree in the same file. As `veritysetup
/// format --hash-offset` lays it out (Gaia's verity transform): a verity
/// superblock at `hash_offset`, the hash tree one hash block after it.
pub fn verity_table(v: &Verity, device: &str, root: &str) -> (u64, String) {
    let sectors = v.data_blocks * u64::from(v.data_block_size) / 512;
    let hash_start = v.hash_offset / u64::from(v.hash_block_size) + 1;
    let salt = if v.salt.is_empty() { "-" } else { &v.salt };
    let params = format!(
        "1 {device} {device} {} {} {} {hash_start} sha256 {root} {salt}",
        v.data_block_size, v.hash_block_size, v.data_blocks
    );
    (sectors, params)
}

/// The kernel's encoded `dev_t` (new_encode_dev) as major and minor.
pub fn decode_dev(dev: u64) -> (u32, u32) {
    let major = ((dev & 0xfff00) >> 8) as u32;
    let minor = ((dev & 0xff) | ((dev >> 12) & 0xfff00)) as u32;
    (major, minor)
}

/// `atlas.<key>=` values from a kernel command line.
pub fn cmdline_arg<'a>(cmdline: &'a str, key: &str) -> Option<&'a str> {
    cmdline
        .split_whitespace()
        .find_map(|word| word.strip_prefix(key)?.strip_prefix('='))
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod sys;

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: &str = r#"{"schema":1,"id":"photonvision-2026.4.0-styx-1a2b3c4d","family":"photonvision",
        "version":"2026.4.0","variant":"styx","kernel":"7.2.9-v8-16k",
        "image":{"size":471859200,"sha256":"00"},
        "verity":{"root_hash":"ab","salt":"5eed","hash_offset":467664896,"data_blocks":114176,
                  "data_block_size":4096,"hash_block_size":4096}}"#;

    #[test]
    fn a_manifest_gives_the_verity_table() {
        let m = Manifest::parse(SLOT).unwrap();
        assert_eq!(m.kernel, "7.2.9-v8-16k");
        let root = root_hash(&"AB".repeat(32)).unwrap();
        let (sectors, params) = verity_table(&m.verity, "7:0", &root);
        assert_eq!(sectors, 114176 * 8);
        assert_eq!(
            params,
            format!("1 7:0 7:0 4096 4096 114176 114177 sha256 {root} 5eed")
        );
    }

    #[test]
    fn bad_manifests_and_hashes_are_refused() {
        let overlapping = SLOT.replace("467664896", "4096");
        assert!(Manifest::parse(&overlapping).is_err());
        let odd = SLOT.replace("\"hash_block_size\":4096", "\"hash_block_size\":3000");
        assert!(Manifest::parse(&odd).is_err());
        assert!(root_hash("abc").is_err());
        assert!(root_hash(&"g".repeat(64)).is_err());
    }

    #[test]
    fn data_dirs_ids_and_state() {
        let with = SLOT.replace(
            "\"variant\":\"styx\",",
            "\"variant\":\"styx\",\"data_dirs\":[\"photonvision_config\"],",
        );
        let m = Manifest::parse(&with).unwrap();
        assert_eq!(m.family, "photonvision");
        assert_eq!(m.data_dirs, ["photonvision_config"]);
        for bad in ["../etc", "/etc", "a/../b", ""] {
            assert!(
                Manifest::parse(&SLOT.replace(
                    "\"variant\":\"styx\",",
                    &format!("\"variant\":\"styx\",\"data_dirs\":[\"{bad}\"],")
                ))
                .is_err(),
                "{bad}"
            );
        }
        assert!(valid_id("photonvision-2026.4.0-styx-1a2b3c4d"));
        assert!(!valid_id("../x") && !valid_id("A") && !valid_id(".hidden"));
        let state =
            State::parse(r#"{"schema":1,"default":"a","previous":"b","trial":null,"slots":{}}"#)
                .unwrap();
        assert_eq!(state.previous.as_deref(), Some("b"));
    }

    #[test]
    fn fields_and_boot_files() {
        let text = r#"{"id":"x","image":{"size":12,"sha256":"aa"},"pinned":true,"boot":{"files":{"kernel8.img":"0000000000000000000000000000000000000000000000000000000000000000","overlays/a.dtbo":"1111111111111111111111111111111111111111111111111111111111111111"}}}"#;
        assert_eq!(field(text, "image.size").unwrap().as_deref(), Some("12"));
        assert_eq!(field(text, "image.sha256").unwrap().as_deref(), Some("aa"));
        assert_eq!(field(text, "pinned").unwrap().as_deref(), Some("true"));
        assert_eq!(field(text, "nope.x").unwrap(), None);
        let files = boot_files(text).unwrap();
        assert_eq!(files.len(), 2);
        assert!(boot_files(&text.replace("overlays/a.dtbo", "../a.dtbo")).is_err());
    }

    #[test]
    fn devices_and_cmdline() {
        // dm-3 on major 253: new_encode_dev.
        assert_eq!(decode_dev((253 << 8) | 3), (253, 3));
        assert_eq!(decode_dev((7 << 8) | 0x34 | (0x100 << 12)), (7, 0x134));
        let cmdline = "console=ttyAMA10 atlas.slot=pv-1 atlas.roothash=abcd panic=5";
        assert_eq!(cmdline_arg(cmdline, "atlas.slot"), Some("pv-1"));
        assert_eq!(cmdline_arg(cmdline, "atlas.data"), None);
    }
}
