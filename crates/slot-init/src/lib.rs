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
//!
//! A layered slot (slot.json `layers`, lowest first: base, platform, app)
//! is several such images, `/data/slots/layers/<layer id>.erofs`, each
//! through its own dm-verity with its own root hash (the command line's
//! `atlas.roothash` lists them, comma-separated, in the same order), stacked
//! read-only as an overlayfs root.

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
    /// A one-image slot's verity (`<id>.erofs`); None for a layered one.
    #[serde(default)]
    pub verity: Option<Verity>,
    /// The image file as built (its size), when slot.json says.
    #[serde(default)]
    pub image: Option<Image>,
    /// A layered slot's layers, lowest first; empty for a one-image slot.
    #[serde(default)]
    pub layers: Vec<Layer>,
}

/// One layer of a layered slot: an EROFS image with its hash tree appended,
/// shared by every slot that has it (named by its id).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(try_from = "LayerJson")]
pub struct Layer {
    /// What it is (`base`, `platform`, `app`): names its dm device.
    pub name: String,
    /// Its content id (sha256, 64 hex digits): the file is `layers/<id>.erofs`.
    pub id: String,
    /// The file's size as built (data and hash tree).
    pub size: u64,
    pub verity: Verity,
}

/// A layer as written: Gaia's layers.json calls the size `erofs_size`, and
/// carries `size` too (the same number).
#[derive(Deserialize)]
struct LayerJson {
    name: String,
    id: String,
    size: Option<u64>,
    erofs_size: Option<u64>,
    verity: Verity,
}

impl TryFrom<LayerJson> for Layer {
    type Error = String;

    fn try_from(raw: LayerJson) -> Result<Self, String> {
        let size = raw
            .size
            .or(raw.erofs_size)
            .ok_or_else(|| format!("layer {}: no size", raw.name))?;
        Ok(Self {
            name: raw.name,
            id: raw.id,
            size,
            verity: raw.verity,
        })
    }
}

/// At most this many layers (overlayfs takes many more; a slot needs three).
pub const MAX_LAYERS: usize = 8;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Image {
    pub size: u64,
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

impl Verity {
    fn check(&self, what: &str) -> Result<(), String> {
        for size in [self.data_block_size, self.hash_block_size] {
            if !size.is_power_of_two() || !(512..=65536).contains(&size) {
                return Err(format!(
                    "{what}: block size {size} isn't a power of two in 512..65536"
                ));
            }
        }
        if !self
            .hash_offset
            .is_multiple_of(u64::from(self.hash_block_size))
        {
            return Err(format!("{what}: hash_offset isn't a whole hash block"));
        }
        if self.data_blocks == 0
            || self.data_blocks * u64::from(self.data_block_size) > self.hash_offset
        {
            return Err(format!("{what}: the data runs into the hash tree"));
        }
        if !hex(&self.salt) {
            return Err(format!("{what}: the salt isn't hex"));
        }
        Ok(())
    }

    /// Where the hash tree's first block starts (after the verity superblock).
    pub fn tree_start(&self) -> u64 {
        self.hash_offset + u64::from(self.hash_block_size)
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(text).map_err(|e| format!("slot.json: {e}"))?;
        match (&manifest.verity, manifest.layers.is_empty()) {
            (Some(v), true) => v.check("slot.json")?,
            (None, false) => {
                if manifest.layers.len() > MAX_LAYERS {
                    return Err(format!(
                        "slot.json: {} layers, at most {MAX_LAYERS}",
                        manifest.layers.len()
                    ));
                }
                for (n, layer) in manifest.layers.iter().enumerate() {
                    let what = format!("slot.json layer {n}");
                    if !valid_id(&layer.name) {
                        return Err(format!("{what}: `{}` isn't a layer name", layer.name));
                    }
                    if layer.id.len() != 64
                        || !hex(&layer.id)
                        || layer.id != layer.id.to_ascii_lowercase()
                    {
                        return Err(format!("{what}: the id isn't 64 lowercase hex digits"));
                    }
                    if manifest.layers[..n].iter().any(|l| l.name == layer.name) {
                        return Err(format!("{what}: `{}` twice", layer.name));
                    }
                    layer.verity.check(&what)?;
                }
            }
            (Some(_), false) => {
                return Err("slot.json: both verity (one image) and layers".into());
            }
            (None, true) => return Err("slot.json: no verity and no layers".into()),
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
    Ok(scalar(&json(text)?, path))
}

/// Several fields at once, as `field` gives them, in `paths` order: a
/// missing one (or one that isn't a scalar) is "". Err when a value
/// wouldn't fit on its line (a newline or NUL), so the caller reads that
/// one with `field`.
pub fn fields(text: &str, paths: &[&str]) -> Result<Vec<String>, String> {
    let value = json(text)?;
    paths
        .iter()
        .map(|path| {
            let text = scalar(&value, path).unwrap_or_default();
            if text.contains(['\n', '\0']) {
                Err(format!("{path} has a newline or NUL"))
            } else {
                Ok(text)
            }
        })
        .collect()
}

fn json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))
}

fn scalar(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut at = value;
    for key in path.split('.') {
        at = at.get(key)?;
    }
    match at {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A layered slot.json's layers as "<name> <id> <size> <sha256> <root hash>"
/// lines, lowest first (the writer's view: what to install and what goes on
/// the command line). The manifest is checked as the initramfs checks it,
/// and each layer must carry its file's sha256 and its verity root hash.
/// Empty for a one-image slot.
pub fn layer_lines(text: &str) -> Result<Vec<String>, String> {
    let manifest = Manifest::parse(text)?;
    let value = json(text)?;
    manifest
        .layers
        .iter()
        .enumerate()
        .map(|(n, layer)| {
            let at = |key: &str| {
                value
                    .pointer(&format!("/layers/{n}/{key}"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
            };
            // slot.json's own name for it, or Gaia's layers.json's.
            let sha = match at("sha256") {
                s if s.is_empty() => at("erofs_sha256"),
                s => s,
            };
            if sha.len() != 64 || !hex(&sha) {
                return Err(format!("slot.json layer {}: no sha256", layer.name));
            }
            let root = root_hash(&at("verity/root_hash"))
                .map_err(|e| format!("slot.json layer {}: {e}", layer.name))?;
            Ok(format!(
                "{} {} {} {sha} {root}",
                layer.name, layer.id, layer.size
            ))
        })
        .collect()
}

/// A bundle's slot.json (no `layers`, `image` or `verity` of its own) with
/// the layers of the build's layers.json (Gaia's `{"layers":[...]}`, lowest
/// first) as its `layers`: what the writer installs. Checked as the
/// initramfs and the writer check it.
pub fn merge_layers(slot: &str, layers: &str) -> Result<String, String> {
    let mut value = json(slot)?;
    let list = json(layers)?
        .get("layers")
        .cloned()
        .filter(|l| l.is_array())
        .ok_or("layers.json: no layers")?;
    // Each layer as slot.json keeps it: what the board reads, not the
    // build's tar and input records.
    let list: Vec<serde_json::Value> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|layer| {
            let at = |key: &str| layer.get(key).cloned();
            let mut kept = serde_json::Map::new();
            for key in ["name", "id", "verity"] {
                if let Some(value) = at(key) {
                    kept.insert(key.into(), value);
                }
            }
            if let Some(size) = at("size").or_else(|| at("erofs_size")) {
                kept.insert("size".into(), size);
            }
            if let Some(sha) = at("sha256").or_else(|| at("erofs_sha256")) {
                kept.insert("sha256".into(), sha);
            }
            serde_json::Value::Object(kept)
        })
        .collect();
    let map = value.as_object_mut().ok_or("slot.json: not an object")?;
    map.remove("image");
    map.remove("verity");
    map.insert("layers".into(), list.into());
    map.insert("schema".into(), serde_json::Value::from(2));
    let text = serde_json::to_string(&value).map_err(|e| format!("slot.json: {e}"))?;
    layer_lines(&text)?;
    Ok(text)
}

/// A layered slot.json with one layer replaced by `layer` (a layers entry:
/// name, id, size, sha256, verity), the one of the same name: what a pushed
/// layer makes of the running slot's manifest. The result is checked as the
/// initramfs and the writer check it.
pub fn set_layer(slot: &str, layer: &str) -> Result<String, String> {
    let mut value = json(slot)?;
    let new = json(layer)?;
    let name = new
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("layer.json: no name")?
        .to_owned();
    let layers = value
        .get_mut("layers")
        .and_then(|l| l.as_array_mut())
        .ok_or("slot.json: not a layered slot")?;
    let at = layers
        .iter()
        .position(|l| l.get("name").and_then(|n| n.as_str()) == Some(name.as_str()))
        .ok_or_else(|| format!("the slot has no layer {name}"))?;
    layers[at] = new;
    let text = serde_json::to_string(&value).map_err(|e| format!("slot.json: {e}"))?;
    layer_lines(&text)?;
    Ok(text)
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

/// `atlas.roothash` for a layered slot: one root hash per layer, lowest
/// first, comma-separated.
pub fn root_hashes(s: &str) -> Result<Vec<String>, String> {
    s.split(',').map(root_hash).collect()
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

pub mod boot;
pub mod initramfs;
pub mod partition;
pub mod record;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod sys;

#[cfg(test)]
mod tests;
