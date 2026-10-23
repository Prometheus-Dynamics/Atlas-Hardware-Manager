//! `atlas slots push`: a layer image and its entry from the build's
//! layers.json, as the archive the board's `update push` reads: `layer.json`
//! (the entry) and `<id>.erofs`, a plain ustar.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The entry for layer `name`: from a layers.json (`{"layers":[...]}` or a
/// bare list) or a single entry (`{"name":...}`), with what the board needs.
pub(crate) fn entry(text: &str, name: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let list = match &value {
        Value::Object(map) if map.contains_key("layers") => map["layers"].clone(),
        Value::Array(_) => value.clone(),
        Value::Object(_) => Value::Array(vec![value.clone()]),
        _ => return Err("not a layers list or a layer".into()),
    };
    let mut found = list
        .as_array()
        .into_iter()
        .flatten()
        .find(|layer| layer.get("name").and_then(Value::as_str) == Some(name))
        .cloned()
        .ok_or_else(|| format!("no layer named {name}"))?;
    // Gaia's layers.json calls the file's size and sha256 erofs_size and
    // erofs_sha256; the board's layer.json has them as size and sha256.
    if let Some(map) = found.as_object_mut() {
        for (want, gaia) in [("size", "erofs_size"), ("sha256", "erofs_sha256")] {
            if !map.contains_key(want)
                && let Some(value) = map.get(gaia).cloned()
            {
                map.insert(want.into(), value);
            }
        }
    }
    if found.get("verity").is_none_or(Value::is_null) {
        return Err(format!(
            "layer {name} was built without verity, which the board needs ([image.layers] verity = true)"
        ));
    }
    let hex64 = |path: &str| {
        found
            .pointer(path)
            .and_then(Value::as_str)
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| format!("layer {name}: {} isn't 64 hex digits", &path[1..]))
    };
    hex64("/id")?;
    hex64("/sha256")?;
    hex64("/verity/root_hash")?;
    found
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("layer {name}: no size"))?;
    Ok(found)
}

/// The id and size an entry names.
pub(crate) fn id_and_size(entry: &Value) -> (String, u64) {
    (
        entry["id"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase(),
        entry["size"].as_u64().unwrap_or_default(),
    )
}

/// One ustar header: a regular file, mode 0644, owned by root, mtime 0.
fn header(name: &str, size: u64) -> Result<[u8; 512], String> {
    if name.len() > 99 {
        return Err(format!("{name}: too long a name for the archive"));
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    let mut put = |at: usize, len: usize, text: String| {
        h[at..at + text.len().min(len)].copy_from_slice(&text.as_bytes()[..text.len().min(len)]);
    };
    put(100, 8, "0000644\0".into());
    put(108, 8, "0000000\0".into());
    put(116, 8, "0000000\0".into());
    put(124, 12, format!("{size:011o}\0"));
    put(136, 12, "00000000000\0".into());
    put(156, 1, "0".into());
    put(257, 6, "ustar\0".into());
    put(263, 2, "00".into());
    // The checksum: the header's bytes with its own field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    Ok(h)
}

/// Writes the push archive for `layer` (its file) and `entry` to `out`;
/// returns its size.
pub(crate) fn write_archive(entry: &Value, layer: &Path, out: &Path) -> Result<u64, String> {
    let (id, size) = id_and_size(entry);
    let json = serde_json::to_vec(entry).map_err(|error| error.to_string())?;
    let mut file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut written = 0u64;
    let mut emit = |bytes: &[u8]| -> Result<(), String> {
        written += bytes.len() as u64;
        file.write_all(bytes)
            .map_err(|e| format!("{}: {e}", out.display()))
    };
    let pad = |len: u64| vec![0u8; ((512 - len % 512) % 512) as usize];
    emit(&header("layer.json", json.len() as u64)?)?;
    emit(&json)?;
    emit(&pad(json.len() as u64))?;
    emit(&header(&format!("{id}.erofs"), size)?)?;
    let mut src = std::fs::File::open(layer).map_err(|e| format!("{}: {e}", layer.display()))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut copied = 0u64;
    loop {
        let read = src
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", layer.display()))?;
        if read == 0 {
            break;
        }
        copied += read as u64;
        emit(&buffer[..read])?;
    }
    if copied != size {
        return Err(format!(
            "{} is {copied} bytes, the layer's entry says {size}",
            layer.display()
        ));
    }
    emit(&pad(size))?;
    emit(&[0u8; 1024])?;
    Ok(written)
}

/// A temporary path for the archive.
pub(crate) fn temp_archive() -> PathBuf {
    std::env::temp_dir().join(format!("atlas-layer-push-{}.tar", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str) -> String {
        format!(
            r#"{{"name":"{name}","id":"{}","size":5,"sha256":"{}","verity":{{"root_hash":"{}","salt":"00","hash_offset":4096,"data_blocks":1}}}}"#,
            "A".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        )
    }

    #[test]
    fn the_entry_comes_from_a_layers_list_a_bare_list_or_one_layer() {
        let doc = format!(
            r#"{{"order":["base","app"],"layers":[{},{}]}}"#,
            layer("base"),
            layer("app")
        );
        assert_eq!(entry(&doc, "app").unwrap()["name"], "app");
        assert_eq!(
            entry(&format!("[{}]", layer("app")), "app").unwrap()["name"],
            "app"
        );
        assert_eq!(entry(&layer("app"), "app").unwrap()["name"], "app");
        assert!(entry(&doc, "platform").is_err());
        // What the board needs is checked here first.
        assert!(entry(&layer("app").replace(&"b".repeat(64), "x"), "app").is_err());
        assert!(entry(&layer("app").replace(r#""size":5,"#, ""), "app").is_err());
        let (id, size) = id_and_size(&entry(&layer("app"), "app").unwrap());
        assert_eq!((id.as_str(), size), ("a".repeat(64).as_str(), 5));
    }

    #[test]
    fn gaias_record_is_taken_as_it_is() {
        let gaia = format!(
            r#"{{"layers":[{{"name":"app","id":"{0}","tar":"layers/app.tar","tar_sha256":"{1}","erofs":"layers/app.erofs","erofs_sha256":"{0}","erofs_size":5,"verity":{{"root_hash":"{2}","salt":"00","hash_offset":4096,"data_blocks":1}}}}]}}"#,
            "a".repeat(64),
            "9".repeat(64),
            "c".repeat(64)
        );
        let got = entry(&gaia, "app").unwrap();
        assert_eq!(got["size"], 5);
        assert_eq!(got["sha256"], "a".repeat(64));
        let bare = gaia.replace(
            &format!(
                r#"{{"root_hash":"{}","salt":"00","hash_offset":4096,"data_blocks":1}}"#,
                "c".repeat(64)
            ),
            "null",
        );
        assert!(entry(&bare, "app").unwrap_err().contains("without verity"));
    }

    #[test]
    fn the_archive_is_a_ustar_with_the_entry_and_the_image() {
        let dir = std::env::temp_dir().join(format!("atlas-layer-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("layer.erofs");
        std::fs::write(&image, b"hello").unwrap();
        let entry = entry(&layer("app"), "app").unwrap();
        let out = dir.join("push.tar");
        let size = write_archive(&entry, &image, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(size % 512, 0);
        assert_eq!(&bytes[..10], b"layer.json");
        assert_eq!(&bytes[257..263], b"ustar\0");
        // Each header's checksum is its bytes, the field counted as spaces.
        let mut h = bytes[..512].to_vec();
        let stored = u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap();
        h[148..156].copy_from_slice(b"        ");
        assert_eq!(stored, h.iter().map(|&b| u32::from(b)).sum::<u32>());
        let json_len = serde_json::to_vec(&entry).unwrap().len();
        let second = 512 + json_len.div_ceil(512) * 512;
        assert!(bytes[second..].starts_with(format!("{}.erofs", "a".repeat(64)).as_bytes()));
        assert_eq!(&bytes[second + 512..second + 517], b"hello");
        // With a tar on this machine, it lists the two members.
        if let Ok(listing) = std::process::Command::new("tar")
            .arg("-tf")
            .arg(&out)
            .output()
            && listing.status.success()
        {
            let text = String::from_utf8_lossy(&listing.stdout);
            assert_eq!(
                text.lines().collect::<Vec<_>>(),
                ["layer.json", &format!("{}.erofs", "a".repeat(64))]
            );
        }
        // An image that isn't the entry's size isn't sent.
        std::fs::write(&image, b"hello!").unwrap();
        assert!(write_archive(&entry, &image, &out).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
